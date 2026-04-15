use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table};

use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};
use crate::config::{
    InferenceTask, resolve_inference_capability_config_for_repo,
    resolve_preferred_daemon_config_path_for_repo,
};
use crate::host::inference::{BITLOOPS_INFERENCE_RUNTIME_ID, BITLOOPS_PLATFORM_CHAT_DRIVER};

use super::managed::{
    ManagedInferenceInstallPhase, ManagedInferenceInstallProgress, install_or_bootstrap_inference,
    install_or_bootstrap_inference_with_progress,
};

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OLLAMA_CHAT_BASE_URL: &str = "http://127.0.0.1:11434/api/chat";
const DEFAULT_SUMMARY_TEMPERATURE: &str = "0.1";
const DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS: i64 = 200;
const DEFAULT_SUMMARY_PROFILE_NAME: &str = "summary_local";
const DEFAULT_PLATFORM_SUMMARY_PROFILE_NAME: &str = "summary_llm";
const DEFAULT_PLATFORM_SUMMARY_MODEL: &str = "ministral-3-3b-instruct";
const DEFAULT_PLATFORM_SUMMARY_API_KEY: &str = "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}";
const PLATFORM_CHAT_COMPLETIONS_URL_ENV: &str = "BITLOOPS_PLATFORM_CHAT_COMPLETIONS_URL";
const PLATFORM_GATEWAY_URL_ENV: &str = "BITLOOPS_PLATFORM_GATEWAY_URL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SummarySetupSelection {
    Cloud,
    Local,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SummarySetupOutcome {
    InstalledRuntimeOnly,
    Configured { model_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SummarySetupPhase {
    Queued,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
    WritingProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarySetupProgress {
    pub(crate) phase: SummarySetupPhase,
    pub(crate) asset_name: Option<String>,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) version: Option<String>,
    pub(crate) message: Option<String>,
}

impl Default for SummarySetupProgress {
    fn default() -> Self {
        Self {
            phase: SummarySetupPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarySetupExecutionResult {
    pub(crate) outcome: SummarySetupOutcome,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedSummarySetupAction {
    InstallRuntimeOnly {
        message: String,
    },
    InstallRuntimeOnlyPendingProbe {
        message: String,
    },
    ConfigureLocal {
        model_name: String,
    },
    ConfigureCloud {
        gateway_url_override: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedSummarySetupPlan {
    action: PreparedSummarySetupAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OllamaAvailability {
    MissingCli,
    NotRunning,
    Running { models: Vec<String> },
}

#[cfg(test)]
type OllamaProbeHook = dyn Fn() -> Result<OllamaAvailability>;
#[cfg(test)]
type SummaryGenerationConfiguredHook = dyn Fn(&Path) -> bool;
#[cfg(test)]
type OllamaProbeHookCell = std::cell::RefCell<Option<std::rc::Rc<OllamaProbeHook>>>;
#[cfg(test)]
type SummaryGenerationConfiguredHookCell =
    std::cell::RefCell<Option<std::rc::Rc<SummaryGenerationConfiguredHook>>>;

#[cfg(test)]
thread_local! {
    static OLLAMA_PROBE_HOOK: OllamaProbeHookCell = std::cell::RefCell::new(None);
    static SUMMARY_GENERATION_CONFIGURED_HOOK: SummaryGenerationConfiguredHookCell =
        std::cell::RefCell::new(None);
}

pub(crate) fn summary_generation_configured(repo_root: &Path) -> bool {
    #[cfg(test)]
    if let Some(hook) = SUMMARY_GENERATION_CONFIGURED_HOOK.with(|cell| cell.borrow().clone()) {
        return hook(repo_root);
    }

    let capability = resolve_inference_capability_config_for_repo(repo_root);
    let Some(profile_name) = capability
        .semantic_clones
        .inference
        .summary_generation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    let Some(profile) = capability.inference.profiles.get(profile_name) else {
        return false;
    };
    let Some(runtime_name) = profile
        .runtime
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(runtime) = capability.inference.runtimes.get(runtime_name) else {
        return false;
    };

    let driver = profile.driver.trim();

    profile.task == InferenceTask::TextGeneration
        && profile
            .model
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && profile
            .runtime
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && !runtime.command.trim().is_empty()
        && (driver == BITLOOPS_PLATFORM_CHAT_DRIVER
            || profile
                .base_url
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()))
        && profile
            .temperature
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && profile.max_output_tokens.is_some_and(|value| value > 0)
}

pub(crate) fn configure_local_summary_generation(
    repo_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<SummarySetupOutcome> {
    let plan = prepare_local_summary_generation_plan(out, input, interactive)?;
    let lines = install_or_bootstrap_inference(repo_root)?;
    for line in lines {
        writeln!(out, "{line}")?;
    }

    let execution = apply_prepared_summary_setup(repo_root, plan)?;
    writeln!(out, "{}", execution.message)?;
    Ok(execution.outcome)
}

pub(crate) fn configure_cloud_summary_generation(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
) -> Result<String> {
    let _ = install_or_bootstrap_inference(repo_root)?;
    let execution = apply_prepared_summary_setup(
        repo_root,
        prepare_cloud_summary_generation_plan(gateway_url_override),
    )?;
    Ok(execution.message)
}

pub(crate) fn prepare_cloud_summary_generation_plan(
    gateway_url_override: Option<&str>,
) -> PreparedSummarySetupPlan {
    PreparedSummarySetupPlan {
        action: PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override: gateway_url_override.map(str::to_string),
        },
    }
}

pub(crate) fn platform_summary_gateway_url_override() -> Option<String> {
    let explicit = read_non_empty_env_value(PLATFORM_CHAT_COMPLETIONS_URL_ENV);
    if explicit.is_some() {
        return explicit;
    }

    read_non_empty_env_value(PLATFORM_GATEWAY_URL_ENV).map(|base_url| {
        let trimmed = base_url.trim_end_matches('/');
        format!("{trimmed}/v1/chat/completions")
    })
}

pub(crate) fn prompt_summary_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
    default_to_local_when_noninteractive: bool,
    cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    if !interactive {
        return Ok(if cloud_logged_in {
            SummarySetupSelection::Cloud
        } else if default_to_local_when_noninteractive {
            SummarySetupSelection::Local
        } else {
            SummarySetupSelection::Skip
        });
    }

    if can_use_terminal_picker() {
        return prompt_summary_setup_selection_with_picker(out, cloud_logged_in);
    }

    prompt_summary_setup_selection_with_text_input(out, input, cloud_logged_in)
}

fn prompt_summary_setup_selection_with_picker(
    out: &mut dyn Write,
    cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Bitloops cloud (recommended)",
            vec![
                "Requires you to create or use your free Bitloops account. No local model needed."
                    .to_string(),
            ],
        ),
        SingleSelectOption::new(
            "Local Ollama",
            vec![
                "No code leaves your machine but requires RAM >32GB and GPU acceleration (64GB+ recommended)."
                    .to_string(),
            ],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];
    let mut footer = Vec::new();
    if !cloud_logged_in {
        footer.push(
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
                .to_string(),
        );
    }

    let selection = prompt_single_select(
        out,
        "How would you like Bitloops to configure semantic summaries?",
        &options,
        0,
        &footer,
    )?;

    Ok(match selection {
        0 => SummarySetupSelection::Cloud,
        1 => SummarySetupSelection::Local,
        2 => SummarySetupSelection::Skip,
        _ => unreachable!("terminal picker returned invalid summary selection"),
    })
}

fn prompt_summary_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    writeln!(out)?;
    writeln!(
        out,
        "How would you like Bitloops to configure semantic summaries?"
    )?;
    writeln!(out, "1. Bitloops cloud (recommended)")?;
    writeln!(
        out,
        "   Requires you to create or use your free Bitloops account. No local model needed."
    )?;
    writeln!(out, "2. Local Ollama")?;
    writeln!(
        out,
        "   No code leaves your machine but requires RAM >32GB and GPU acceleration (64GB+ recommended)."
    )?;
    writeln!(out, "3. Skip for now")?;
    if !cloud_logged_in {
        writeln!(
            out,
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
        )?;
    }

    loop {
        writeln!(out, "Select an option [1/2/3]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading semantic summary setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "cloud" | "bitloops" => return Ok(SummarySetupSelection::Cloud),
            "2" | "local" | "ollama" => return Ok(SummarySetupSelection::Local),
            "3" | "skip" | "later" => return Ok(SummarySetupSelection::Skip),
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}

pub(crate) fn prepare_local_summary_generation_plan(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<PreparedSummarySetupPlan> {
    let mut availability = probe_ollama_availability()?;
    loop {
        match availability {
            OllamaAvailability::MissingCli => {
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::InstallRuntimeOnly {
                        message: "Ollama was not found on PATH; installed `bitloops-inference` but skipped semantic summary setup.".to_string(),
                    },
                });
            }
            OllamaAvailability::NotRunning if interactive => {
                writeln!(
                    out,
                    "Ollama is installed but not responding at {DEFAULT_OLLAMA_BASE_URL}."
                )?;
                writeln!(out, "Retry summary setup or skip it for now? (r/S)")?;
                write!(out, "> ")?;
                out.flush()?;
                let mut line = String::new();
                input
                    .read_line(&mut line)
                    .context("reading Ollama retry prompt response")?;
                match line.trim().to_ascii_lowercase().as_str() {
                    "r" | "retry" => {
                        availability = probe_ollama_availability()?;
                        continue;
                    }
                    "" | "s" | "skip" => {
                        return Ok(PreparedSummarySetupPlan {
                            action: PreparedSummarySetupAction::InstallRuntimeOnly {
                                message: "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running.".to_string(),
                            },
                        });
                    }
                    _ => {
                        writeln!(out, "Please answer `r` to retry or `s` to skip.")?;
                        continue;
                    }
                }
            }
            OllamaAvailability::NotRunning => {
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe {
                        message: "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running.".to_string(),
                    },
                });
            }
            OllamaAvailability::Running { ref models } => {
                let model_name = select_ollama_model(models, out, input, interactive)?;
                let Some(model_name) = model_name else {
                    return Ok(PreparedSummarySetupPlan {
                        action: PreparedSummarySetupAction::InstallRuntimeOnly {
                            message: "Installed `bitloops-inference`; skipped semantic summary profile setup.".to_string(),
                        },
                    });
                };
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::ConfigureLocal { model_name },
                });
            }
        }
    }
}

pub(crate) fn execute_prepared_summary_setup_with_progress<R>(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
    mut report: R,
) -> Result<SummarySetupExecutionResult>
where
    R: FnMut(SummarySetupProgress) -> Result<()>,
{
    install_or_bootstrap_inference_with_progress(repo_root, |progress| {
        report(summary_setup_progress_from_managed(progress))
    })?;
    apply_prepared_summary_setup_with_progress(repo_root, plan, &mut report)
}

fn apply_prepared_summary_setup(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
) -> Result<SummarySetupExecutionResult> {
    match plan.action {
        PreparedSummarySetupAction::InstallRuntimeOnly { message } => {
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            if let Some(model_name) = auto_configured_summary_model_name()? {
                write_summary_profile(repo_root, &model_name)?;
                return Ok(SummarySetupExecutionResult {
                    outcome: SummarySetupOutcome::Configured {
                        model_name: model_name.clone(),
                    },
                    message: format!(
                        "Configured semantic summaries to use Ollama model `{model_name}`."
                    ),
                });
            }

            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            write_summary_profile(repo_root, &model_name)?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: model_name.clone(),
                },
                message: format!(
                    "Configured semantic summaries to use Ollama model `{model_name}`."
                ),
            })
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
        } => {
            write_platform_summary_profile(repo_root, gateway_url_override.as_deref())?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: DEFAULT_PLATFORM_SUMMARY_MODEL.to_string(),
                },
                message: "Configured semantic summaries to use Bitloops cloud summaries."
                    .to_string(),
            })
        }
    }
}

fn apply_prepared_summary_setup_with_progress<R>(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
    report: &mut R,
) -> Result<SummarySetupExecutionResult>
where
    R: FnMut(SummarySetupProgress) -> Result<()>,
{
    match plan.action {
        PreparedSummarySetupAction::InstallRuntimeOnly { message } => {
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            report(SummarySetupProgress {
                phase: SummarySetupPhase::WritingProfile,
                message: Some("Rechecking Ollama before applying summary profile".to_string()),
                ..Default::default()
            })?;
            if let Some(model_name) = auto_configured_summary_model_name()? {
                report(SummarySetupProgress {
                    phase: SummarySetupPhase::WritingProfile,
                    message: Some(format!("Applying summary profile for `{model_name}`")),
                    ..Default::default()
                })?;
                write_summary_profile(repo_root, &model_name)?;
                return Ok(SummarySetupExecutionResult {
                    outcome: SummarySetupOutcome::Configured {
                        model_name: model_name.clone(),
                    },
                    message: format!(
                        "Configured semantic summaries to use Ollama model `{model_name}`."
                    ),
                });
            }

            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            report(SummarySetupProgress {
                phase: SummarySetupPhase::WritingProfile,
                message: Some(format!("Applying summary profile for `{model_name}`")),
                ..Default::default()
            })?;
            write_summary_profile(repo_root, &model_name)?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: model_name.clone(),
                },
                message: format!(
                    "Configured semantic summaries to use Ollama model `{model_name}`."
                ),
            })
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
        } => {
            report(SummarySetupProgress {
                phase: SummarySetupPhase::WritingProfile,
                message: Some("Applying Bitloops cloud summary profile".to_string()),
                ..Default::default()
            })?;
            write_platform_summary_profile(repo_root, gateway_url_override.as_deref())?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: DEFAULT_PLATFORM_SUMMARY_MODEL.to_string(),
                },
                message: "Configured semantic summaries to use Bitloops cloud summaries."
                    .to_string(),
            })
        }
    }
}

fn summary_setup_progress_from_managed(
    progress: ManagedInferenceInstallProgress,
) -> SummarySetupProgress {
    SummarySetupProgress {
        phase: match progress.phase {
            ManagedInferenceInstallPhase::Queued => SummarySetupPhase::Queued,
            ManagedInferenceInstallPhase::ResolvingRelease => SummarySetupPhase::ResolvingRelease,
            ManagedInferenceInstallPhase::DownloadingRuntime => {
                SummarySetupPhase::DownloadingRuntime
            }
            ManagedInferenceInstallPhase::ExtractingRuntime => SummarySetupPhase::ExtractingRuntime,
            ManagedInferenceInstallPhase::RewritingRuntime => SummarySetupPhase::RewritingRuntime,
        },
        asset_name: progress.asset_name,
        bytes_downloaded: progress.bytes_downloaded,
        bytes_total: progress.bytes_total,
        version: progress.version,
        message: progress.message,
    }
}

fn auto_configured_summary_model_name() -> Result<Option<String>> {
    match probe_ollama_availability()? {
        OllamaAvailability::Running { models } => Ok(select_preferred_ollama_model(&models)),
        OllamaAvailability::MissingCli | OllamaAvailability::NotRunning => Ok(None),
    }
}

fn select_preferred_ollama_model(models: &[String]) -> Option<String> {
    models
        .iter()
        .find(|model| is_recommended_ollama_model(model))
        .cloned()
}

fn select_ollama_model(
    models: &[String],
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<Option<String>> {
    if !interactive {
        return Ok(select_preferred_ollama_model(models));
    }

    if models.is_empty() {
        writeln!(out, "Ollama is running, but no models are installed.")?;
        return Ok(None);
    }

    let default_model = select_preferred_ollama_model(models);
    writeln!(out, "Select an Ollama model for semantic summaries:")?;
    for (index, model) in models.iter().enumerate() {
        let suffix = if Some(model) == default_model.as_ref() {
            " (mistral-3-3b recommended)"
        } else {
            ""
        };
        writeln!(out, "  {}. {}{}", index + 1, model, suffix)?;
    }

    if let Some(model_name) = default_model.as_ref() {
        writeln!(
            out,
            "Press Enter to use `{model_name}`, type a number to choose another model, or `s` to skip:"
        )?;
    } else {
        writeln!(out, "Type a number to choose a model, or `s` to skip:")?;
    }

    loop {
        write!(out, "> ")?;
        out.flush()?;
        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading Ollama model selection")?;
        let selected = line.trim();
        if selected.is_empty() {
            if let Some(model_name) = default_model.clone() {
                return Ok(Some(model_name));
            }
            writeln!(out, "Please choose a model number or enter `s` to skip.")?;
            continue;
        }
        if matches!(selected.to_ascii_lowercase().as_str(), "s" | "skip") {
            return Ok(None);
        }
        if let Ok(index) = selected.parse::<usize>()
            && (1..=models.len()).contains(&index)
        {
            return Ok(Some(models[index - 1].clone()));
        }
        if let Some(model_name) = models.iter().find(|model| model.as_str() == selected) {
            return Ok(Some(model_name.clone()));
        }
        writeln!(
            out,
            "Please choose one of the listed models or enter `s` to skip."
        )?;
    }
}

fn probe_ollama_availability() -> Result<OllamaAvailability> {
    #[cfg(test)]
    if let Some(hook) = OLLAMA_PROBE_HOOK.with(|cell| cell.borrow().clone()) {
        return hook();
    }

    let cli_available = command_exists("ollama");
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .context("building Ollama probe client")?;
    let response = match client
        .get(format!("{DEFAULT_OLLAMA_BASE_URL}/api/tags"))
        .send()
    {
        Ok(response) => response,
        Err(_) => {
            return Ok(if cli_available {
                OllamaAvailability::NotRunning
            } else {
                OllamaAvailability::MissingCli
            });
        }
    };
    if !response.status().is_success() {
        return Ok(if cli_available {
            OllamaAvailability::NotRunning
        } else {
            OllamaAvailability::MissingCli
        });
    }
    let payload = response
        .json::<OllamaTagsResponse>()
        .context("parsing Ollama model list")?;
    Ok(OllamaAvailability::Running {
        models: payload.models.into_iter().map(|model| model.name).collect(),
    })
}

fn command_exists(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    let candidate = std::path::Path::new(command);
    if candidate.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return candidate.exists();
    }

    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(command).exists()))
        .unwrap_or(false)
}

fn write_summary_profile(repo_root: &Path, model_name: &str) -> Result<()> {
    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", config_path.display())
            });
        }
    };
    let mut doc = if contents.trim().is_empty() {
        DocumentMut::new()
    } else {
        contents
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", config_path.display()))?
    };

    let profile_name = {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        select_summary_profile_name(profiles)
    };

    {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        let profile = ensure_child_table(profiles, &profile_name);
        profile["task"] = Item::Value("text_generation".into());
        profile["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        profile["driver"] = Item::Value("ollama_chat".into());
        profile["model"] = Item::Value(model_name.into());
        profile["base_url"] = Item::Value(DEFAULT_OLLAMA_CHAT_BASE_URL.into());
        profile["temperature"] = Item::Value(DEFAULT_SUMMARY_TEMPERATURE.into());
        profile["max_output_tokens"] = Item::Value(DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS.into());
        profile.remove("api_key");
        profile.remove("cache_dir");
    }

    let semantic_clones = ensure_table(&mut doc, "semantic_clones");
    let semantic_inference = ensure_child_table(semantic_clones, "inference");
    semantic_inference["summary_generation"] = Item::Value(profile_name.as_str().into());

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", config_path.display()))?;
    Ok(())
}

fn write_platform_summary_profile(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
) -> Result<()> {
    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", config_path.display())
            });
        }
    };
    let mut doc = if contents.trim().is_empty() {
        DocumentMut::new()
    } else {
        contents
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", config_path.display()))?
    };

    let profile_name = {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        select_profile_name(
            profiles,
            DEFAULT_PLATFORM_SUMMARY_PROFILE_NAME,
            is_managed_platform_summary_profile,
        )
    };

    {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        let profile = ensure_child_table(profiles, &profile_name);
        profile["task"] = Item::Value("text_generation".into());
        profile["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        profile["driver"] = Item::Value(BITLOOPS_PLATFORM_CHAT_DRIVER.into());
        profile["model"] = Item::Value(DEFAULT_PLATFORM_SUMMARY_MODEL.into());
        profile["api_key"] = Item::Value(DEFAULT_PLATFORM_SUMMARY_API_KEY.into());
        if let Some(gateway_url_override) = gateway_url_override {
            profile["base_url"] = Item::Value(gateway_url_override.into());
        } else {
            profile.remove("base_url");
        }
        profile["temperature"] = Item::Value(DEFAULT_SUMMARY_TEMPERATURE.into());
        profile["max_output_tokens"] = Item::Value(DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS.into());
        profile.remove("cache_dir");
    }

    let semantic_clones = ensure_table(&mut doc, "semantic_clones");
    let semantic_inference = ensure_child_table(semantic_clones, "inference");
    semantic_inference["summary_generation"] = Item::Value(profile_name.as_str().into());

    std::fs::write(&config_path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", config_path.display()))?;
    Ok(())
}

fn select_summary_profile_name(profiles: &Table) -> String {
    select_profile_name(
        profiles,
        DEFAULT_SUMMARY_PROFILE_NAME,
        is_managed_summary_profile,
    )
}

fn select_profile_name(
    profiles: &Table,
    default_name: &str,
    is_managed_profile: fn(&Table) -> bool,
) -> String {
    match profiles.get(default_name).and_then(Item::as_table) {
        None => default_name.to_string(),
        Some(profile) if is_managed_profile(profile) => default_name.to_string(),
        Some(_) => next_available_profile_name(profiles, default_name),
    }
}

fn next_available_profile_name(profiles: &Table, prefix: &str) -> String {
    let mut suffix = 1usize;
    loop {
        let candidate = format!("{prefix}_{suffix}");
        if !profiles.contains_key(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn is_managed_summary_profile(profile: &Table) -> bool {
    profile
        .get("task")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        == Some("text_generation")
        && profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some(BITLOOPS_INFERENCE_RUNTIME_ID)
        && profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some("ollama_chat")
}

fn is_managed_platform_summary_profile(profile: &Table) -> bool {
    profile
        .get("task")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        == Some("text_generation")
        && profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some(BITLOOPS_INFERENCE_RUNTIME_ID)
        && profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|driver| {
                driver == BITLOOPS_PLATFORM_CHAT_DRIVER || driver == "openai_chat_completions"
            })
        && profile
            .get("api_key")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_none_or(|api_key| api_key == "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}")
}

fn is_recommended_ollama_model(model_name: &str) -> bool {
    matches!(
        normalised_ollama_model_name(model_name).as_str(),
        "mistral-3-3b" | "ministral-3-3b"
    )
}

fn normalised_ollama_model_name(model_name: &str) -> String {
    model_name
        .trim()
        .to_ascii_lowercase()
        .replace([':', '_'], "-")
}

fn read_non_empty_env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.contains_key(key) || !doc[key].is_table() {
        doc[key] = Item::Table(Table::new());
    }
    doc[key].as_table_mut().expect("table inserted above")
}

fn ensure_child_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !table.contains_key(key) || !table[key].is_table() {
        table[key] = Item::Table(Table::new());
    }
    table[key].as_table_mut().expect("table inserted above")
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}

#[cfg(test)]
pub(crate) fn with_ollama_probe_hook<T>(
    hook: impl Fn() -> Result<OllamaAvailability> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    OLLAMA_PROBE_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "Ollama probe hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    OLLAMA_PROBE_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
pub(crate) fn with_summary_generation_configured_hook<T>(
    hook: impl Fn(&Path) -> bool + 'static,
    f: impl FnOnce() -> T,
) -> T {
    SUMMARY_GENERATION_CONFIGURED_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "summary generation configured hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    SUMMARY_GENERATION_CONFIGURED_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}
