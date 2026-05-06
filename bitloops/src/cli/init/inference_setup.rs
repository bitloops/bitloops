use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Result, bail};

use crate::cli::inference::{
    BitloopsInferenceSetupSelection, TextGenerationRuntime,
    bitloops_inference_generation_configured, prompt_bitloops_inference_setup_selection,
};
use crate::cli::telemetry_consent;

use super::InitArgs;

pub(crate) const NON_INTERACTIVE_INIT_BITLOOPS_INFERENCE_SELECTION_ERROR: &str = "`bitloops init --install-default-daemon` requires an explicit Bitloops inference choice when not running interactively. Pass `--bitloops-inference-runtime local`, `--bitloops-inference-runtime platform`, or `--no-bitloops-inference`.";

pub(crate) async fn choose_bitloops_inference_setup_during_init(
    repo_root: &Path,
    args: &InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<BitloopsInferenceSetupSelection> {
    if args.no_bitloops_inference || args.no_summaries || args.no_context_guidance {
        return Ok(BitloopsInferenceSetupSelection::Skip);
    }

    if bitloops_inference_generation_configured(repo_root) {
        return Ok(BitloopsInferenceSetupSelection::Skip);
    }

    if !args.install_default_daemon {
        return Ok(BitloopsInferenceSetupSelection::Skip);
    }

    if let Some(runtime) = args
        .bitloops_inference_runtime
        .or(args.context_guidance_runtime)
    {
        return Ok(match runtime {
            TextGenerationRuntime::Local => BitloopsInferenceSetupSelection::Local,
            TextGenerationRuntime::Platform => BitloopsInferenceSetupSelection::Cloud,
        });
    }

    if args.bitloops_inference_gateway_url.is_some()
        || args.bitloops_inference_api_key_env.is_some()
        || args.context_guidance_gateway_url.is_some()
        || args.context_guidance_api_key_env.is_some()
    {
        return Ok(BitloopsInferenceSetupSelection::Cloud);
    }

    if !telemetry_consent::can_prompt_interactively() {
        bail!(NON_INTERACTIVE_INIT_BITLOOPS_INFERENCE_SELECTION_ERROR);
    }

    let cloud_logged_in = crate::daemon::resolve_workos_session_status()
        .await?
        .is_some();

    prompt_bitloops_inference_setup_selection(out, input, true, cloud_logged_in)
}
