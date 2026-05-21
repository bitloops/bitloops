use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;

use super::InitArgs;
use super::args::SummariesRuntime;

pub(crate) async fn choose_summary_setup_during_init(
    repo_root: &Path,
    args: &InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if args.no_summaries {
        return Ok(SummarySetupSelection::Skip);
    }

    if let Some(runtime) = args.summaries_runtime {
        return Ok(match runtime {
            SummariesRuntime::Local => SummarySetupSelection::Local,
            SummariesRuntime::Platform => SummarySetupSelection::Cloud,
        });
    }

    if summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    let interactive = telemetry_consent::can_prompt_interactively();
    if !interactive {
        return Ok(SummarySetupSelection::Skip);
    }

    prompt_summary_setup_selection(out, input, interactive, args.install_default_daemon, false)
}
