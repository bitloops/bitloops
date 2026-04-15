use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use bitloops_inference_protocol::{
    DescribeRequest, DescribeResponse, ErrorResponse, InferRequest, InferResponse,
    PROTOCOL_VERSION, ResponseMode, RuntimeRequest, RuntimeResponse, ShutdownRequest,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::InferenceRuntimeConfig;

use super::{BITLOOPS_PLATFORM_CHAT_DRIVER, TextGenerationService};

const SHARED_TEXT_GENERATION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SHARED_TEXT_GENERATION_SWEEP_INTERVAL: Duration = Duration::from_secs(5);

pub(super) struct BitloopsInferenceTextGenerationService {
    profile_name: String,
    descriptor: String,
    cache_key: String,
    shared_session: Arc<SharedBitloopsInferenceSession>,
}

impl BitloopsInferenceTextGenerationService {
    pub(super) fn new(
        profile_name: &str,
        driver: &str,
        runtime: &InferenceRuntimeConfig,
        config_path: &Path,
    ) -> Result<Self> {
        let session_config = BitloopsInferenceSessionConfig {
            command: runtime.command.clone(),
            args: runtime.args.clone(),
            startup_timeout_secs: runtime.startup_timeout_secs,
            request_timeout_secs: runtime.request_timeout_secs,
            config_path: config_path.to_path_buf(),
            profile_name: profile_name.to_string(),
            driver: driver.to_string(),
            launch_artifact_fingerprint: runtime_launch_artifact_fingerprint(
                &runtime.command,
                &runtime.args,
            ),
            process_environment_fingerprint: process_environment_fingerprint(),
        };
        let shared_session =
            shared_bitloops_inference_session_registry().get_or_create(&session_config)?;
        let describe = shared_session.describe().with_context(|| {
            format!(
                "requesting standalone `bitloops-inference` runtime for profile `{profile_name}`"
            )
        })?;
        let descriptor = format!(
            "{}:{}",
            describe.provider.provider_name, describe.provider.model_name
        );
        let cache_key = format!("profile={profile_name}::driver={driver}::provider={descriptor}");

        Ok(Self {
            profile_name: profile_name.to_string(),
            descriptor,
            cache_key,
            shared_session,
        })
    }
}

impl TextGenerationService for BitloopsInferenceTextGenerationService {
    fn descriptor(&self) -> String {
        self.descriptor.clone()
    }

    fn cache_key(&self) -> String {
        self.cache_key.clone()
    }

    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let response = self
            .shared_session
            .infer(system_prompt, user_prompt)
            .with_context(|| {
                format!(
                    "requesting standalone `bitloops-inference` runtime for profile `{}`",
                    self.profile_name
                )
            })?;
        let text = canonical_text_from_response(&response)?;
        if text.trim().is_empty() {
            bail!(
                "text-generation runtime `{}` returned no content",
                self.descriptor
            );
        }
        Ok(text)
    }
}

fn canonical_text_from_response(response: &InferResponse) -> Result<String> {
    if !response.text.trim().is_empty() {
        return Ok(response.text.clone());
    }

    if let Some(parsed_json) = response.parsed_json.as_ref() {
        return serde_json::to_string(parsed_json)
            .context("serialising structured text-generation response");
    }

    Ok(String::new())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BitloopsInferenceSessionConfig {
    command: String,
    args: Vec<String>,
    startup_timeout_secs: u64,
    request_timeout_secs: u64,
    config_path: PathBuf,
    profile_name: String,
    driver: String,
    launch_artifact_fingerprint: String,
    process_environment_fingerprint: String,
}

struct BitloopsInferenceSession {
    config: BitloopsInferenceSessionConfig,
    child: Child,
    stdin: ChildStdin,
    response_rx: Receiver<BitloopsInferenceSessionOutput>,
}

enum BitloopsInferenceSessionOutput {
    Response(RuntimeResponse),
    ReadError(String),
    Closed,
}

struct SharedBitloopsInferenceSessionRegistry {
    sessions: Mutex<HashMap<BitloopsInferenceSessionConfig, Arc<SharedBitloopsInferenceSession>>>,
}

struct SharedBitloopsInferenceSession {
    config: BitloopsInferenceSessionConfig,
    state: Mutex<SharedBitloopsInferenceSessionState>,
}

struct SharedBitloopsInferenceSessionState {
    session: Option<BitloopsInferenceSession>,
    describe: Option<DescribeResponse>,
    last_used_at: Instant,
}

impl SharedBitloopsInferenceSessionRegistry {
    fn get_or_create(
        &self,
        config: &BitloopsInferenceSessionConfig,
    ) -> Result<Arc<SharedBitloopsInferenceSession>> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("shared text-generation session registry mutex was poisoned"))?;
        Ok(sessions
            .entry(config.clone())
            .or_insert_with(|| Arc::new(SharedBitloopsInferenceSession::new(config.clone())))
            .clone())
    }

    fn shutdown_idle_sessions(&self, idle_timeout: Duration) {
        let sessions = match self.sessions.lock() {
            Ok(sessions) => sessions.values().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };
        for session in sessions {
            session.shutdown_if_idle(idle_timeout);
        }
    }
}

impl SharedBitloopsInferenceSession {
    fn new(config: BitloopsInferenceSessionConfig) -> Self {
        Self {
            config,
            state: Mutex::new(SharedBitloopsInferenceSessionState {
                session: None,
                describe: None,
                last_used_at: Instant::now(),
            }),
        }
    }

    fn describe(&self) -> Result<DescribeResponse> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("shared text-generation runtime session mutex was poisoned"))?;
        if let Some(describe) = state.describe.clone() {
            return Ok(describe);
        }

        match self.ensure_session_started(&mut state)?.describe() {
            Ok(describe) => {
                state.last_used_at = Instant::now();
                state.describe = Some(describe.clone());
                Ok(describe)
            }
            Err(first_err) => {
                state.session = None;
                state.describe = None;
                let restarted = BitloopsInferenceSession::start(&self.config)
                    .context("restarting standalone `bitloops-inference` runtime after failure")?;
                state.session = Some(restarted);
                let retry = state
                    .session
                    .as_mut()
                    .expect("session replaced above")
                    .describe()
                    .with_context(|| {
                        format!(
                            "retrying standalone `bitloops-inference` describe after failure: {first_err:#}"
                        )
                    });
                match retry {
                    Ok(describe) => {
                        state.last_used_at = Instant::now();
                        state.describe = Some(describe.clone());
                        Ok(describe)
                    }
                    Err(err) => {
                        state.session = None;
                        state.describe = None;
                        Err(err)
                    }
                }
            }
        }
    }

    fn infer(&self, system_prompt: &str, user_prompt: &str) -> Result<InferResponse> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("shared text-generation runtime session mutex was poisoned"))?;
        if state.describe.is_none() {
            state.describe = Some(self.ensure_session_started(&mut state)?.describe()?);
        }

        match self
            .ensure_session_started(&mut state)?
            .infer(system_prompt, user_prompt)
        {
            Ok(response) => {
                state.last_used_at = Instant::now();
                Ok(response)
            }
            Err(first_err) => {
                state.session = None;
                state.describe = None;
                let restarted = BitloopsInferenceSession::start(&self.config)
                    .context("restarting standalone `bitloops-inference` runtime after failure")?;
                state.session = Some(restarted);
                state.describe = Some(
                    state
                        .session
                        .as_mut()
                        .expect("session replaced above")
                        .describe()
                        .context("describing restarted standalone `bitloops-inference` runtime")?,
                );
                let retry = state
                    .session
                    .as_mut()
                    .expect("session replaced above")
                    .infer(system_prompt, user_prompt)
                    .with_context(|| {
                        format!(
                            "retrying standalone `bitloops-inference` request after failure: {first_err:#}"
                        )
                    });
                match retry {
                    Ok(response) => {
                        state.last_used_at = Instant::now();
                        Ok(response)
                    }
                    Err(err) => {
                        state.session = None;
                        state.describe = None;
                        Err(err)
                    }
                }
            }
        }
    }

    fn shutdown_if_idle(&self, idle_timeout: Duration) {
        let session = {
            let mut state = match self.state.try_lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.session.is_none() || state.last_used_at.elapsed() < idle_timeout {
                return;
            }
            state.describe = None;
            state.session.take()
        };
        drop(session);
    }

    fn ensure_session_started<'a>(
        &self,
        state: &'a mut SharedBitloopsInferenceSessionState,
    ) -> Result<&'a mut BitloopsInferenceSession> {
        if state.session.is_none() {
            state.session = Some(BitloopsInferenceSession::start(&self.config)?);
        }
        Ok(state.session.as_mut().expect("session ensured above"))
    }
}

fn shared_bitloops_inference_session_registry()
-> &'static Arc<SharedBitloopsInferenceSessionRegistry> {
    static REGISTRY: OnceLock<Arc<SharedBitloopsInferenceSessionRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = Arc::new(SharedBitloopsInferenceSessionRegistry {
            sessions: Mutex::new(HashMap::new()),
        });
        let sweeper_registry = Arc::clone(&registry);
        let _ = thread::Builder::new()
            .name("bitloops-inference-ipc-sweeper".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(SHARED_TEXT_GENERATION_SWEEP_INTERVAL);
                    sweeper_registry.shutdown_idle_sessions(SHARED_TEXT_GENERATION_IDLE_TIMEOUT);
                }
            });
        registry
    })
}

#[cfg(test)]
type PlatformRuntimeAuthEnvironmentHook = dyn Fn() -> Result<Vec<(String, String)>>;
#[cfg(test)]
type PlatformRuntimeAuthEnvironmentHookCell =
    std::cell::RefCell<Option<std::rc::Rc<PlatformRuntimeAuthEnvironmentHook>>>;

#[cfg(test)]
thread_local! {
    static PLATFORM_RUNTIME_AUTH_ENVIRONMENT_HOOK: PlatformRuntimeAuthEnvironmentHookCell =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(super) fn with_platform_runtime_auth_environment_hook<T>(
    hook: impl Fn() -> Result<Vec<(String, String)>> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    PLATFORM_RUNTIME_AUTH_ENVIRONMENT_HOOK.with(|cell| {
        let previous = cell.replace(Some(std::rc::Rc::new(hook)));
        let output = f();
        cell.replace(previous);
        output
    })
}

fn process_environment_fingerprint() -> String {
    let mut vars = std::env::vars_os()
        .map(|(key, value)| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        .collect::<Vec<_>>();
    vars.sort();
    sha256_hex(vars.join("\n").as_bytes())
}

fn runtime_launch_artifact_fingerprint(command: &str, args: &[String]) -> String {
    let command_path = Path::new(command);
    let mut candidates = vec![command];
    if runtime_command_uses_script_argument(command_path)
        && let Some(script_path) = args.first()
    {
        candidates.push(script_path.as_str());
    }

    let mut artefacts = candidates
        .into_iter()
        .filter_map(|candidate| {
            let path = Path::new(candidate);
            if !path.is_file() {
                return None;
            }
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let metadata = std::fs::metadata(&canonical).ok()?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
                .map(|duration| format!("{}:{}", duration.as_secs(), duration.subsec_nanos()))
                .unwrap_or_else(|| "unknown".to_string());
            Some(format!(
                "{}|{}|{}",
                canonical.display(),
                metadata.len(),
                modified
            ))
        })
        .collect::<Vec<_>>();
    artefacts.sort();
    sha256_hex(artefacts.join("\n").as_bytes())
}

fn platform_runtime_auth_environment() -> Vec<(String, String)> {
    #[cfg(test)]
    if let Some(result) = PLATFORM_RUNTIME_AUTH_ENVIRONMENT_HOOK
        .with(|cell| cell.borrow().clone())
        .map(|hook| hook())
    {
        return result.unwrap_or_else(|err| {
            log::debug!("skipping platform gateway auth injection via test hook: {err:#}");
            Vec::new()
        });
    }

    match crate::daemon::platform_gateway_bearer_token() {
        Ok(Some(token)) => vec![(crate::daemon::PLATFORM_GATEWAY_TOKEN_ENV.to_string(), token)],
        Ok(None) => Vec::new(),
        Err(err) => {
            log::debug!("skipping platform gateway auth injection: {err:#}");
            Vec::new()
        }
    }
}

fn ensure_runtime_auth_environment_available(
    config: &BitloopsInferenceSessionConfig,
) -> Result<()> {
    if config.driver != BITLOOPS_PLATFORM_CHAT_DRIVER {
        return Ok(());
    }

    if !platform_runtime_auth_environment().is_empty() {
        return Ok(());
    }

    bail!(
        "platform-backed text-generation profile `{}` requires an authenticated Bitloops session; run `bitloops login`",
        config.profile_name
    );
}

fn runtime_command_uses_script_argument(command: &Path) -> bool {
    command
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "sh" | "bash"
                    | "zsh"
                    | "python"
                    | "python3"
                    | "python3.11"
                    | "python3.12"
                    | "python3.13"
                    | "node"
                    | "ruby"
                    | "perl"
                    | "pwsh"
                    | "powershell"
            )
        })
        .unwrap_or(false)
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
fn evict_idle_text_generation_sessions_for_tests(idle_timeout: Duration) {
    shared_bitloops_inference_session_registry().shutdown_idle_sessions(idle_timeout);
}

impl BitloopsInferenceSession {
    fn start(config: &BitloopsInferenceSessionConfig) -> Result<Self> {
        if config.command.trim().is_empty() {
            bail!(
                "standalone `bitloops-inference` runtime command is not configured for profile `{}`",
                config.profile_name
            );
        }
        ensure_runtime_auth_environment_available(config)?;

        let mut command = Command::new(&config.command);
        command.args(&config.args);
        command.arg("run");
        command.arg("--config").arg(&config.config_path);
        command.arg("--profile").arg(&config.profile_name);
        command.envs(platform_runtime_auth_environment());
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit());

        let mut child = command.spawn().with_context(|| {
            format!(
                "spawning standalone `bitloops-inference` runtime `{}` for profile `{}`",
                config.command, config.profile_name
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .context("capturing standalone text-generation runtime stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("capturing standalone text-generation runtime stdout")?;
        Ok(Self {
            config: config.clone(),
            child,
            stdin,
            response_rx: Self::spawn_stdout_reader(stdout),
        })
    }

    fn describe(&mut self) -> Result<DescribeResponse> {
        let request = RuntimeRequest::Describe(DescribeRequest {
            request_id: next_request_id(),
        });
        let request_id = request.request_id().to_string();
        self.write_json_line(&request)?;
        match self.read_response(
            &request_id,
            self.config.startup_timeout_secs,
            "waiting for standalone text-generation runtime description",
        )? {
            RuntimeResponse::Describe(response) => {
                if response.protocol_version != PROTOCOL_VERSION {
                    self.terminate_child();
                    bail!(
                        "standalone `bitloops-inference` runtime protocol mismatch: expected {}, got {}",
                        PROTOCOL_VERSION,
                        response.protocol_version
                    );
                }
                Ok(response)
            }
            RuntimeResponse::Error(error) => {
                self.terminate_child();
                Err(runtime_error(error))
            }
            other => {
                self.terminate_child();
                bail!(
                    "standalone `bitloops-inference` runtime returned unexpected response to describe: {other:?}"
                )
            }
        }
    }

    fn infer(&mut self, system_prompt: &str, user_prompt: &str) -> Result<InferResponse> {
        let request = RuntimeRequest::Infer(InferRequest {
            request_id: next_request_id(),
            system_prompt: system_prompt.to_string(),
            user_prompt: user_prompt.to_string(),
            response_mode: ResponseMode::JsonObject,
            temperature: None,
            max_output_tokens: None,
            metadata: None,
        });
        let request_id = request.request_id().to_string();
        self.write_json_line(&request)?;
        match self.read_response(
            &request_id,
            self.config.request_timeout_secs,
            "waiting for standalone text-generation runtime response",
        )? {
            RuntimeResponse::Infer(response) => Ok(response),
            RuntimeResponse::Error(error) => {
                self.terminate_child();
                Err(runtime_error(error))
            }
            other => {
                self.terminate_child();
                bail!(
                    "standalone `bitloops-inference` runtime returned unexpected response to infer: {other:?}"
                )
            }
        }
    }

    fn shutdown(&mut self) -> Result<()> {
        let request = RuntimeRequest::Shutdown(ShutdownRequest {
            request_id: next_request_id(),
        });
        let request_id = request.request_id().to_string();
        self.write_json_line(&request)?;
        let _ = self.read_response(
            &request_id,
            1,
            "waiting for standalone text-generation runtime shutdown",
        );
        self.terminate_child();
        Ok(())
    }

    fn write_json_line(&mut self, value: &RuntimeRequest) -> Result<()> {
        let line = serde_json::to_string(value)
            .context("serialising standalone text-generation runtime request")?;
        writeln!(self.stdin, "{line}")
            .context("writing standalone text-generation runtime request")?;
        self.stdin
            .flush()
            .context("flushing standalone text-generation runtime request")
    }

    fn spawn_stdout_reader(stdout: ChildStdout) -> Receiver<BitloopsInferenceSessionOutput> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(BitloopsInferenceSessionOutput::Closed);
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<RuntimeResponse>(trimmed) {
                            Ok(response) => {
                                if tx
                                    .send(BitloopsInferenceSessionOutput::Response(response))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = tx.send(BitloopsInferenceSessionOutput::ReadError(
                                    anyhow!(
                                        "parsing standalone text-generation runtime response `{trimmed}`: {err}"
                                    )
                                    .to_string(),
                                ));
                                break;
                            }
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(BitloopsInferenceSessionOutput::ReadError(
                            anyhow!(err)
                                .context("reading standalone text-generation runtime response")
                                .to_string(),
                        ));
                        break;
                    }
                }
            }
        });
        rx
    }

    fn read_response(
        &mut self,
        request_id: &str,
        timeout_secs: u64,
        operation: &str,
    ) -> Result<RuntimeResponse> {
        let next = if timeout_secs == 0 {
            self.response_rx.recv().map_err(|_| {
                anyhow!("standalone `bitloops-inference` runtime exited before replying")
            })
        } else {
            self.response_rx
                .recv_timeout(Duration::from_secs(timeout_secs))
                .map_err(|err| match err {
                    RecvTimeoutError::Timeout => {
                        anyhow!("{operation} timed out after {timeout_secs}s")
                    }
                    RecvTimeoutError::Disconnected => {
                        anyhow!("standalone `bitloops-inference` runtime exited before replying")
                    }
                })
        };
        let response = match next {
            Ok(BitloopsInferenceSessionOutput::Response(response)) => response,
            Ok(BitloopsInferenceSessionOutput::ReadError(message)) => {
                self.terminate_child();
                return Err(anyhow!(message));
            }
            Ok(BitloopsInferenceSessionOutput::Closed) => {
                self.terminate_child();
                return Err(anyhow!(
                    "standalone `bitloops-inference` runtime exited before replying"
                ));
            }
            Err(err) => {
                self.terminate_child();
                return Err(err);
            }
        };
        if response.request_id() != request_id {
            self.terminate_child();
            bail!("standalone `bitloops-inference` runtime returned mismatched request id");
        }
        Ok(response)
    }

    fn terminate_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for BitloopsInferenceSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn runtime_error(error: ErrorResponse) -> anyhow::Error {
    let details = error
        .details
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    if details.is_empty() {
        anyhow!(
            "standalone `bitloops-inference` runtime error `{}`: {}",
            error.code,
            error.message
        )
    } else {
        anyhow!(
            "standalone `bitloops-inference` runtime error `{}`: {} ({details})",
            error.code,
            error.message
        )
    }
}

fn next_request_id() -> String {
    static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "text-generation-{}",
        NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
#[path = "text_generation/tests.rs"]
mod tests;
