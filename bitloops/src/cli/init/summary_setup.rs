use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;

pub(crate) async fn choose_summary_setup_during_init(
    repo_root: &Path,
    install_default_daemon: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    if !install_default_daemon {
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
