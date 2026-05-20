use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;
use crate::config::{SemanticSummaryMode, resolve_semantic_clones_config_for_repo};

use super::cloud_login_status::resolve_cloud_logged_in_for_optional_setup;

pub(crate) async fn choose_summary_setup_during_init(
    repo_root: &Path,
    install_default_daemon: bool,
    no_summaries: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    allow_prompt: bool,
) -> Result<SummarySetupSelection> {
    if no_summaries {
        return Ok(SummarySetupSelection::Skip);
    }

    if resolve_semantic_clones_config_for_repo(repo_root).summary_mode == SemanticSummaryMode::Off {
        return Ok(SummarySetupSelection::Skip);
    }

    if summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    if !allow_prompt {
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
