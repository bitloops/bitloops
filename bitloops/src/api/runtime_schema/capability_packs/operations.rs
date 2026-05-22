use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result as AnyhowResult;
use serde_json::Value;

use super::catalog::{pack_metadata, ui_spec_for_pack};
use super::models::{
    ApplyCapabilityPackConfigInput, ApplyCapabilityPackConfigResult, CapabilityPackCardObject,
    CapabilityPackConfigPlan, CapabilityPackObject, PackSelectionState,
    PlanCapabilityPackConfigInput,
};
use super::patches::{
    apply_patches_to_json, apply_patches_to_toml, build_combined_patches, hash_plan,
    load_daemon_file,
};
use super::review::build_review_groups;
use super::sections::build_pack_sections;
use super::selection::resolve_selection;
use crate::api::DashboardState;
use crate::config::validate_daemon_config_text;
use crate::graphql::{bad_user_input_error, graphql_error};

pub(crate) async fn capability_packs_catalog(
    state: &DashboardState,
) -> async_graphql::Result<Vec<CapabilityPackObject>> {
    let daemon = load_daemon_file(state.config_path.clone()).map_err(|err| {
        graphql_error("internal", format!("failed to load daemon config: {err:#}"))
    })?;
    let catalog = compute_catalog(&state.config_root, &daemon.value, &[]).map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to build capability pack catalog: {err:#}"),
        )
    })?;
    Ok(catalog)
}

pub(crate) async fn plan_capability_pack_config(
    state: &DashboardState,
    input: PlanCapabilityPackConfigInput,
) -> async_graphql::Result<CapabilityPackConfigPlan> {
    build_plan(state.config_path.clone(), &state.config_root, input).map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to build capability pack config plan: {err:#}"),
        )
    })
}

pub(crate) async fn apply_capability_pack_config(
    state: &DashboardState,
    input: ApplyCapabilityPackConfigInput,
) -> async_graphql::Result<ApplyCapabilityPackConfigResult> {
    let plan = build_plan(
        state.config_path.clone(),
        &state.config_root,
        input.plan_input.clone(),
    )
    .map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to rebuild capability pack config plan: {err:#}"),
        )
    })?;

    if plan.plan_hash != input.plan_hash {
        return Err(bad_user_input_error(
            "capability pack plan changed while editing; reload before saving".to_string(),
        ));
    }
    if plan.daemon_revision != input.expected_daemon_revision {
        return Err(bad_user_input_error(
            "daemon config changed on disk; reload before saving".to_string(),
        ));
    }
    if !plan.blockers.is_empty() {
        return Err(bad_user_input_error(format!(
            "cannot apply blocked capability config plan: {}",
            plan.blockers.join("; ")
        )));
    }

    let daemon_path = PathBuf::from(plan.daemon_config_path.clone());
    let daemon_target = load_daemon_file(daemon_path.clone()).map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to reload daemon config before save: {err:#}"),
        )
    })?;

    if daemon_target.revision.as_deref() != Some(input.expected_daemon_revision.as_str()) {
        return Err(bad_user_input_error(
            "daemon config changed on disk; reload before saving".to_string(),
        ));
    }

    let combined = build_combined_patches(&input.plan_input, &daemon_target.value)
        .map_err(|err| bad_user_input_error(format!("invalid daemon config patch: {err:#}")))?;
    let daemon_text = apply_patches_to_toml(&daemon_target.raw_text, &combined).map_err(|err| {
        bad_user_input_error(format!("failed to apply daemon config patch: {err:#}"))
    })?;

    validate_daemon_config_text(&daemon_text, &daemon_path).map_err(|err| {
        bad_user_input_error(format!("updated daemon config is invalid: {err:#}"))
    })?;

    if let Some(parent) = daemon_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            graphql_error(
                "internal",
                format!("failed to prepare daemon config directory: {err}"),
            )
        })?;
    }

    fs::write(&daemon_path, daemon_text.as_bytes()).map_err(|err| {
        graphql_error("internal", format!("failed to write daemon config: {err}"))
    })?;
    let restart_scheduled = if plan.restart_required {
        crate::daemon::schedule_delayed_daemon_restart(&daemon_path).map_err(|err| {
            graphql_error(
                "internal",
                format!("failed to schedule daemon restart: {err:#}"),
            )
        })?;
        true
    } else {
        false
    };

    Ok(ApplyCapabilityPackConfigResult {
        message: "Capability configuration saved.".to_string(),
        daemon_config_path: daemon_path.display().to_string(),
        restart_required: plan.restart_required,
        restart_scheduled,
        reload_required: plan.reload_required,
        plan_hash: plan.plan_hash,
    })
}

fn build_plan(
    daemon_path: PathBuf,
    metadata_root: &Path,
    input: PlanCapabilityPackConfigInput,
) -> AnyhowResult<CapabilityPackConfigPlan> {
    let daemon = load_daemon_file(daemon_path)?;
    let catalog = compute_catalog(metadata_root, &daemon.value, &input.explicit_enabled)?;
    let combined_patches = build_combined_patches(&input, &daemon.value)?;
    let daemon_preview = apply_patches_to_json(&daemon.value, &combined_patches)?;

    let selection = resolve_selection(
        metadata_root,
        &daemon_preview,
        &input.explicit_enabled,
        &input.explicit_disabled,
    )?;
    let shared_sections = Vec::new();
    let mut ownership = BTreeMap::<String, String>::new();
    let mut packs = Vec::new();
    let mut blockers = selection.blockers.clone();
    let mut warnings = Vec::new();

    for metadata in pack_metadata(metadata_root)? {
        let Some(ui_spec) = ui_spec_for_pack(metadata.id.as_str()) else {
            continue;
        };
        if !ui_spec.dashboard_configurable {
            continue;
        }
        let state = selection
            .states
            .get(metadata.id.as_str())
            .cloned()
            .unwrap_or(PackSelectionState {
                enabled: false,
                selection_source: "default".to_string(),
                dependency_reason: None,
            });
        if !state.enabled {
            continue;
        }
        let (sections, pack_warnings, ready) = build_pack_sections(
            metadata_root,
            metadata.id.as_str(),
            &daemon.value,
            &daemon_preview,
            &mut ownership,
        )?;
        warnings.extend(pack_warnings.iter().cloned());
        if !ready {
            blockers.push(format!("{} still needs setup", metadata.display_name));
        }
        packs.push(CapabilityPackCardObject {
            id: metadata.id.clone(),
            display_name: metadata.display_name.clone(),
            description: metadata.description.clone(),
            selection_source: state.selection_source,
            enabled: true,
            readiness: if ready { "ready" } else { "needs_setup" }.to_string(),
            dependency_reason: state.dependency_reason,
            warnings: pack_warnings,
            sections,
        });
    }

    packs.sort_by(|left, right| {
        let left_order = ui_spec_for_pack(left.id.as_str())
            .map(|spec| spec.display_order)
            .unwrap_or(i32::MAX);
        let right_order = ui_spec_for_pack(right.id.as_str())
            .map(|spec| spec.display_order)
            .unwrap_or(i32::MAX);
        left_order
            .cmp(&right_order)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    let review_groups = build_review_groups(&daemon.value, &daemon_preview);
    let plan_hash = hash_plan(
        &input,
        daemon.revision.as_deref().unwrap_or(""),
        &daemon_preview,
    )?;

    Ok(CapabilityPackConfigPlan {
        plan_hash,
        daemon_config_path: daemon.path.display().to_string(),
        daemon_revision: daemon.revision.unwrap_or_default(),
        warnings,
        blockers,
        catalog,
        shared_sections,
        packs,
        review_groups,
        restart_required: true,
        reload_required: true,
    })
}

fn compute_catalog(
    repo_root: &Path,
    daemon_value: &Value,
    explicit_enabled_override: &[String],
) -> AnyhowResult<Vec<CapabilityPackObject>> {
    let selection = resolve_selection(repo_root, daemon_value, explicit_enabled_override, &[])?;
    let mut items = Vec::new();
    for metadata in pack_metadata(repo_root)? {
        let Some(ui_spec) = ui_spec_for_pack(metadata.id.as_str()) else {
            continue;
        };
        let state = selection
            .states
            .get(metadata.id.as_str())
            .cloned()
            .unwrap_or(PackSelectionState {
                enabled: false,
                selection_source: "default".to_string(),
                dependency_reason: None,
            });
        let (_, pack_warnings, ready) = if ui_spec.dashboard_configurable && state.enabled {
            let mut ownership = BTreeMap::new();
            build_pack_sections(
                repo_root,
                metadata.id.as_str(),
                daemon_value,
                daemon_value,
                &mut ownership,
            )?
        } else {
            (Vec::new(), Vec::new(), false)
        };
        items.push(CapabilityPackObject {
            id: metadata.id.clone(),
            display_name: metadata.display_name.clone(),
            description: metadata.description.clone(),
            version: metadata.version.clone(),
            experimental: metadata.experimental,
            default_enabled: metadata.default_enabled,
            dashboard_configurable: ui_spec.dashboard_configurable,
            display_order: ui_spec.display_order,
            selection_source: state.selection_source,
            enabled: state.enabled,
            readiness: if state.enabled {
                if ready { "ready" } else { "needs_setup" }
            } else {
                "needs_setup"
            }
            .to_string(),
            dependencies: metadata.dependencies.clone(),
            inference_slots: metadata.inference_slots.clone(),
            warnings: pack_warnings,
        });
    }
    items.sort_by(|left, right| {
        left.display_order
            .cmp(&right.display_order)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    Ok(items)
}
