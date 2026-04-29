use async_graphql::{Context, Result};

use crate::graphql::DevqlGraphqlContext;

use super::errors::operation_error;
use super::results::UpdateCliTelemetryConsentResult;
use super::validation::require_non_empty_input;

pub(super) async fn update_cli_telemetry_consent(
    ctx: &Context<'_>,
    cli_version: String,
    telemetry: Option<bool>,
) -> Result<UpdateCliTelemetryConsentResult> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context.require_global_write_scope().map_err(|err| {
        operation_error(
            "BAD_USER_INPUT",
            "validation",
            "updateCliTelemetryConsent",
            err,
        )
    })?;

    let cli_version =
        require_non_empty_input(cli_version, "cliVersion", "updateCliTelemetryConsent")?;
    let state = crate::config::update_daemon_telemetry_consent(
        Some(context.daemon_config_path().as_path()),
        &cli_version,
        telemetry,
    )
    .map_err(|err| {
        operation_error(
            "BACKEND_ERROR",
            "configuration",
            "updateCliTelemetryConsent",
            err,
        )
    })?;

    Ok(UpdateCliTelemetryConsentResult {
        telemetry: state.telemetry,
        needs_prompt: state.needs_prompt,
    })
}
