use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

use super::constants::DEFAULT_OLLAMA_BASE_URL;
use super::types::OllamaAvailability;

#[cfg(test)]
type OllamaProbeHook = dyn Fn() -> Result<OllamaAvailability>;
#[cfg(test)]
type OllamaProbeHookCell = std::cell::RefCell<Option<std::rc::Rc<OllamaProbeHook>>>;

#[cfg(test)]
thread_local! {
    static OLLAMA_PROBE_HOOK: OllamaProbeHookCell = std::cell::RefCell::new(None);
}

pub(super) fn auto_configured_summary_model_name() -> Result<Option<String>> {
    match probe_ollama_availability()? {
        OllamaAvailability::Running { models } => Ok(select_preferred_ollama_model(&models)),
        OllamaAvailability::MissingCli | OllamaAvailability::NotRunning => Ok(None),
    }
}

pub(super) fn probe_ollama_availability() -> Result<OllamaAvailability> {
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

pub(super) fn select_ollama_model(
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

fn select_preferred_ollama_model(models: &[String]) -> Option<String> {
    models
        .iter()
        .find(|model| is_recommended_ollama_model(model))
        .cloned()
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
