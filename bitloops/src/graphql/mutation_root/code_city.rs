use async_graphql::{Context, Result};

use crate::graphql::DevqlGraphqlContext;

use super::errors::operation_error;
use super::inputs::CodeCityRefreshInput;
use super::results::CodeCityRefreshResultObject;

pub(super) async fn refresh_code_city(
    ctx: &Context<'_>,
    input: CodeCityRefreshInput,
) -> Result<CodeCityRefreshResultObject> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "refreshCodeCity", err))?;
    let cfg = context
        .devql_config()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "refreshCodeCity", err))?;
    let project_path = normalise_codecity_project_path(input.project_path, "refreshCodeCity")?;
    let config = crate::capability_packs::codecity::services::config::CodeCityConfig::default();
    let config_fingerprint = config.fingerprint().map_err(|err| {
        operation_error(
            "BACKEND_ERROR",
            "configuration",
            "refreshCodeCity",
            format!("failed to fingerprint CodeCity configuration: {err:#}"),
        )
    })?;
    let snapshot_key =
        crate::capability_packs::codecity::storage::snapshot_key_for(project_path.as_deref());
    let repo =
        crate::capability_packs::codecity::storage::SqliteCodeCityRepository::open_for_repo_root(
            &cfg.repo_root,
        )
        .and_then(|repo| {
            repo.initialise_schema()?;
            Ok(repo)
        })
        .map_err(|err| operation_error("BACKEND_ERROR", "storage", "refreshCodeCity", err))?;
    let Some(latest_generation) =
        crate::daemon::capability_event_latest_generation(&cfg.repo.repo_id)
            .map_err(|err| operation_error("BACKEND_ERROR", "queue", "refreshCodeCity", err))?
    else {
        let status = crate::capability_packs::codecity::storage::missing_snapshot_status(
            &cfg.repo.repo_id,
            &snapshot_key,
            project_path.as_deref(),
            &config_fingerprint,
        );
        return Ok(CodeCityRefreshResultObject {
            success: false,
            queued: false,
            run_id: None,
            message: "Run DevQL sync first before refreshing Code Atlas.".to_string(),
            snapshot_status: status.into(),
        });
    };

    let mut status = repo
        .upsert_snapshot_request(
            &cfg.repo.repo_id,
            project_path.as_deref(),
            &config_fingerprint,
            Some(latest_generation),
        )
        .map_err(|err| operation_error("BACKEND_ERROR", "storage", "refreshCodeCity", err))?;
    let run = crate::daemon::force_current_state_consumer_run_for_config(
        &cfg,
        crate::capability_packs::codecity::types::CODECITY_CAPABILITY_ID,
        crate::capability_packs::codecity::types::CODECITY_SNAPSHOT_CONSUMER_ID,
    )
    .map_err(|err| operation_error("BACKEND_ERROR", "queue", "refreshCodeCity", err))?;
    status.run_id = Some(run.run_id.clone());
    status.state = match run.status {
        crate::daemon::CapabilityEventRunStatus::Running => {
            crate::capability_packs::codecity::types::CodeCitySnapshotState::Running
        }
        _ => crate::capability_packs::codecity::types::CodeCitySnapshotState::Queued,
    };

    Ok(CodeCityRefreshResultObject {
        success: true,
        queued: true,
        run_id: Some(run.run_id),
        message: "Code Atlas refresh queued.".to_string(),
        snapshot_status: status.into(),
    })
}

fn normalise_codecity_project_path(
    value: Option<String>,
    operation: &'static str,
) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            "`projectPath` must not be empty",
        ));
    }
    Ok(crate::capability_packs::codecity::storage::normalise_project_path(Some(trimmed)))
}
