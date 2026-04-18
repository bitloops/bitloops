use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::Args;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use crate::cli::embeddings::{
    EmbeddingsInstallState, EmbeddingsRuntime, inspect_embeddings_install_state,
};
use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;
use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};
use crate::config::{REPO_POLICY_LOCAL_FILE_NAME, bootstrap_default_daemon_environment};
use crate::utils::branding::color_hex_if_enabled;

#[path = "init/agent_hooks.rs"]
mod agent_hooks;
#[path = "init/agent_selection.rs"]
mod agent_selection;
#[path = "init/progress.rs"]
mod progress;
#[path = "init/workflow.rs"]
mod workflow;

pub use agent_selection::{InitAgentSelection, detect_or_select_agent};

pub type AgentSelector = dyn Fn(&[String], bool) -> std::result::Result<InitAgentSelection, String>;
const DEFAULT_INIT_INGEST_BACKFILL: usize = 50;
const NON_INTERACTIVE_INIT_EMBEDDINGS_SELECTION_ERROR: &str = "`bitloops init --install-default-daemon` requires an explicit embeddings choice when not running interactively. Pass `--embeddings-runtime local`, `--embeddings-runtime platform`, or `--no-embeddings`.";

#[cfg(test)]
type InstallDefaultDaemonHook = dyn Fn(bool) -> Result<()> + 'static;

#[cfg(test)]
type EnableDefaultDaemonServiceHook = dyn Fn(bool) -> Result<()> + 'static;

#[cfg(test)]
thread_local! {
    static INSTALL_DEFAULT_DAEMON_HOOK: RefCell<Option<Rc<InstallDefaultDaemonHook>>> =
        RefCell::new(None);
    static ENABLE_DEFAULT_DAEMON_SERVICE_HOOK: RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>> =
        RefCell::new(None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InitEmbeddingsSetupSelection {
    Existing,
    Cloud,
    Local,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InitFinalSetupSelection {
    pub sync: bool,
    pub ingest: bool,
    pub telemetry: bool,
    pub auto_start_daemon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InitFinalSetupPromptOptions {
    pub show_telemetry: bool,
    pub show_auto_start_daemon: bool,
}

#[derive(Args)]
pub struct InitArgs {
    /// Bootstrap and start the default Bitloops daemon service if it is not already running.
    #[arg(long, default_value_t = false)]
    pub install_default_daemon: bool,

    /// Remove and reinstall existing hooks for selected agents.
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Do not install the repo-local Bitloops Skill or rule alongside agent hooks.
    #[arg(long, default_value_t = false)]
    pub disable_bitloops_skill: bool,

    /// Target specific agent setups (repeatable).
    #[arg(long = "agent", value_name = "AGENT")]
    pub agent: Vec<String>,

    /// Enable anonymous telemetry for this CLI version.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub telemetry: Option<bool>,

    /// Disable anonymous telemetry for this CLI version.
    #[arg(
        long = "no-telemetry",
        conflicts_with = "telemetry",
        default_value_t = false
    )]
    pub no_telemetry: bool,

    /// Accepted for compatibility; `bitloops init` no longer runs the initial baseline sync.
    #[arg(long, default_value_t = false)]
    pub skip_baseline: bool,

    /// Queue an initial DevQL sync after hook setup.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub sync: Option<bool>,

    /// Run historical DevQL ingest after hook setup.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub ingest: Option<bool>,

    /// Bound init-triggered historical ingest to the latest N commits (bare flag = 50).
    #[arg(
        long,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "50",
        value_parser = parse_backfill_value
    )]
    pub backfill: Option<usize>,

    /// Exclude repo-relative paths/globs from DevQL indexing (repeatable).
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Load additional exclusion globs from files under the repo-policy root (repeatable).
    #[arg(long = "exclude-from")]
    pub exclude_from: Vec<String>,

    /// Select which embeddings runtime to configure when embeddings are installed during init.
    #[arg(long, value_enum)]
    pub embeddings_runtime: Option<EmbeddingsRuntime>,

    /// Skip embeddings setup during init.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "embeddings_runtime",
        conflicts_with = "embeddings_gateway_url",
        conflicts_with = "embeddings_api_key_env"
    )]
    pub no_embeddings: bool,

    /// Public platform embeddings endpoint used when `--embeddings-runtime platform` is selected.
    #[arg(long)]
    pub embeddings_gateway_url: Option<String>,

    /// Environment variable that contains the platform gateway bearer token.
    #[arg(long, default_value = "BITLOOPS_PLATFORM_GATEWAY_TOKEN")]
    pub embeddings_api_key_env: String,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_with_io_async(args, &mut out, &mut input, None).await
}

#[cfg(test)]
fn run_with_writer_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating runtime for `bitloops init`")?;
    let mut input = io::Cursor::new(Vec::<u8>::new());
    runtime.block_on(run_with_io_async_for_project_root(
        args,
        project_root,
        out,
        &mut input,
        select_fn,
    ))
}

async fn run_with_io_async(
    args: InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let project_root = std::env::current_dir().context("getting current directory")?;
    run_with_io_async_for_project_root(args, &project_root, out, input, select_fn).await
}

async fn run_with_io_async_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    workflow::run_for_project_root(args, project_root, out, input, select_fn).await
}

fn should_install_embeddings_during_init(
    repo_root: &Path,
    args: &InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    if !matches!(
        inspect_embeddings_install_state(repo_root),
        EmbeddingsInstallState::NotConfigured
    ) {
        return Ok(InitEmbeddingsSetupSelection::Existing);
    }

    if args.install_default_daemon {
        if args.no_embeddings {
            return Ok(InitEmbeddingsSetupSelection::Skip);
        }

        if let Some(runtime) = args.embeddings_runtime {
            return Ok(match runtime {
                EmbeddingsRuntime::Local => InitEmbeddingsSetupSelection::Local,
                EmbeddingsRuntime::Platform => InitEmbeddingsSetupSelection::Cloud,
            });
        }

        if !telemetry_consent::can_prompt_interactively() {
            bail!(NON_INTERACTIVE_INIT_EMBEDDINGS_SELECTION_ERROR);
        }
        return prompt_install_embeddings_setup_selection(out, input);
    }

    if !telemetry_consent::can_prompt_interactively() {
        return Ok(InitEmbeddingsSetupSelection::Skip);
    }

    Ok(if args.no_embeddings {
        InitEmbeddingsSetupSelection::Skip
    } else if prompt_install_embeddings(out, input)? {
        match args.embeddings_runtime {
            Some(EmbeddingsRuntime::Platform) => InitEmbeddingsSetupSelection::Cloud,
            Some(EmbeddingsRuntime::Local) | None => InitEmbeddingsSetupSelection::Local,
        }
    } else {
        InitEmbeddingsSetupSelection::Skip
    })
}

fn prompt_install_embeddings(out: &mut dyn Write, input: &mut dyn BufRead) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "Install local embeddings as well?")?;
    writeln!(
        out,
        "This is recommended and lets sync and ingest include them."
    )?;

    loop {
        writeln!(out, "Install embeddings now? (Y/n)")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading init embeddings install prompt response")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(out, "Please answer yes or no.")?,
        }
    }
}

fn prompt_install_embeddings_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    if can_use_terminal_picker() {
        return prompt_install_embeddings_setup_selection_with_picker(out);
    }

    prompt_install_embeddings_setup_selection_with_text_input(out, input)
}

fn prompt_install_embeddings_setup_selection_with_picker(
    out: &mut dyn Write,
) -> Result<InitEmbeddingsSetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Bitloops Cloud (recommended)",
            vec!["Fast setup. No local compute required.".to_string()],
        ),
        SingleSelectOption::new(
            "Local embeddings",
            vec!["Runs on your machine (~4GB RAM, GPU recommended).".to_string()],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];

    writeln!(out)?;
    let selection = prompt_single_select(
        out,
        "Configure embeddings",
        &[
            "Embeddings power semantic search across your codebase".to_string(),
            "(e.g. “find where authentication is handled”).".to_string(),
            String::new(),
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
                .to_string(),
        ],
        &options,
        0,
        &[],
    )?;

    Ok(match selection {
        0 => InitEmbeddingsSetupSelection::Cloud,
        1 => InitEmbeddingsSetupSelection::Local,
        2 => InitEmbeddingsSetupSelection::Skip,
        _ => unreachable!("terminal picker returned invalid embeddings selection"),
    })
}

fn prompt_install_embeddings_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure embeddings")?;
    writeln!(out)?;
    writeln!(out, "Embeddings power semantic search across your codebase")?;
    writeln!(out, "(e.g. “find where authentication is handled”).")?;
    writeln!(out)?;
    writeln!(
        out,
        "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
    )?;
    writeln!(out)?;
    writeln!(out, "1. Bitloops Cloud (recommended)")?;
    writeln!(out, "   Fast setup. No local compute required.")?;
    writeln!(out, "2. Local embeddings")?;
    writeln!(out, "   Runs on your machine (~4GB RAM, GPU recommended).")?;
    writeln!(out, "3. Skip for now")?;

    loop {
        writeln!(out, "Select an option [1/2/3]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading init embeddings setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "cloud" | "bitloops" => return Ok(InitEmbeddingsSetupSelection::Cloud),
            "2" | "local" => return Ok(InitEmbeddingsSetupSelection::Local),
            "3" | "skip" | "later" | "none" => {
                return Ok(InitEmbeddingsSetupSelection::Skip);
            }
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}

pub(super) async fn choose_summary_setup_during_init(
    repo_root: &Path,
    install_default_daemon: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    let cloud_logged_in = crate::daemon::resolve_workos_session_status()
        .await?
        .is_some();

    prompt_summary_setup_selection(
        out,
        input,
        telemetry_consent::can_prompt_interactively(),
        install_default_daemon,
        cloud_logged_in,
    )
}

#[derive(Clone, Copy)]
enum InitFinalSetupOptionKind {
    Sync,
    Ingest,
    Telemetry,
    AutoStartDaemon,
}

#[derive(Clone, Copy)]
struct InitFinalSetupOptionSpec {
    kind: InitFinalSetupOptionKind,
    label: &'static str,
    insert_spacing_before: bool,
}

fn choose_final_setup_options(
    sync: Option<bool>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    ingest: Option<bool>,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    let defaults = InitFinalSetupSelection {
        sync: sync.unwrap_or(true),
        ingest: ingest.unwrap_or(true),
        telemetry: prompt_options.show_telemetry,
        auto_start_daemon: prompt_options.show_auto_start_daemon,
    };
    let requires_prompt = sync.is_none()
        || ingest.is_none()
        || prompt_options.show_telemetry
        || prompt_options.show_auto_start_daemon;

    if !requires_prompt {
        return Ok(defaults);
    }

    if !telemetry_consent::can_prompt_interactively() {
        bail!(
            "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
        );
    }

    prompt_final_setup_selection(out, input, defaults, prompt_options)
}

fn prompt_final_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    defaults: InitFinalSetupSelection,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    #[cfg(test)]
    let use_picker = false;
    #[cfg(not(test))]
    let use_picker = can_use_terminal_picker();

    if use_picker {
        return prompt_final_setup_selection_with_picker(out, defaults, prompt_options);
    }

    prompt_final_setup_selection_with_text_input(out, input, defaults, prompt_options)
}

fn prompt_final_setup_selection_with_picker(
    out: &mut dyn Write,
    defaults: InitFinalSetupSelection,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    let options = final_setup_option_specs(prompt_options);
    let mut selected = options
        .iter()
        .map(|option| final_setup_selection_value(defaults, option.kind))
        .collect::<Vec<_>>();
    let mut cursor = 0usize;
    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = InitPickerRawMode::enter()?;
    let mut rendered_lines = render_follow_up_picker(out, &options, &selected, cursor, None)?;

    loop {
        match read_follow_up_key(&mut tty_in)? {
            FollowUpKey::Up => {
                cursor = cursor.saturating_sub(1);
            }
            FollowUpKey::Down => {
                if cursor + 1 < options.len() {
                    cursor += 1;
                }
            }
            FollowUpKey::Toggle => {
                selected[cursor] = !selected[cursor];
            }
            FollowUpKey::Cancel => bail!("cancelled by user"),
            FollowUpKey::Submit => break,
            FollowUpKey::Unknown => {}
        }

        rendered_lines =
            render_follow_up_picker(out, &options, &selected, cursor, Some(rendered_lines))?;
    }

    writeln!(out)?;
    out.flush()?;

    let mut selection = InitFinalSetupSelection {
        sync: false,
        ingest: false,
        telemetry: false,
        auto_start_daemon: false,
    };
    for (option, is_selected) in options.iter().zip(selected) {
        set_final_setup_selection_value(&mut selection, option.kind, is_selected);
    }
    Ok(selection)
}

fn prompt_final_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    defaults: InitFinalSetupSelection,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    let options = final_setup_option_specs(prompt_options);
    writeln!(out)?;
    writeln!(out, "Final setup")?;
    writeln!(out)?;
    writeln!(out, "And we made it to the last setup options!:")?;
    writeln!(
        out,
        "{}",
        style_follow_up_hint("Use space to select, enter to confirm.")
    )?;
    writeln!(out)?;
    for (index, option) in options.iter().enumerate() {
        if option.insert_spacing_before {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{}. {}{}",
            index + 1,
            option.label,
            if final_setup_selection_value(defaults, option.kind) {
                " (selected)"
            } else {
                ""
            }
        )?;
    }

    loop {
        let available = (1..=options.len())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "Select options [{available}] (comma-separated, empty to accept defaults)"
        )?;
        write!(out, "> ")?;
        out.flush()?;

        let mut response = String::new();
        input
            .read_line(&mut response)
            .context("reading final setup selection for `bitloops init`")?;
        let response = response.trim().to_ascii_lowercase();
        if response.is_empty() {
            return Ok(defaults);
        }

        if matches!(response.as_str(), "none" | "skip") {
            return Ok(InitFinalSetupSelection {
                sync: false,
                ingest: false,
                telemetry: false,
                auto_start_daemon: false,
            });
        }

        let mut selection = InitFinalSetupSelection {
            sync: false,
            ingest: false,
            telemetry: false,
            auto_start_daemon: false,
        };
        let mut invalid = false;
        for token in response
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if matches!(token, "all" | "everything") {
                for option in &options {
                    set_final_setup_selection_value(&mut selection, option.kind, true);
                }
                continue;
            }

            if token == "both" {
                selection.sync = true;
                selection.ingest = true;
                continue;
            }

            let Some(option) = option_for_final_setup_token(&options, token) else {
                invalid = true;
                break;
            };
            set_final_setup_selection_value(&mut selection, option.kind, true);
        }

        if !invalid {
            return Ok(selection);
        }

        writeln!(
            out,
            "Please choose option numbers, `all`, `none`, or press enter to accept the defaults."
        )?;
    }
}

fn final_setup_option_specs(
    prompt_options: InitFinalSetupPromptOptions,
) -> Vec<InitFinalSetupOptionSpec> {
    let mut options = vec![
        InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::Sync,
            label: "Sync codebase",
            insert_spacing_before: false,
        },
        InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::Ingest,
            label: "Import commit history",
            insert_spacing_before: false,
        },
    ];

    let mut first_setting = true;
    if prompt_options.show_telemetry {
        options.push(InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::Telemetry,
            label: "Enable anonymous telemetry",
            insert_spacing_before: first_setting,
        });
        first_setting = false;
    }
    if prompt_options.show_auto_start_daemon {
        options.push(InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::AutoStartDaemon,
            label: "Start Bitloops daemon automatically when you sign in",
            insert_spacing_before: first_setting,
        });
    }

    options
}

fn final_setup_selection_value(
    selection: InitFinalSetupSelection,
    kind: InitFinalSetupOptionKind,
) -> bool {
    match kind {
        InitFinalSetupOptionKind::Sync => selection.sync,
        InitFinalSetupOptionKind::Ingest => selection.ingest,
        InitFinalSetupOptionKind::Telemetry => selection.telemetry,
        InitFinalSetupOptionKind::AutoStartDaemon => selection.auto_start_daemon,
    }
}

fn set_final_setup_selection_value(
    selection: &mut InitFinalSetupSelection,
    kind: InitFinalSetupOptionKind,
    value: bool,
) {
    match kind {
        InitFinalSetupOptionKind::Sync => selection.sync = value,
        InitFinalSetupOptionKind::Ingest => selection.ingest = value,
        InitFinalSetupOptionKind::Telemetry => selection.telemetry = value,
        InitFinalSetupOptionKind::AutoStartDaemon => selection.auto_start_daemon = value,
    }
}

fn option_for_final_setup_token<'a>(
    options: &'a [InitFinalSetupOptionSpec],
    token: &str,
) -> Option<&'a InitFinalSetupOptionSpec> {
    if let Ok(index) = token.parse::<usize>() {
        return index.checked_sub(1).and_then(|index| options.get(index));
    }

    options.iter().find(|option| match option.kind {
        InitFinalSetupOptionKind::Sync => matches!(token, "sync" | "codebase"),
        InitFinalSetupOptionKind::Ingest => {
            matches!(token, "ingest" | "history" | "commit-history")
        }
        InitFinalSetupOptionKind::Telemetry => matches!(token, "telemetry"),
        InitFinalSetupOptionKind::AutoStartDaemon => matches!(
            token,
            "daemon" | "auto-start" | "autostart" | "startup" | "sign-in"
        ),
    })
}

#[derive(Clone, Copy)]
enum FollowUpKey {
    Up,
    Down,
    Toggle,
    Cancel,
    Submit,
    Unknown,
}

struct InitPickerRawMode {
    original_mode: String,
}

impl InitPickerRawMode {
    fn enter() -> Result<Self> {
        let tty = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .context("opening tty for final setup picker")?;

        let output = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::from(
                tty.try_clone()
                    .context("cloning tty handle for final setup picker")?,
            ))
            .output()
            .context("reading tty mode for final setup picker")?;
        if !output.status.success() {
            bail!("failed to read tty mode");
        }

        let original_mode = String::from_utf8(output.stdout)
            .context("parsing tty mode for final setup picker")?
            .trim()
            .to_string();

        let status = Command::new("stty")
            .args(["-icanon", "-echo", "min", "1", "time", "0"])
            .stdin(Stdio::from(tty))
            .status()
            .context("setting raw tty mode for final setup picker")?;
        if !status.success() {
            bail!("failed to set raw tty mode");
        }

        Ok(Self { original_mode })
    }
}

impl Drop for InitPickerRawMode {
    fn drop(&mut self) {
        if let Ok(tty) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
        {
            let _ = Command::new("stty")
                .arg(self.original_mode.clone())
                .stdin(Stdio::from(tty))
                .status();
        }
    }
}

fn read_follow_up_key(input: &mut dyn Read) -> Result<FollowUpKey> {
    let mut first = [0u8; 1];
    input.read_exact(&mut first)?;
    match first[0] {
        3 => Ok(FollowUpKey::Cancel),
        b' ' => Ok(FollowUpKey::Toggle),
        b'\r' | b'\n' => Ok(FollowUpKey::Submit),
        b'k' => Ok(FollowUpKey::Up),
        b'j' => Ok(FollowUpKey::Down),
        27 => {
            let mut seq = [0u8; 2];
            if input.read_exact(&mut seq).is_err() {
                return Ok(FollowUpKey::Unknown);
            }
            if seq == [b'[', b'A'] {
                Ok(FollowUpKey::Up)
            } else if seq == [b'[', b'B'] {
                Ok(FollowUpKey::Down)
            } else {
                Ok(FollowUpKey::Unknown)
            }
        }
        _ => Ok(FollowUpKey::Unknown),
    }
}

fn render_follow_up_picker(
    out: &mut dyn Write,
    options: &[InitFinalSetupOptionSpec],
    selected: &[bool],
    cursor: usize,
    previous_lines: Option<usize>,
) -> Result<usize> {
    let mut lines = vec![
        "Final setup".to_string(),
        String::new(),
        "And we made it to the last setup options!:".to_string(),
        style_follow_up_hint("Use space to select, enter to confirm."),
        String::new(),
    ];

    for (idx, option) in options.iter().enumerate() {
        if option.insert_spacing_before {
            lines.push(String::new());
        }
        let pointer = if idx == cursor {
            color_hex_if_enabled(">", crate::utils::branding::BITLOOPS_PURPLE_HEX)
        } else {
            " ".to_string()
        };
        let checkbox = if selected[idx] {
            selected_follow_up_checkbox()
        } else {
            "[ ]".to_string()
        };
        let label = if selected[idx] {
            selected_follow_up_label(option.label)
        } else {
            option.label.to_string()
        };
        lines.push(format!("{pointer} {checkbox} {label}"));
    }

    lines.push(String::new());
    lines.push(format!(
        "space {} • ↑/↓ {} • enter {}",
        style_follow_up_hint("toggle"),
        style_follow_up_hint("move"),
        style_follow_up_hint("submit")
    ));

    if let Some(previous_lines) = previous_lines {
        if previous_lines > 1 {
            write!(out, "\x1b[{}F", previous_lines - 1)?;
        } else {
            write!(out, "\r")?;
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        write!(out, "\r\x1b[2K{line}")?;
        if idx + 1 < lines.len() {
            writeln!(out)?;
        }
    }
    out.flush()?;
    Ok(lines.len())
}

fn style_follow_up_hint(detail: &str) -> String {
    if crate::utils::branding::should_use_color_output() {
        format!("\x1b[2;3m{detail}\x1b[0m")
    } else {
        detail.to_string()
    }
}

fn selected_follow_up_checkbox() -> String {
    const SELECTION_WHITE_HEX: &str = "#ffffff";
    format!(
        "{}{}{}",
        color_hex_if_enabled("[", SELECTION_WHITE_HEX),
        color_hex_if_enabled("•", crate::utils::branding::BITLOOPS_PURPLE_HEX),
        color_hex_if_enabled("]", SELECTION_WHITE_HEX)
    )
}

fn selected_follow_up_label(label: &str) -> String {
    const SELECTION_WHITE_HEX: &str = "#ffffff";
    color_hex_if_enabled(label, SELECTION_WHITE_HEX)
}

fn parse_backfill_value(raw: &str) -> std::result::Result<usize, String> {
    let parsed = raw
        .parse::<usize>()
        .map_err(|_| format!("invalid value `{raw}` for `--backfill`"))?;
    if parsed == 0 {
        return Err("`--backfill` must be greater than zero".to_string());
    }
    Ok(parsed)
}

fn normalize_cli_exclusions(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_exclude_from_paths(policy_root: &Path, values: &[String]) -> Result<Vec<String>> {
    let policy_root = policy_root
        .canonicalize()
        .unwrap_or_else(|_| policy_root.to_path_buf());
    let mut normalized = Vec::new();

    for raw_value in values {
        let raw_value = raw_value.trim();
        if raw_value.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(raw_value);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            policy_root.join(candidate)
        };
        let absolute = normalize_lexical_path(&absolute);
        if !absolute.starts_with(&policy_root) {
            bail!(
                "`--exclude-from` path `{}` must be under repo-policy root {}",
                raw_value,
                policy_root.display()
            );
        }
        let relative = absolute
            .strip_prefix(&policy_root)
            .unwrap_or(absolute.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if !relative.is_empty() {
            normalized.push(relative);
        }
    }

    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn default_daemon_server_config() -> crate::api::DashboardServerConfig {
    crate::api::DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    }
}

#[cfg(not(test))]
fn daemon_server_config_from_status(
    runtime: Option<&crate::daemon::DaemonRuntimeState>,
) -> crate::api::DashboardServerConfig {
    runtime.map_or_else(default_daemon_server_config, |runtime| {
        crate::api::DashboardServerConfig {
            host: Some(runtime.host.clone()),
            port: runtime.port,
            no_open: true,
            force_http: runtime.url.starts_with("http://"),
            recheck_local_dashboard_net: false,
            bundle_dir: Some(runtime.bundle_dir.clone()),
        }
    })
}

async fn maybe_install_default_daemon(
    install_default_daemon: bool,
    telemetry: Option<bool>,
) -> Result<()> {
    #[cfg(test)]
    if let Some(result) = maybe_run_install_default_daemon_hook(install_default_daemon) {
        return result;
    }

    if !install_default_daemon {
        return Ok(());
    }

    let status = crate::daemon::status().await?;
    if status.runtime.is_some() {
        return Ok(());
    }

    let config_path = bootstrap_default_daemon_environment()?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(config_path.as_path()))?;
    let _ =
        crate::daemon::start_detached(&daemon_config, default_daemon_server_config(), telemetry)
            .await?;
    Ok(())
}

async fn maybe_enable_default_daemon_service(
    enable_default_daemon_service: bool,
    _daemon_config_path: &Path,
    _telemetry: Option<bool>,
) -> Result<()> {
    if !enable_default_daemon_service {
        return Ok(());
    }

    #[cfg(test)]
    {
        if let Some(result) =
            maybe_run_enable_default_daemon_service_hook(enable_default_daemon_service)
        {
            return result;
        }

        return Ok(());
    }

    #[cfg(not(test))]
    {
        let status = crate::daemon::status().await?;
        let already_service_managed = status
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.mode == crate::daemon::DaemonMode::Service)
            || status.service.is_some();
        if already_service_managed {
            return Ok(());
        }

        let config = daemon_server_config_from_status(status.runtime.as_ref());
        if status.runtime.is_some() {
            crate::daemon::stop().await?;
        }

        let daemon_config = crate::daemon::resolve_daemon_config(Some(_daemon_config_path))?;
        let _ = crate::daemon::start_service(&daemon_config, config, _telemetry).await?;
        Ok(())
    }
}

#[cfg(test)]
fn maybe_run_install_default_daemon_hook(install_default_daemon: bool) -> Option<Result<()>> {
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        cell.borrow()
            .as_ref()
            .map(|hook| hook(install_default_daemon))
    })
}

#[cfg(test)]
pub(super) fn with_install_default_daemon_hook<T>(
    hook: impl Fn(bool) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        assert!(
            cell.borrow().is_none(),
            "install default daemon hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn maybe_run_enable_default_daemon_service_hook(
    enable_default_daemon_service: bool,
) -> Option<Result<()>> {
    ENABLE_DEFAULT_DAEMON_SERVICE_HOOK.with(
        |cell: &RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>>| {
            cell.borrow()
                .as_ref()
                .map(|hook| hook(enable_default_daemon_service))
        },
    )
}

#[cfg(test)]
pub(super) fn with_enable_default_daemon_service_hook<T>(
    hook: impl Fn(bool) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    ENABLE_DEFAULT_DAEMON_SERVICE_HOOK.with(
        |cell: &RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "enable default daemon service hook already installed"
            );
            *cell.borrow_mut() = Some(Rc::new(hook));
        },
    );
    let result = f();
    ENABLE_DEFAULT_DAEMON_SERVICE_HOOK.with(
        |cell: &RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
    result
}

fn ensure_repo_local_policy_excluded(git_root: &Path, project_root: &Path) -> Result<()> {
    let exclude_path = git_root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating git exclude directory {}", parent.display()))?;
    }

    let mut content = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    let relative_local_policy = project_root
        .strip_prefix(git_root)
        .unwrap_or(project_root)
        .join(REPO_POLICY_LOCAL_FILE_NAME);
    let relative_local_policy = relative_local_policy.to_string_lossy().replace('\\', "/");

    let entry = relative_local_policy.as_str();
    if !content.lines().any(|line| line.trim() == entry) {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(entry);
        content.push('\n');
    }

    std::fs::write(&exclude_path, content)
        .with_context(|| format!("writing {}", exclude_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
