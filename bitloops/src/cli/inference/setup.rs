use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table};

use crate::config::{
    InferenceTask, resolve_daemon_config_path_for_repo,
    resolve_inference_capability_config_for_repo,
};
use crate::host::inference::BITLOOPS_INFERENCE_RUNTIME_ID;

use super::managed::install_or_bootstrap_inference;

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_SUMMARY_PROFILE_NAME: &str = "summary_local";
const PREFERRED_OLLAMA_MODELS: &[&str] = &["ministral-3:3b", "ministral-3:8b"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SummarySetupOutcome {
    InstalledRuntimeOnly,
    Configured { model_name: String },
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

    profile.task == InferenceTask::TextGeneration
        && profile
            .runtime
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

pub(crate) fn configure_local_summary_generation(
    repo_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<SummarySetupOutcome> {
    let lines = install_or_bootstrap_inference(repo_root)?;
    for line in lines {
        writeln!(out, "{line}")?;
    }

    let mut availability = probe_ollama_availability()?;
    loop {
        match availability {
            OllamaAvailability::MissingCli => {
                writeln!(
                    out,
                    "Ollama was not found on PATH; installed `bitloops-inference` but skipped semantic summary setup."
                )?;
                return Ok(SummarySetupOutcome::InstalledRuntimeOnly);
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
                        writeln!(
                            out,
                            "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running."
                        )?;
                        return Ok(SummarySetupOutcome::InstalledRuntimeOnly);
                    }
                    _ => {
                        writeln!(out, "Please answer `r` to retry or `s` to skip.")?;
                        continue;
                    }
                }
            }
            OllamaAvailability::NotRunning => {
                writeln!(
                    out,
                    "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running."
                )?;
                return Ok(SummarySetupOutcome::InstalledRuntimeOnly);
            }
            OllamaAvailability::Running { ref models } => {
                let model_name = select_ollama_model(models, out, input, interactive)?;
                let Some(model_name) = model_name else {
                    writeln!(
                        out,
                        "Installed `bitloops-inference`; skipped semantic summary profile setup."
                    )?;
                    return Ok(SummarySetupOutcome::InstalledRuntimeOnly);
                };
                write_summary_profile(repo_root, &model_name)?;
                writeln!(
                    out,
                    "Configured semantic summaries to use Ollama model `{model_name}`."
                )?;
                return Ok(SummarySetupOutcome::Configured { model_name });
            }
        }
    }
}

fn select_ollama_model(
    models: &[String],
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<Option<String>> {
    if let Some(preferred) = PREFERRED_OLLAMA_MODELS
        .iter()
        .find(|candidate| models.iter().any(|model| model == **candidate))
    {
        return Ok(Some((*preferred).to_string()));
    }

    if !interactive {
        return Ok(None);
    }

    if !models.is_empty() {
        writeln!(
            out,
            "Ollama is running. Installed models: {}",
            models.join(", ")
        )?;
    }
    writeln!(
        out,
        "Enter an Ollama model name for semantic summaries, or press Enter to skip:"
    )?;
    write!(out, "> ")?;
    out.flush()?;
    let mut line = String::new();
    input
        .read_line(&mut line)
        .context("reading Ollama model selection")?;
    let selected = line.trim();
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected.to_string()))
    }
}

fn probe_ollama_availability() -> Result<OllamaAvailability> {
    #[cfg(test)]
    if let Some(hook) = OLLAMA_PROBE_HOOK.with(|cell| cell.borrow().clone()) {
        return hook();
    }

    if !command_exists("ollama") {
        return Ok(OllamaAvailability::MissingCli);
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .context("building Ollama probe client")?;
    let response = match client
        .get(format!("{DEFAULT_OLLAMA_BASE_URL}/api/tags"))
        .send()
    {
        Ok(response) => response,
        Err(_) => return Ok(OllamaAvailability::NotRunning),
    };
    if !response.status().is_success() {
        return Ok(OllamaAvailability::NotRunning);
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
    let config_path = resolve_daemon_config_path_for_repo(repo_root)?;
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
        profile["base_url"] = Item::Value(DEFAULT_OLLAMA_BASE_URL.into());
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

fn select_summary_profile_name(profiles: &Table) -> String {
    match profiles
        .get(DEFAULT_SUMMARY_PROFILE_NAME)
        .and_then(Item::as_table)
    {
        None => DEFAULT_SUMMARY_PROFILE_NAME.to_string(),
        Some(profile) if is_managed_summary_profile(profile) => {
            DEFAULT_SUMMARY_PROFILE_NAME.to_string()
        }
        Some(_) => next_available_summary_profile_name(profiles),
    }
}

fn next_available_summary_profile_name(profiles: &Table) -> String {
    let mut suffix = 1usize;
    loop {
        let candidate = format!("{DEFAULT_SUMMARY_PROFILE_NAME}_{suffix}");
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
