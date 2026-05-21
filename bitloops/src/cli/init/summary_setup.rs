use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;
use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};
use crate::config::SemanticSummaryMode;
use crate::config::settings::repo_semantic_embedding_policy;

use super::cloud_login_status::resolve_cloud_logged_in_for_optional_setup;

pub(crate) async fn choose_summary_setup_during_init(
    repo_root: &Path,
    install_default_daemon: bool,
    no_summaries: bool,
    repo_selected_summaries: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if no_summaries || !repo_selected_summaries {
        return Ok(SummarySetupSelection::Skip);
    }

    if repo_summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    let interactive = telemetry_consent::can_prompt_interactively();
    let cloud_logged_in = if interactive {
        false
    } else {
        resolve_cloud_logged_in_for_optional_setup()
            .await
            .unwrap_or(false)
    };

    prompt_summary_setup_selection(
        out,
        input,
        interactive,
        install_default_daemon,
        cloud_logged_in,
    )
}

pub(crate) fn choose_summary_embeddings_setup_during_init(
    no_embeddings: bool,
    no_summaries: bool,
    repo_selected_summaries: bool,
    repo_selected_summary_embeddings: bool,
    summary_embedding_profile_name: Option<&str>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    if no_embeddings || no_summaries || !repo_selected_summaries {
        return Ok(false);
    }
    if repo_selected_summary_embeddings {
        return Ok(true);
    }
    if summary_embedding_profile_name
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .is_none()
    {
        return Ok(false);
    }
    if !telemetry_consent::can_prompt_interactively() {
        return Ok(false);
    }

    prompt_summary_embeddings_setup_selection(out, input)
}

fn prompt_summary_embeddings_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    if can_use_terminal_picker() {
        return prompt_summary_embeddings_setup_selection_with_picker(out);
    }

    prompt_summary_embeddings_setup_selection_with_text_input(out, input)
}

fn prompt_summary_embeddings_setup_selection_with_picker(out: &mut dyn Write) -> Result<bool> {
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
            "(e.g. “find modules responsible for request routing”).".to_string(),
        ],
        &options,
        0,
        &[],
    )?;

    Ok(selection == 0)
}

fn prompt_summary_embeddings_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "Configure summary embeddings")?;
    writeln!(out)?;
    writeln!(
        out,
        "Summary embeddings power semantic search over generated summaries"
    )?;
    writeln!(
        out,
        "(e.g. “find modules responsible for request routing”)."
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
            "" | "1" | "enable" | "enabled" | "yes" | "y" => return Ok(true),
            "2" | "skip" | "later" | "none" | "no" | "n" => return Ok(false),
            _ => writeln!(out, "Please choose 1 or 2.")?,
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
