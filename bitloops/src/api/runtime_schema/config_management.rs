mod errors;
mod identity;
mod patch;
mod persistence;
mod redaction;
mod sections;
mod snapshot;
mod targets;
mod types;
mod validation;

use std::fs;

use anyhow::anyhow;
use async_graphql::ID;
use toml_edit::DocumentMut;

use crate::api::DashboardState;
use crate::graphql::bad_user_input_error;

use errors::{internal_config_error, map_snapshot_error};
use identity::revision_for_bytes;
use patch::apply_patch_to_document;
use persistence::write_atomic;
use snapshot::build_snapshot;
use targets::{discover_config_targets, resolve_target};
pub(crate) use types::{
    RuntimeConfigSnapshotObject, RuntimeConfigTargetObject, UpdateRuntimeConfigInput,
    UpdateRuntimeConfigResult,
};
use validation::validate_target_text;

pub(crate) async fn list_config_targets(
    state: &DashboardState,
) -> async_graphql::Result<Vec<RuntimeConfigTargetObject>> {
    discover_config_targets(state)
        .await
        .map(|targets| targets.into_iter().map(Into::into).collect())
        .map_err(internal_config_error)
}

pub(crate) async fn load_config_snapshot(
    state: &DashboardState,
    target_id: &ID,
) -> async_graphql::Result<RuntimeConfigSnapshotObject> {
    let target = resolve_target(state, target_id).await?;
    build_snapshot(&target).map_err(map_snapshot_error)
}

pub(crate) async fn update_config(
    state: &DashboardState,
    input: UpdateRuntimeConfigInput,
) -> async_graphql::Result<UpdateRuntimeConfigResult> {
    let target = resolve_target(state, &input.target_id).await?;
    if !target.exists {
        return Err(bad_user_input_error(format!(
            "config target {} does not exist",
            target.path.display()
        )));
    }

    let original = fs::read_to_string(&target.path).map_err(|err| {
        internal_config_error(anyhow!(
            "failed to read config target {}: {err}",
            target.path.display()
        ))
    })?;
    let current_revision = revision_for_bytes(original.as_bytes());
    if current_revision != input.expected_revision {
        return Err(bad_user_input_error(format!(
            "config target changed on disk; reload {} before saving",
            target.path.display()
        )));
    }

    let mut doc = original.parse::<DocumentMut>().map_err(|err| {
        bad_user_input_error(format!(
            "failed to parse config target {}: {err}",
            target.path.display()
        ))
    })?;

    for patch in input.patches {
        apply_patch_to_document(&mut doc, patch)
            .map_err(|err| bad_user_input_error(format!("{err:#}")))?;
    }

    let next = doc.to_string();
    validate_target_text(&target, &next).map_err(|err| {
        bad_user_input_error(format!(
            "updated config is invalid for {}: {err:#}",
            target.path.display()
        ))
    })?;
    write_atomic(&target.path, next.as_bytes()).map_err(internal_config_error)?;

    let snapshot = build_snapshot(&target).map_err(map_snapshot_error)?;
    Ok(UpdateRuntimeConfigResult {
        restart_required: snapshot.restart_required,
        reload_required: snapshot.reload_required,
        path: target.path.display().to_string(),
        snapshot,
        message: "Configuration saved.".to_string(),
    })
}
