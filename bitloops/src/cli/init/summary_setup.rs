use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;
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
