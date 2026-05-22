use async_graphql::types::Json;
use serde_json::json;
use tempfile::TempDir;

use super::models::{
    CapabilityPackFieldPatchInput, PlanCapabilityPackConfigInput, PlannerTargetKind,
};
use super::patches::build_combined_patches;
use super::selection::resolve_selection;

#[test]
fn resolve_selection_closes_context_guidance_dependencies() {
    let temp = TempDir::new().expect("temp dir");
    let daemon = json!({
        "runtime": {
            "capability_policy": {
                "explicit_enabled": ["context_guidance"],
                "explicit_disabled": []
            }
        }
    });

    let selection = resolve_selection(temp.path(), &daemon, &["context_guidance".to_string()], &[])
        .expect("selection");

    assert!(
        selection
            .states
            .get("context_guidance")
            .is_some_and(|state| state.enabled)
    );
    assert!(
        selection
            .states
            .get("knowledge")
            .is_some_and(|state| state.enabled)
    );
    assert!(
        selection
            .states
            .get("test_harness")
            .is_some_and(|state| state.enabled)
    );
}

#[test]
fn resolve_selection_blocks_disabling_required_dependency() {
    let temp = TempDir::new().expect("temp dir");
    let daemon = json!({
        "runtime": {
            "capability_policy": {
                "explicit_enabled": ["context_guidance"],
                "explicit_disabled": ["knowledge"]
            }
        }
    });

    let selection = resolve_selection(
        temp.path(),
        &daemon,
        &["context_guidance".to_string()],
        &["knowledge".to_string()],
    )
    .expect("selection");

    assert!(
        selection
            .blockers
            .iter()
            .any(|blocker| blocker.contains("explicitly disabled"))
    );
}

#[test]
fn build_combined_patches_writes_only_daemon_config() {
    let input = PlanCapabilityPackConfigInput {
        explicit_enabled: vec!["context_guidance".to_string()],
        explicit_disabled: Vec::new(),
        daemon_patches: Vec::new(),
    };
    let daemon = json!({});

    let patches = build_combined_patches(&input, &daemon).expect("patches");

    assert!(
        patches
            .iter()
            .all(|patch| patch.target == PlannerTargetKind::Daemon)
    );
    assert!(
        patches
            .iter()
            .any(|patch| patch.path == ["runtime", "capability_policy", "explicit_enabled"])
    );
    assert!(patches.iter().all(|patch| !matches!(
        patch.path.first().map(String::as_str),
        Some("agents" | "devql")
    )));
}

#[test]
fn build_combined_patches_rejects_repo_local_targets() {
    let input = PlanCapabilityPackConfigInput {
        explicit_enabled: Vec::new(),
        explicit_disabled: Vec::new(),
        daemon_patches: vec![CapabilityPackFieldPatchInput {
            target: "repo_local".to_string(),
            path: vec!["devql".to_string(), "sync_enabled".to_string()],
            value: Some(Json(json!(true))),
            unset: None,
        }],
    };

    let err = build_combined_patches(&input, &json!({})).expect_err("repo-local patch should fail");

    assert!(err.to_string().contains("daemon-scoped"));
}
