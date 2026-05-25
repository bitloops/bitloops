use super::*;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, mpsc::channel};
use std::thread::{self, JoinHandle};
use toml_edit::de::from_str;

const CONFIG_RELOAD_DEBOUNCE: Duration = Duration::from_millis(250);
const DELAYED_RESTART_DELAY: Duration = Duration::from_millis(750);
const RESTART_DEDUPE_WINDOW: Duration = Duration::from_secs(3);
const HOT_RELOAD_TOP_LEVEL_KEYS: &[&str] = &[
    "inference",
    "knowledge",
    "semantic_clones",
    "context_guidance",
    "architecture",
];

static DELAYED_RESTARTS: LazyLock<Mutex<HashMap<PathBuf, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ACCEPTED_CONFIG_TEXTS: LazyLock<Mutex<HashMap<PathBuf, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(test)]
type RestartScheduleHook = dyn Fn(&Path, Duration) -> Result<()> + Send + Sync + 'static;

#[cfg(test)]
static RESTART_SCHEDULE_HOOK: LazyLock<Mutex<Option<Arc<RestartScheduleHook>>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonConfigChangeAction {
    Unchanged,
    HotReload,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonConfigApplyReport {
    pub(crate) restart_required: bool,
    pub(crate) reload_applied: bool,
    pub(crate) restart_scheduled: bool,
    pub(crate) apply_message: String,
}

impl DaemonConfigApplyReport {
    pub(crate) fn repo_reload() -> Self {
        Self {
            restart_required: false,
            reload_applied: true,
            restart_scheduled: false,
            apply_message: "Configuration saved and applied.".to_string(),
        }
    }
}

pub(crate) fn classify_daemon_config_change(
    previous_text: &str,
    next_text: &str,
) -> Result<DaemonConfigChangeAction> {
    let previous = parse_daemon_config_value(previous_text)?;
    let next = parse_daemon_config_value(next_text)?;

    if previous == next {
        return Ok(DaemonConfigChangeAction::Unchanged);
    }

    if structural_projection(previous) == structural_projection(next) {
        Ok(DaemonConfigChangeAction::HotReload)
    } else {
        Ok(DaemonConfigChangeAction::Restart)
    }
}

pub(crate) fn apply_daemon_config_change_after_save(
    config_path: &Path,
    previous_text: &str,
    next_text: &str,
) -> Result<DaemonConfigApplyReport> {
    crate::config::validate_daemon_config_text(next_text, config_path)
        .with_context(|| format!("validating updated daemon config {}", config_path.display()))?;

    let report = match classify_daemon_config_change(previous_text, next_text)? {
        DaemonConfigChangeAction::Unchanged => DaemonConfigApplyReport {
            restart_required: false,
            reload_applied: false,
            restart_scheduled: false,
            apply_message: "Configuration saved.".to_string(),
        },
        DaemonConfigChangeAction::HotReload => {
            log::info!(
                "daemon config reload applied: config={}",
                config_path.display()
            );
            DaemonConfigApplyReport {
                restart_required: false,
                reload_applied: true,
                restart_scheduled: false,
                apply_message: "Configuration saved and applied.".to_string(),
            }
        }
        DaemonConfigChangeAction::Restart => {
            schedule_delayed_daemon_restart(config_path)?;
            log::info!(
                "daemon config change requires restart: config={} restart_scheduled=true",
                config_path.display()
            );
            DaemonConfigApplyReport {
                restart_required: true,
                reload_applied: false,
                restart_scheduled: true,
                apply_message: "Configuration saved. Restart scheduled.".to_string(),
            }
        }
    };
    record_accepted_config_text(config_path, next_text)?;
    Ok(report)
}

pub(crate) fn schedule_delayed_daemon_restart(config_path: &Path) -> Result<bool> {
    let config_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let now = Instant::now();
    {
        let mut scheduled = DELAYED_RESTARTS
            .lock()
            .map_err(|_| anyhow::anyhow!("delayed daemon restart registry is poisoned"))?;
        scheduled.retain(|_, instant| now.duration_since(*instant) < RESTART_DEDUPE_WINDOW);
        if scheduled
            .get(&config_path)
            .is_some_and(|instant| now.duration_since(*instant) < RESTART_DEDUPE_WINDOW)
        {
            log::debug!(
                "daemon delayed restart already scheduled: config={}",
                config_path.display()
            );
            return Ok(false);
        }
        scheduled.insert(config_path.clone(), now);
    }

    #[cfg(test)]
    {
        if let Some(hook) = RESTART_SCHEDULE_HOOK
            .lock()
            .expect("restart hook lock")
            .as_ref()
            .cloned()
        {
            hook(&config_path, DELAYED_RESTART_DELAY)?;
        }
        Ok(true)
    }

    #[cfg(not(test))]
    {
        let executable = env::current_exe().context("resolving Bitloops executable for restart")?;
        let mut command = Command::new(executable);
        command
            .arg("__delayed-daemon-restart")
            .arg("--config")
            .arg(&config_path)
            .arg("--delay-ms")
            .arg(DELAYED_RESTART_DELAY.as_millis().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().with_context(|| {
            format!(
                "spawning delayed daemon restart for {}",
                config_path.display()
            )
        })?;
        Ok(true)
    }
}

pub(super) struct DaemonConfigReloadWatcher {
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for DaemonConfigReloadWatcher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take()
            && let Err(err) = thread.join()
        {
            log::warn!("daemon config reload watcher join failed: {err:?}");
        }
    }
}

pub(super) fn start_daemon_config_reload_watcher(
    config_path: &Path,
) -> Result<DaemonConfigReloadWatcher> {
    let initial_text = fs::read_to_string(config_path)
        .with_context(|| format!("reading daemon config {}", config_path.display()))?;
    let config_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    record_accepted_config_text(&config_path, &initial_text)?;
    let config_parent = config_path
        .parent()
        .map(Path::to_path_buf)
        .context("resolving daemon config parent directory for reload watcher")?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let thread = thread::Builder::new()
        .name("bitloops-config-reload-watcher".to_string())
        .spawn(move || {
            if let Err(err) = run_config_reload_watcher_loop(
                config_path,
                config_parent,
                initial_text,
                thread_shutdown,
            ) {
                log::warn!("daemon config reload watcher exited with error: {err:#}");
            }
        })
        .context("spawning daemon config reload watcher")?;

    Ok(DaemonConfigReloadWatcher {
        shutdown,
        thread: Some(thread),
    })
}

fn run_config_reload_watcher_loop(
    config_path: PathBuf,
    config_parent: PathBuf,
    initial_text: String,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = tx.send(event);
        },
        Config::default().with_poll_interval(Duration::from_millis(500)),
    )
    .context("creating daemon config reload watcher")?;
    watcher
        .watch(&config_parent, RecursiveMode::NonRecursive)
        .with_context(|| {
            format!(
                "watching daemon config directory {}",
                config_parent.display()
            )
        })?;

    let mut state = DaemonConfigReloadState::new(initial_text);
    log::info!(
        "daemon config reload watcher started: config={}",
        config_path.display()
    );

    while !shutdown.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                if event
                    .paths
                    .iter()
                    .any(|path| event_path_matches_config(path, &config_path))
                {
                    state.observe_event(Instant::now());
                }
            }
            Ok(Err(err)) => {
                log::warn!("daemon config reload watcher event failed: {err:#}");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        match state.apply_if_due(&config_path, Instant::now(), CONFIG_RELOAD_DEBOUNCE) {
            Ok(Some(_)) | Ok(None) => {}
            Err(err) => {
                log::warn!(
                    "daemon config reload failed: config={} error={err:#}",
                    config_path.display()
                );
            }
        }
    }

    log::info!(
        "daemon config reload watcher stopped: config={}",
        config_path.display()
    );
    Ok(())
}

fn event_path_matches_config(event_path: &Path, config_path: &Path) -> bool {
    event_path == config_path
        || (event_path.file_name() == config_path.file_name()
            && event_path.parent() == config_path.parent())
}

fn parse_daemon_config_value(text: &str) -> Result<Value> {
    from_str::<Value>(text).context("parsing daemon config for reload classification")
}

fn structural_projection(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        for key in HOT_RELOAD_TOP_LEVEL_KEYS {
            object.remove(*key);
        }
    }
    value
}

fn config_path_key(config_path: &Path) -> PathBuf {
    config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf())
}

fn accepted_config_text(config_path: &Path) -> Result<Option<String>> {
    let config_path = config_path_key(config_path);
    ACCEPTED_CONFIG_TEXTS
        .lock()
        .map_err(|_| anyhow::anyhow!("daemon config reload registry is poisoned"))
        .map(|accepted| accepted.get(&config_path).cloned())
}

fn record_accepted_config_text(config_path: &Path, text: &str) -> Result<()> {
    let config_path = config_path_key(config_path);
    ACCEPTED_CONFIG_TEXTS
        .lock()
        .map_err(|_| anyhow::anyhow!("daemon config reload registry is poisoned"))?
        .insert(config_path, text.to_string());
    Ok(())
}

#[derive(Debug)]
struct DaemonConfigReloadState {
    accepted_text: String,
    last_event_at: Option<Instant>,
}

impl DaemonConfigReloadState {
    fn new(accepted_text: String) -> Self {
        Self {
            accepted_text,
            last_event_at: None,
        }
    }

    fn observe_event(&mut self, now: Instant) {
        self.last_event_at = Some(now);
    }

    fn apply_if_due(
        &mut self,
        config_path: &Path,
        now: Instant,
        debounce: Duration,
    ) -> Result<Option<DaemonConfigApplyReport>> {
        let Some(last_event_at) = self.last_event_at else {
            return Ok(None);
        };
        if now.duration_since(last_event_at) < debounce {
            return Ok(None);
        }
        self.last_event_at = None;

        let next_text = fs::read_to_string(config_path)
            .with_context(|| format!("reading daemon config {}", config_path.display()))?;
        crate::config::validate_daemon_config_text(&next_text, config_path)
            .with_context(|| format!("validating daemon config {}", config_path.display()))?;
        let previous_text =
            accepted_config_text(config_path)?.unwrap_or_else(|| self.accepted_text.clone());
        if previous_text == next_text {
            self.accepted_text = next_text;
            return Ok(None);
        }
        let report =
            apply_daemon_config_change_after_save(config_path, &previous_text, &next_text)?;
        self.accepted_text = next_text;
        Ok(Some(report))
    }
}

#[cfg(test)]
pub(crate) fn reset_delayed_restart_dedupe_for_tests() {
    DELAYED_RESTARTS
        .lock()
        .expect("delayed restarts lock")
        .clear();
    ACCEPTED_CONFIG_TEXTS
        .lock()
        .expect("accepted configs lock")
        .clear();
}

#[cfg(test)]
pub(crate) fn with_delayed_restart_schedule_hook<T>(
    hook: impl Fn(&Path, Duration) -> Result<()> + Send + Sync + 'static,
    f: impl FnOnce() -> T,
) -> T {
    let hook = Arc::new(hook);
    {
        let mut slot = RESTART_SCHEDULE_HOOK.lock().expect("restart hook lock");
        assert!(slot.is_none(), "restart schedule hook already installed");
        *slot = Some(hook);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    {
        let mut slot = RESTART_SCHEDULE_HOOK.lock().expect("restart hook lock");
        *slot = None;
    }
    match result {
        Ok(value) => value,
        Err(err) => std::panic::resume_unwind(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;
    use toml_edit::{DocumentMut, Item, Value as TomlValue};

    fn default_config_doc() -> DocumentMut {
        crate::config::default_daemon_config_toml()
            .expect("default config")
            .parse::<DocumentMut>()
            .expect("parse default config")
    }

    #[test]
    fn classifies_inference_profile_changes_as_hot_reloadable() {
        let original = default_config_doc();
        let mut updated = original.clone();
        updated["inference"]["profiles"]["summary_llm"]["max_output_tokens"] =
            Item::Value(TomlValue::from(256));

        assert_eq!(
            classify_daemon_config_change(&original.to_string(), &updated.to_string())
                .expect("classify config change"),
            DaemonConfigChangeAction::HotReload
        );
    }

    #[test]
    fn classifies_store_path_changes_as_restart_required() {
        let original = default_config_doc();
        let mut updated = original.clone();
        updated["stores"]["events"]["duckdb_path"] =
            Item::Value(TomlValue::from("/tmp/bitloops/new-events.duckdb"));

        assert_eq!(
            classify_daemon_config_change(&original.to_string(), &updated.to_string())
                .expect("classify config change"),
            DaemonConfigChangeAction::Restart
        );
    }

    #[test]
    fn classifies_dashboard_changes_as_restart_required() {
        let original = default_config_doc();
        let mut updated = original.clone();
        updated["dashboard"]["port"] = Item::Value(TomlValue::from(7667));

        assert_eq!(
            classify_daemon_config_change(&original.to_string(), &updated.to_string())
                .expect("classify config change"),
            DaemonConfigChangeAction::Restart
        );
    }

    #[test]
    fn delayed_restart_scheduling_is_deduplicated() {
        reset_delayed_restart_dedupe_for_tests();
        let count = Arc::new(AtomicUsize::new(0));
        let count_for_hook = Arc::clone(&count);
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        fs::write(&config_path, "[runtime]\nlocal_dev = false\n").expect("write config");

        with_delayed_restart_schedule_hook(
            move |_, _| {
                count_for_hook.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || {
                assert!(
                    schedule_delayed_daemon_restart(&config_path).expect("schedule restart first")
                );
                assert!(
                    !schedule_delayed_daemon_restart(&config_path)
                        .expect("schedule restart second")
                );
            },
        );

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reload_state_debounces_and_keeps_previous_config_after_validation_failure() {
        reset_delayed_restart_dedupe_for_tests();
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let original = crate::config::default_daemon_config_toml().expect("default config");
        fs::write(&config_path, &original).expect("write config");
        let mut state = DaemonConfigReloadState::new(original.clone());
        let now = Instant::now();

        state.observe_event(now);
        assert!(
            state
                .apply_if_due(&config_path, now, CONFIG_RELOAD_DEBOUNCE)
                .expect("debounced reload")
                .is_none()
        );

        fs::write(&config_path, "not = [valid").expect("write invalid config");
        let err = state
            .apply_if_due(
                &config_path,
                now + CONFIG_RELOAD_DEBOUNCE + Duration::from_millis(1),
                CONFIG_RELOAD_DEBOUNCE,
            )
            .expect_err("invalid config should fail");
        assert!(format!("{err:#}").contains("validating daemon config"));
        assert_eq!(state.accepted_text, original);
    }

    #[test]
    fn watcher_state_ignores_changes_already_applied_by_dashboard_save() {
        reset_delayed_restart_dedupe_for_tests();
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let original = crate::config::default_daemon_config_toml().expect("default config");
        let mut updated = original
            .parse::<DocumentMut>()
            .expect("parse default config");
        updated["inference"]["profiles"]["summary_llm"]["max_output_tokens"] =
            Item::Value(TomlValue::from(256));
        let updated = updated.to_string();
        fs::write(&config_path, &updated).expect("write updated config");
        let mut state = DaemonConfigReloadState::new(original.clone());
        apply_daemon_config_change_after_save(&config_path, &original, &updated)
            .expect("direct dashboard apply");

        let now = Instant::now();
        state.observe_event(now);
        let report = state
            .apply_if_due(
                &config_path,
                now + CONFIG_RELOAD_DEBOUNCE + Duration::from_millis(1),
                CONFIG_RELOAD_DEBOUNCE,
            )
            .expect("watcher reload");

        assert_eq!(report, None);
        assert_eq!(state.accepted_text, updated);
    }

    #[test]
    fn reload_state_coalesces_rapid_writes_into_one_action() {
        reset_delayed_restart_dedupe_for_tests();
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let original = crate::config::default_daemon_config_toml().expect("default config");
        fs::write(&config_path, &original).expect("write original config");
        let mut first = original
            .parse::<DocumentMut>()
            .expect("parse default config");
        first["inference"]["profiles"]["summary_llm"]["max_output_tokens"] =
            Item::Value(TomlValue::from(256));
        let first = first.to_string();
        let mut second = original
            .parse::<DocumentMut>()
            .expect("parse default config");
        second["inference"]["profiles"]["summary_llm"]["max_output_tokens"] =
            Item::Value(TomlValue::from(512));
        let second = second.to_string();
        let mut state = DaemonConfigReloadState::new(original);
        let now = Instant::now();

        fs::write(&config_path, first).expect("write first config");
        state.observe_event(now);
        fs::write(&config_path, &second).expect("write second config");
        state.observe_event(now + Duration::from_millis(50));
        assert!(
            state
                .apply_if_due(
                    &config_path,
                    now + Duration::from_millis(100),
                    CONFIG_RELOAD_DEBOUNCE,
                )
                .expect("still debounced")
                .is_none()
        );
        let report = state
            .apply_if_due(
                &config_path,
                now + CONFIG_RELOAD_DEBOUNCE + Duration::from_millis(60),
                CONFIG_RELOAD_DEBOUNCE,
            )
            .expect("reload action")
            .expect("reload report");

        assert!(report.reload_applied);
        assert_eq!(state.accepted_text, second);
    }
}
