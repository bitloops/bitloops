use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};

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
use crate::config::SemanticSummaryMode;
use crate::config::settings::repo_semantic_embedding_policy;

use super::SummariesRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitEmbeddingsSetupSelection {
    Unchanged,
    Existing,
    Cloud,
    Local,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitSummaryEmbeddingsSetupSelection {
    Existing,
    UseSelectedEmbeddingsProvider,
    Cloud,
    Local,
    Skip,
}

pub(crate) fn choose_embeddings_setup_during_init(
    repo_root: &Path,
    repo_selected_embedding_lanes: bool,
    explicit_runtime: Option<EmbeddingsRuntime>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    if !repo_selected_embedding_lanes {
        return Ok(InitEmbeddingsSetupSelection::Skip);
    }

    if !matches!(
        inspect_embeddings_install_state(repo_root),
        EmbeddingsInstallState::NotConfigured
    ) {
        return Ok(InitEmbeddingsSetupSelection::Existing);
    }

    if let Some(runtime) = explicit_runtime {
        return Ok(match runtime {
            EmbeddingsRuntime::Local => InitEmbeddingsSetupSelection::Local,
            EmbeddingsRuntime::Platform => InitEmbeddingsSetupSelection::Cloud,
        });
    }

    if !telemetry_consent::can_prompt_interactively() {
        return Ok(InitEmbeddingsSetupSelection::Unchanged);
    }

    prompt_embeddings_setup_selection(out, input)
}

pub(crate) fn choose_summary_setup_during_init(
    repo_root: &Path,
    repo_selected_summaries: bool,
    explicit_runtime: Option<SummariesRuntime>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if !repo_selected_summaries {
        return Ok(SummarySetupSelection::Skip);
    }

    if repo_summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    if let Some(runtime) = explicit_runtime {
        return Ok(match runtime {
            SummariesRuntime::Local => SummarySetupSelection::Local,
            SummariesRuntime::Platform => SummarySetupSelection::Cloud,
        });
    }

    prompt_summary_setup_selection(
        out,
        input,
        telemetry_consent::can_prompt_interactively(),
        false,
        false,
    )
}

pub(crate) fn choose_summary_embeddings_setup_during_init(
    repo_selected_summaries: bool,
    repo_selected_summary_embeddings: bool,
    summary_embedding_profile_name: Option<&str>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
    if !repo_selected_summaries {
        return Ok(InitSummaryEmbeddingsSetupSelection::Skip);
    }
    if repo_selected_summary_embeddings {
        return Ok(InitSummaryEmbeddingsSetupSelection::Existing);
    }
    if !telemetry_consent::can_prompt_interactively() {
        return Ok(InitSummaryEmbeddingsSetupSelection::Skip);
    }
    if summary_embedding_profile_name
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .is_some()
    {
        return prompt_summary_embeddings_setup_selection(out, input);
    }

    prompt_summary_embeddings_provider_selection(out, input)
}

fn prompt_embeddings_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    if can_use_terminal_picker() {
        return prompt_embeddings_setup_selection_with_picker(out);
    }

    prompt_embeddings_setup_selection_with_text_input(out, input)
}

fn prompt_embeddings_setup_selection_with_picker(
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
            "Choosing Bitloops Cloud will open the Bitloops sign-in flow in your browser."
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

fn prompt_embeddings_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure embeddings")?;
    writeln!(out)?;
    writeln!(out, "Embeddings power semantic search across your codebase")?;
    writeln!(out)?;
    writeln!(
        out,
        "Choosing Bitloops Cloud will open the Bitloops sign-in flow in your browser."
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

fn prompt_summary_embeddings_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
    if can_use_terminal_picker() {
        return prompt_summary_embeddings_setup_selection_with_picker(out);
    }

    prompt_summary_embeddings_setup_selection_with_text_input(out, input)
}

fn prompt_summary_embeddings_setup_selection_with_picker(
    out: &mut dyn Write,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Enable summary embeddings (recommended)",
            vec!["Use the selected embeddings provider for generated summaries.".to_string()],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];

    writeln!(out)?;
    let selection = prompt_single_select(
        out,
        "Configure summary embeddings",
        &[
            "Summary embeddings power semantic search over generated summaries".to_string(),
            "For example, find modules responsible for request routing.".to_string(),
        ],
        &options,
        0,
        &[],
    )?;

    Ok(if selection == 0 {
        InitSummaryEmbeddingsSetupSelection::UseSelectedEmbeddingsProvider
    } else {
        InitSummaryEmbeddingsSetupSelection::Skip
    })
}

fn prompt_summary_embeddings_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure summary embeddings")?;
    writeln!(out)?;
    writeln!(
        out,
        "Summary embeddings power semantic search over generated summaries"
    )?;
    writeln!(
        out,
        "For example, find modules responsible for request routing."
    )?;
    writeln!(out)?;
    writeln!(out, "1. Enable summary embeddings (recommended)")?;
    writeln!(
        out,
        "   Use the selected embeddings provider for generated summaries."
    )?;
    writeln!(out, "2. Skip for now")?;

    loop {
        writeln!(out, "Select an option [1/2]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading init summary embeddings setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "enable" | "enabled" | "yes" | "y" => {
                return Ok(InitSummaryEmbeddingsSetupSelection::UseSelectedEmbeddingsProvider);
            }
            "2" | "skip" | "later" | "none" | "no" | "n" => {
                return Ok(InitSummaryEmbeddingsSetupSelection::Skip);
            }
            _ => writeln!(out, "Please choose 1 or 2.")?,
        }
    }
}

fn prompt_summary_embeddings_provider_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
    if can_use_terminal_picker() {
        return prompt_summary_embeddings_provider_selection_with_picker(out);
    }

    prompt_summary_embeddings_provider_selection_with_text_input(out, input)
}

fn prompt_summary_embeddings_provider_selection_with_picker(
    out: &mut dyn Write,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
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
        "Configure summary embeddings",
        &[
            "Summary embeddings power semantic search over generated summaries".to_string(),
            "They can use their own embeddings provider even when code embeddings are skipped."
                .to_string(),
        ],
        &options,
        0,
        &[],
    )?;

    Ok(match selection {
        0 => InitSummaryEmbeddingsSetupSelection::Cloud,
        1 => InitSummaryEmbeddingsSetupSelection::Local,
        2 => InitSummaryEmbeddingsSetupSelection::Skip,
        _ => unreachable!("terminal picker returned invalid summary embeddings selection"),
    })
}

fn prompt_summary_embeddings_provider_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitSummaryEmbeddingsSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure summary embeddings")?;
    writeln!(out)?;
    writeln!(
        out,
        "Summary embeddings power semantic search over generated summaries"
    )?;
    writeln!(
        out,
        "They can use their own embeddings provider even when code embeddings are skipped."
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
            .context("reading init summary embeddings provider selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "cloud" | "bitloops" => {
                return Ok(InitSummaryEmbeddingsSetupSelection::Cloud);
            }
            "2" | "local" => return Ok(InitSummaryEmbeddingsSetupSelection::Local),
            "3" | "skip" | "later" | "none" => {
                return Ok(InitSummaryEmbeddingsSetupSelection::Skip);
            }
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}

fn repo_summary_generation_configured(repo_root: &Path) -> bool {
    let Ok(policy) = repo_semantic_embedding_policy(repo_root) else {
        return false;
    };
    if policy.summary_mode == Some(SemanticSummaryMode::Off) {
        return false;
    }
    let repo_selected_summary_profile = policy
        .inference
        .summary_generation
        .as_deref()
        .map(str::trim)
        .is_some_and(|profile| !profile.is_empty());
    repo_selected_summary_profile && summary_generation_configured(repo_root)
}
