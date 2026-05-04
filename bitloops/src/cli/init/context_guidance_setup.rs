use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::cli::inference::{
    ContextGuidanceSetupSelection, TextGenerationRuntime, context_guidance_generation_configured,
    prompt_context_guidance_setup_selection,
};
use crate::cli::telemetry_consent;

use super::InitArgs;

pub(crate) async fn choose_context_guidance_setup_during_init(
    repo_root: &Path,
    args: &InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<ContextGuidanceSetupSelection> {
    if args.no_context_guidance {
        return Ok(ContextGuidanceSetupSelection::Skip);
    }

    if context_guidance_generation_configured(repo_root) {
        return Ok(ContextGuidanceSetupSelection::Skip);
    }

    if !args.install_default_daemon {
        return Ok(ContextGuidanceSetupSelection::Skip);
    }

    if let Some(runtime) = args.context_guidance_runtime {
        return Ok(match runtime {
            TextGenerationRuntime::Local => ContextGuidanceSetupSelection::Local,
            TextGenerationRuntime::Platform => ContextGuidanceSetupSelection::Cloud,
        });
    }
    if args.context_guidance_gateway_url.is_some() || args.context_guidance_api_key_env.is_some() {
        return Ok(ContextGuidanceSetupSelection::Cloud);
    }

    if !telemetry_consent::can_prompt_interactively() {
        return Ok(ContextGuidanceSetupSelection::Skip);
    }

    let cloud_logged_in = crate::daemon::resolve_workos_session_status()
        .await?
        .is_some();

    prompt_context_guidance_setup_selection(
        out,
        input,
        true,
        args.install_default_daemon,
        cloud_logged_in,
    )
}
