use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result as AnyhowResult, anyhow, bail};
use async_graphql::{InputObject, SimpleObject, types::Json};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, de::from_str};

use crate::api::DashboardState;
use crate::capability_packs::builtin_packs;
use crate::config::validate_daemon_config_text;
use crate::graphql::{bad_user_input_error, graphql_error};
use crate::host::inference::BITLOOPS_INFERENCE_RUNTIME_ID;

type ConfigJsonScalar = Json<Value>;

const STRUCTURED_GENERATION_TASK: &str = "structured_generation";
const TEXT_GENERATION_TASK: &str = "text_generation";
const EMBEDDINGS_TASK: &str = "embeddings";

const ARCHITECTURE_ROLE_ADJUDICATION_SLOT: &str = "role_adjudication";
const ARCHITECTURE_FACT_SYNTHESIS_SLOT: &str = "fact_synthesis";
const CONTEXT_GUIDANCE_GENERATION_SLOT: &str = "guidance_generation";
const SEMANTIC_CLONES_SUMMARY_SLOT: &str = "summary_generation";
const SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT: &str = "code_embeddings";
const SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT: &str = "summary_embeddings";

const DRIVER_OPTIONS_TEXT_GENERATION: &[&str] = &[
    "bitloops_platform_chat",
    "openai_chat_completions",
    "ollama_chat",
];
const DRIVER_OPTIONS_STRUCTURED_GENERATION: &[&str] = &["codex_exec", "claude_code_print"];
const DRIVER_OPTIONS_EMBEDDINGS: &[&str] = &["bitloops_embeddings_ipc"];
const THINKING_LEVEL_OPTIONS: &[&str] = &["low", "medium", "high"];
const ARCHITECTURE_RUNTIME_OPTIONS: &[&str] = &["codex", "claude"];
const TEST_HARNESS_COVERAGE_FORMAT_OPTIONS: &[&str] = &["lcov"];
const SEMANTIC_SUMMARY_MODE_OPTIONS: &[&str] = &["auto", "off"];
const SEMANTIC_EMBEDDING_MODE_OPTIONS: &[&str] = &[
    "off",
    "deterministic",
    "semantic_aware_once",
    "refresh_on_upgrade",
];

#[derive(Debug, Clone, InputObject)]
pub(crate) struct CapabilityPackFieldPatchInput {
    pub(crate) target: String,
    pub(crate) path: Vec<String>,
    #[graphql(default)]
    pub(crate) value: Option<ConfigJsonScalar>,
    #[graphql(default)]
    pub(crate) unset: Option<bool>,
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct PlanCapabilityPackConfigInput {
    #[graphql(name = "explicitEnabled", default)]
    pub(crate) explicit_enabled: Vec<String>,
    #[graphql(name = "explicitDisabled", default)]
    pub(crate) explicit_disabled: Vec<String>,
    #[graphql(name = "daemonPatches", default)]
    pub(crate) daemon_patches: Vec<CapabilityPackFieldPatchInput>,
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct ApplyCapabilityPackConfigInput {
    #[graphql(flatten)]
    pub(crate) plan_input: PlanCapabilityPackConfigInput,
    #[graphql(name = "expectedDaemonRevision")]
    pub(crate) expected_daemon_revision: String,
    #[graphql(name = "planHash")]
    pub(crate) plan_hash: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackDependencyObject {
    #[graphql(name = "packId")]
    pub(crate) pack_id: String,
    #[graphql(name = "minVersion")]
    pub(crate) min_version: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackInferenceSlotObject {
    pub(crate) name: String,
    pub(crate) task: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackObject {
    pub(crate) id: String,
    #[graphql(name = "displayName")]
    pub(crate) display_name: String,
    pub(crate) description: String,
    pub(crate) version: String,
    pub(crate) experimental: bool,
    #[graphql(name = "defaultEnabled")]
    pub(crate) default_enabled: bool,
    #[graphql(name = "dashboardConfigurable")]
    pub(crate) dashboard_configurable: bool,
    #[graphql(name = "displayOrder")]
    pub(crate) display_order: i32,
    #[graphql(name = "selectionSource")]
    pub(crate) selection_source: String,
    pub(crate) enabled: bool,
    pub(crate) readiness: String,
    pub(crate) dependencies: Vec<CapabilityPackDependencyObject>,
    #[graphql(name = "inferenceSlots")]
    pub(crate) inference_slots: Vec<CapabilityPackInferenceSlotObject>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackFieldObject {
    pub(crate) key: String,
    #[graphql(name = "targetKind")]
    pub(crate) target_kind: String,
    #[graphql(name = "configPath")]
    pub(crate) config_path: Vec<String>,
    pub(crate) label: String,
    pub(crate) description: String,
    #[graphql(name = "fieldType")]
    pub(crate) field_type: String,
    #[graphql(name = "currentValue")]
    pub(crate) current_value: ConfigJsonScalar,
    #[graphql(name = "proposedValue")]
    pub(crate) proposed_value: ConfigJsonScalar,
    #[graphql(name = "allowedValues")]
    pub(crate) allowed_values: Vec<String>,
    pub(crate) required: bool,
    pub(crate) secret: bool,
    pub(crate) editable: bool,
    #[graphql(name = "sharedOwner")]
    pub(crate) shared_owner: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackSectionObject {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) fields: Vec<CapabilityPackFieldObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackCardObject {
    pub(crate) id: String,
    #[graphql(name = "displayName")]
    pub(crate) display_name: String,
    pub(crate) description: String,
    #[graphql(name = "selectionSource")]
    pub(crate) selection_source: String,
    pub(crate) enabled: bool,
    pub(crate) readiness: String,
    #[graphql(name = "dependencyReason")]
    pub(crate) dependency_reason: Option<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) sections: Vec<CapabilityPackSectionObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackReviewGroupObject {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) items: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct CapabilityPackConfigPlan {
    #[graphql(name = "planHash")]
    pub(crate) plan_hash: String,
    #[graphql(name = "daemonConfigPath")]
    pub(crate) daemon_config_path: String,
    #[graphql(name = "daemonRevision")]
    pub(crate) daemon_revision: String,
    pub(crate) warnings: Vec<String>,
    pub(crate) blockers: Vec<String>,
    pub(crate) catalog: Vec<CapabilityPackObject>,
    #[graphql(name = "sharedSections")]
    pub(crate) shared_sections: Vec<CapabilityPackSectionObject>,
    pub(crate) packs: Vec<CapabilityPackCardObject>,
    #[graphql(name = "reviewGroups")]
    pub(crate) review_groups: Vec<CapabilityPackReviewGroupObject>,
    #[graphql(name = "restartRequired")]
    pub(crate) restart_required: bool,
    #[graphql(name = "reloadRequired")]
    pub(crate) reload_required: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct ApplyCapabilityPackConfigResult {
    pub(crate) message: String,
    #[graphql(name = "daemonConfigPath")]
    pub(crate) daemon_config_path: String,
    #[graphql(name = "restartRequired")]
    pub(crate) restart_required: bool,
    #[graphql(name = "restartScheduled")]
    pub(crate) restart_scheduled: bool,
    #[graphql(name = "reloadRequired")]
    pub(crate) reload_required: bool,
    #[graphql(name = "planHash")]
    pub(crate) plan_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannerTargetKind {
    Daemon,
}

impl PlannerTargetKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
        }
    }
}

#[derive(Debug, Clone)]
struct PackMetadata {
    id: String,
    display_name: String,
    description: String,
    version: String,
    default_enabled: bool,
    experimental: bool,
    dependencies: Vec<CapabilityPackDependencyObject>,
    inference_slots: Vec<CapabilityPackInferenceSlotObject>,
}

#[derive(Debug, Clone, Copy)]
struct PackUiSpec {
    pack_id: &'static str,
    dashboard_configurable: bool,
    display_order: i32,
}

const PACK_UI_SPECS: &[PackUiSpec] = &[
    PackUiSpec {
        pack_id: "architecture_graph",
        dashboard_configurable: true,
        display_order: 10,
    },
    PackUiSpec {
        pack_id: "context_guidance",
        dashboard_configurable: true,
        display_order: 20,
    },
    PackUiSpec {
        pack_id: "semantic_clones",
        dashboard_configurable: true,
        display_order: 30,
    },
    PackUiSpec {
        pack_id: "knowledge",
        dashboard_configurable: true,
        display_order: 40,
    },
    PackUiSpec {
        pack_id: "test_harness",
        dashboard_configurable: true,
        display_order: 50,
    },
    PackUiSpec {
        pack_id: "codecity",
        dashboard_configurable: false,
        display_order: 90,
    },
    PackUiSpec {
        pack_id: "http",
        dashboard_configurable: false,
        display_order: 100,
    },
    PackUiSpec {
        pack_id: "navigation_context",
        dashboard_configurable: false,
        display_order: 110,
    },
];

#[derive(Debug, Clone)]
struct LoadedConfigFile {
    path: PathBuf,
    raw_text: String,
    value: Value,
    revision: Option<String>,
}

#[derive(Debug, Clone)]
struct DraftPatch {
    target: PlannerTargetKind,
    path: Vec<String>,
    value: Option<Value>,
    unset: bool,
}

#[derive(Debug, Clone)]
struct PackSelectionState {
    enabled: bool,
    selection_source: String,
    dependency_reason: Option<String>,
}

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
        schedule_delayed_daemon_restart(&daemon_path).map_err(|err| {
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

fn resolve_selection(
    repo_root: &Path,
    daemon_value: &Value,
    explicit_enabled_override: &[String],
    explicit_disabled_override: &[String],
) -> AnyhowResult<ResolvedSelection> {
    let metadata = pack_metadata(repo_root)?;
    let by_id = metadata
        .iter()
        .map(|item| (item.id.clone(), item.clone()))
        .collect::<BTreeMap<_, _>>();

    let policy_enabled = if explicit_enabled_override.is_empty() {
        read_string_list(
            daemon_value,
            &["runtime", "capability_policy", "explicit_enabled"],
        )
    } else {
        explicit_enabled_override
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    };
    let policy_disabled = if explicit_disabled_override.is_empty() {
        read_string_list(
            daemon_value,
            &["runtime", "capability_policy", "explicit_disabled"],
        )
    } else {
        explicit_disabled_override
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    };

    let explicit_enabled = if policy_enabled.is_empty() && policy_disabled.is_empty() {
        metadata
            .iter()
            .filter(|item| item.default_enabled)
            .map(|item| item.id.clone())
            .collect::<BTreeSet<_>>()
    } else {
        policy_enabled.into_iter().collect::<BTreeSet<_>>()
    };
    let explicit_disabled = policy_disabled.into_iter().collect::<BTreeSet<_>>();

    let mut states = BTreeMap::new();
    let mut blockers = Vec::new();
    let mut visited = BTreeSet::new();
    let mut active_stack = Vec::<String>::new();

    struct SelectionVisitContext<'a> {
        by_id: &'a BTreeMap<String, PackMetadata>,
        explicit_disabled: &'a BTreeSet<String>,
        states: &'a mut BTreeMap<String, PackSelectionState>,
        visited: &'a mut BTreeSet<String>,
        active_stack: &'a mut Vec<String>,
        blockers: &'a mut Vec<String>,
    }

    fn visit(id: &str, ctx: &mut SelectionVisitContext<'_>, source: &str, reason: Option<String>) {
        if ctx.visited.contains(id) {
            if let Some(existing) = ctx.states.get_mut(id)
                && existing.selection_source != "explicit"
                && source == "explicit"
            {
                existing.selection_source = "explicit".to_string();
                existing.dependency_reason = None;
            }
            return;
        }
        if ctx.active_stack.iter().any(|entry| entry == id) {
            let mut chain = ctx.active_stack.clone();
            chain.push(id.to_string());
            ctx.blockers
                .push(format!("dependency cycle detected: {}", chain.join(" -> ")));
            return;
        }
        let Some(metadata) = ctx.by_id.get(id) else {
            ctx.blockers.push(format!("missing dependency `{id}`"));
            return;
        };
        if ctx.explicit_disabled.contains(id) && source != "explicit" {
            ctx.blockers.push(format!(
                "dependency `{id}` is explicitly disabled but required by another pack"
            ));
            return;
        }
        let display_name = metadata.display_name.clone();
        let dependencies = metadata.dependencies.clone();
        ctx.active_stack.push(id.to_string());
        for dependency in &dependencies {
            visit(
                dependency.pack_id.as_str(),
                ctx,
                "dependency",
                Some(format!("required by {display_name}")),
            );
        }
        ctx.active_stack.pop();
        ctx.visited.insert(id.to_string());
        ctx.states.insert(
            id.to_string(),
            PackSelectionState {
                enabled: true,
                selection_source: source.to_string(),
                dependency_reason: reason,
            },
        );
    }

    {
        let mut visit_ctx = SelectionVisitContext {
            by_id: &by_id,
            explicit_disabled: &explicit_disabled,
            states: &mut states,
            visited: &mut visited,
            active_stack: &mut active_stack,
            blockers: &mut blockers,
        };
        for id in &explicit_enabled {
            visit(id, &mut visit_ctx, "explicit", None);
        }
    }

    for id in by_id.keys() {
        states.entry(id.clone()).or_insert(PackSelectionState {
            enabled: false,
            selection_source: "default".to_string(),
            dependency_reason: None,
        });
    }

    Ok(ResolvedSelection { states, blockers })
}

#[derive(Debug, Clone)]
struct ResolvedSelection {
    states: BTreeMap<String, PackSelectionState>,
    blockers: Vec<String>,
}

fn build_pack_sections(
    _repo_root: &Path,
    pack_id: &str,
    current: &Value,
    proposed: &Value,
    ownership: &mut BTreeMap<String, String>,
) -> AnyhowResult<(Vec<CapabilityPackSectionObject>, Vec<String>, bool)> {
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let mut ready = true;
    match pack_id {
        "architecture_graph" => {
            let (items, item_warnings, item_ready) =
                build_architecture_sections(current, proposed, ownership);
            sections = items;
            warnings = item_warnings;
            ready = item_ready;
        }
        "context_guidance" => {
            let (items, item_warnings, item_ready) =
                build_context_guidance_sections(current, proposed, ownership);
            sections = items;
            warnings = item_warnings;
            ready = item_ready;
        }
        "semantic_clones" => {
            let (items, item_warnings, item_ready) =
                build_semantic_clone_sections(current, proposed, ownership);
            sections = items;
            warnings = item_warnings;
            ready = item_ready;
        }
        "knowledge" => {
            sections = build_knowledge_sections(current, proposed);
            ready = has_non_empty_value(proposed, &["knowledge", "providers", "github", "token"])
                || has_non_empty_value(proposed, &["knowledge", "providers", "atlassian", "token"]);
            if !ready {
                warnings.push("Knowledge needs at least one configured provider.".to_string());
            }
        }
        "test_harness" => {
            sections = build_test_harness_sections(current, proposed);
            ready = has_truthy_value(
                proposed,
                &["test_harness", "dependencies", "coverage_adapter"],
            ) && has_truthy_value(
                proposed,
                &["test_harness", "dependencies", "test_discovery_adapter"],
            ) && has_truthy_value(
                proposed,
                &["test_harness", "dependencies", "language_support"],
            ) && has_non_empty_value(proposed, &["test_harness", "coverage", "format"]);
            if !ready {
                warnings.push(
                    "Test Harness needs dependency toggles plus coverage format before it is ready."
                        .to_string(),
                );
            }
        }
        other => {
            warnings.push(format!("No guided setup is defined for `{other}` yet."));
        }
    }
    Ok((sections, warnings, ready))
}

fn build_architecture_sections(
    current: &Value,
    proposed: &Value,
    ownership: &mut BTreeMap<String, String>,
) -> (Vec<CapabilityPackSectionObject>, Vec<String>, bool) {
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let mut ready = true;
    sections.push(CapabilityPackSectionObject {
        key: "architecture-bindings".to_string(),
        title: "Architecture bindings".to_string(),
        description: "Structured-generation bindings owned by the Architecture pack.".to_string(),
        fields: vec![
            config_field(
                PlannerTargetKind::Daemon,
                &[
                    "architecture",
                    "inference",
                    ARCHITECTURE_FACT_SYNTHESIS_SLOT,
                ],
                "Fact synthesis",
                "Profile binding for architecture fact synthesis.",
                "string",
                current,
                proposed,
                profile_options_for_task(proposed, STRUCTURED_GENERATION_TASK),
                false,
                false,
                true,
                None,
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &[
                    "architecture",
                    "inference",
                    ARCHITECTURE_ROLE_ADJUDICATION_SLOT,
                ],
                "Role adjudication",
                "Profile binding for architecture role adjudication.",
                "string",
                current,
                proposed,
                profile_options_for_task(proposed, STRUCTURED_GENERATION_TASK),
                false,
                false,
                true,
                None,
            ),
        ],
    });
    for (slot_key, title) in [
        (ARCHITECTURE_FACT_SYNTHESIS_SLOT, "Fact synthesis profile"),
        (
            ARCHITECTURE_ROLE_ADJUDICATION_SLOT,
            "Role adjudication profile",
        ),
    ] {
        let profile_name = string_value_at_path(proposed, &["architecture", "inference", slot_key])
            .unwrap_or_else(|| format!("architecture_{slot_key}_managed"));
        let (profile_sections, runtime_warning, profile_ready) = build_inference_profile_sections(
            current,
            proposed,
            ownership,
            &profile_name,
            title,
            STRUCTURED_GENERATION_TASK,
            slot_key,
            DRIVER_OPTIONS_STRUCTURED_GENERATION,
            Some(ARCHITECTURE_RUNTIME_OPTIONS),
            true,
        );
        sections.extend(profile_sections);
        if let Some(runtime_warning) = runtime_warning {
            warnings.push(runtime_warning);
        }
        ready &= profile_ready
            && has_non_empty_value(proposed, &["architecture", "inference", slot_key]);
    }
    (sections, warnings, ready)
}

fn build_context_guidance_sections(
    current: &Value,
    proposed: &Value,
    ownership: &mut BTreeMap<String, String>,
) -> (Vec<CapabilityPackSectionObject>, Vec<String>, bool) {
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let profile_name = string_value_at_path(
        proposed,
        &[
            "context_guidance",
            "inference",
            CONTEXT_GUIDANCE_GENERATION_SLOT,
        ],
    )
    .unwrap_or_else(|| "guidance_llm".to_string());
    sections.push(CapabilityPackSectionObject {
        key: "context-guidance-binding".to_string(),
        title: "Guidance binding".to_string(),
        description: "Text-generation binding for Context Guidance.".to_string(),
        fields: vec![config_field(
            PlannerTargetKind::Daemon,
            &[
                "context_guidance",
                "inference",
                CONTEXT_GUIDANCE_GENERATION_SLOT,
            ],
            "Guidance generation",
            "Profile binding for Context Guidance generation.",
            "string",
            current,
            proposed,
            profile_options_for_task(proposed, TEXT_GENERATION_TASK),
            false,
            false,
            true,
            None,
        )],
    });
    let (profile_sections, runtime_warning, ready) = build_inference_profile_sections(
        current,
        proposed,
        ownership,
        &profile_name,
        "Guidance generation profile",
        TEXT_GENERATION_TASK,
        CONTEXT_GUIDANCE_GENERATION_SLOT,
        DRIVER_OPTIONS_TEXT_GENERATION,
        None,
        false,
    );
    sections.extend(profile_sections);
    if let Some(runtime_warning) = runtime_warning {
        warnings.push(runtime_warning);
    }
    (
        sections,
        warnings,
        ready
            && has_non_empty_value(
                proposed,
                &[
                    "context_guidance",
                    "inference",
                    CONTEXT_GUIDANCE_GENERATION_SLOT,
                ],
            ),
    )
}

fn build_semantic_clone_sections(
    current: &Value,
    proposed: &Value,
    ownership: &mut BTreeMap<String, String>,
) -> (Vec<CapabilityPackSectionObject>, Vec<String>, bool) {
    let mut sections = vec![CapabilityPackSectionObject {
        key: "semantic-clones-direct".to_string(),
        title: "Semantic clone settings".to_string(),
        description: "Direct semantic-clone configuration.".to_string(),
        fields: vec![
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "summary_mode"],
                "Summary mode",
                "Controls semantic summary generation.",
                "enum",
                current,
                proposed,
                SEMANTIC_SUMMARY_MODE_OPTIONS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                false,
                false,
                true,
                Some(json!("auto")),
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "embedding_mode"],
                "Embedding mode",
                "Controls semantic embedding refresh behaviour.",
                "enum",
                current,
                proposed,
                SEMANTIC_EMBEDDING_MODE_OPTIONS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                false,
                false,
                true,
                Some(json!("semantic_aware_once")),
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "ann_neighbors"],
                "ANN neighbors",
                "Nearest-neighbor count for semantic-clone lookup.",
                "integer",
                current,
                proposed,
                Vec::new(),
                false,
                false,
                true,
                Some(json!(5)),
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "summary_workers"],
                "Summary workers",
                "Concurrent summary workers.",
                "integer",
                current,
                proposed,
                Vec::new(),
                false,
                false,
                true,
                Some(json!(1)),
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "embedding_workers"],
                "Embedding workers",
                "Concurrent embedding workers.",
                "integer",
                current,
                proposed,
                Vec::new(),
                false,
                false,
                true,
                Some(json!(1)),
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "clone_rebuild_workers"],
                "Clone rebuild workers",
                "Concurrent clone rebuild workers.",
                "integer",
                current,
                proposed,
                Vec::new(),
                false,
                false,
                true,
                Some(json!(1)),
            ),
            config_field(
                PlannerTargetKind::Daemon,
                &["semantic_clones", "enrichment_workers"],
                "Enrichment workers",
                "Concurrent enrichment workers.",
                "integer",
                current,
                proposed,
                Vec::new(),
                false,
                false,
                true,
                Some(json!(1)),
            ),
        ],
    }];
    let mut warnings = Vec::new();
    let mut ready = true;
    for (slot_key, title, task, drivers) in [
        (
            SEMANTIC_CLONES_SUMMARY_SLOT,
            "Summary generation profile",
            TEXT_GENERATION_TASK,
            DRIVER_OPTIONS_TEXT_GENERATION,
        ),
        (
            SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT,
            "Code embeddings profile",
            EMBEDDINGS_TASK,
            DRIVER_OPTIONS_EMBEDDINGS,
        ),
        (
            SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
            "Summary embeddings profile",
            EMBEDDINGS_TASK,
            DRIVER_OPTIONS_EMBEDDINGS,
        ),
    ] {
        let path = ["semantic_clones", "inference", slot_key];
        sections.push(CapabilityPackSectionObject {
            key: format!("semantic-clones-binding-{slot_key}"),
            title: format!("{title} binding"),
            description: "Profile binding owned by Semantic Clones.".to_string(),
            fields: vec![config_field(
                PlannerTargetKind::Daemon,
                &path,
                title,
                "Profile binding for Semantic Clones inference.",
                "string",
                current,
                proposed,
                profile_options_for_task(proposed, task),
                false,
                false,
                true,
                None,
            )],
        });
        let profile_name = string_value_at_path(proposed, &path)
            .unwrap_or_else(|| format!("semantic_clones_{slot_key}_managed"));
        let (profile_sections, runtime_warning, section_ready) = build_inference_profile_sections(
            current,
            proposed,
            ownership,
            &profile_name,
            title,
            task,
            slot_key,
            drivers,
            None,
            false,
        );
        sections.extend(profile_sections);
        if let Some(runtime_warning) = runtime_warning {
            warnings.push(runtime_warning);
        }
        ready &= section_ready && has_non_empty_value(proposed, &path);
    }
    (sections, warnings, ready)
}

fn build_knowledge_sections(current: &Value, proposed: &Value) -> Vec<CapabilityPackSectionObject> {
    vec![
        CapabilityPackSectionObject {
            key: "knowledge-github".to_string(),
            title: "GitHub provider".to_string(),
            description: "GitHub knowledge provider settings.".to_string(),
            fields: vec![config_field(
                PlannerTargetKind::Daemon,
                &["knowledge", "providers", "github", "token"],
                "Token",
                "GitHub token or environment placeholder.",
                "string",
                current,
                proposed,
                Vec::new(),
                true,
                true,
                true,
                None,
            )],
        },
        CapabilityPackSectionObject {
            key: "knowledge-atlassian".to_string(),
            title: "Atlassian provider".to_string(),
            description: "Atlassian knowledge provider settings.".to_string(),
            fields: vec![
                config_field(
                    PlannerTargetKind::Daemon,
                    &["knowledge", "providers", "atlassian", "site_url"],
                    "Site URL",
                    "Atlassian cloud site URL.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    true,
                    false,
                    true,
                    None,
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["knowledge", "providers", "atlassian", "email"],
                    "Email",
                    "Atlassian user email.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    true,
                    false,
                    true,
                    None,
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["knowledge", "providers", "atlassian", "token"],
                    "Token",
                    "Atlassian token or environment placeholder.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    true,
                    true,
                    true,
                    None,
                ),
            ],
        },
    ]
}

fn build_test_harness_sections(
    current: &Value,
    proposed: &Value,
) -> Vec<CapabilityPackSectionObject> {
    vec![
        CapabilityPackSectionObject {
            key: "test-harness-adapters".to_string(),
            title: "Adapters".to_string(),
            description: "Test Harness adapter settings.".to_string(),
            fields: vec![
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "coverage_adapter"],
                    "Coverage adapter",
                    "Coverage adapter identifier.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    None,
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "test_discovery_adapter"],
                    "Test discovery adapter",
                    "Test-discovery adapter identifier.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    None,
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "language_support"],
                    "Language support",
                    "Language-support provider identifier.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    None,
                ),
            ],
        },
        CapabilityPackSectionObject {
            key: "test-harness-dependencies".to_string(),
            title: "Dependencies".to_string(),
            description: "Dependency-gate toggles for the Test Harness scaffold.".to_string(),
            fields: vec![
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "dependencies", "coverage_adapter"],
                    "Coverage adapter dependency",
                    "Enable the coverage-adapter dependency hook.",
                    "boolean",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    Some(json!(false)),
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "dependencies", "test_discovery_adapter"],
                    "Test discovery dependency",
                    "Enable the test-discovery dependency hook.",
                    "boolean",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    Some(json!(false)),
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "dependencies", "language_support"],
                    "Language support dependency",
                    "Enable the language-support dependency hook.",
                    "boolean",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    Some(json!(false)),
                ),
            ],
        },
        CapabilityPackSectionObject {
            key: "test-harness-coverage".to_string(),
            title: "Coverage".to_string(),
            description: "Coverage ingestion settings.".to_string(),
            fields: vec![
                config_field(
                    PlannerTargetKind::Daemon,
                    &["test_harness", "coverage", "format"],
                    "Coverage format",
                    "Coverage report format supported by the current scaffold.",
                    "enum",
                    current,
                    proposed,
                    TEST_HARNESS_COVERAGE_FORMAT_OPTIONS
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    true,
                    false,
                    true,
                    Some(json!("lcov")),
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["stores", "relational", "sqlite_path"],
                    "SQLite path",
                    "Related relational-store path used by Test Harness.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    true,
                    None,
                ),
            ],
        },
    ]
}

#[allow(clippy::too_many_arguments)]
fn build_inference_profile_sections(
    current: &Value,
    proposed: &Value,
    ownership: &mut BTreeMap<String, String>,
    profile_name: &str,
    title: &str,
    task: &str,
    slot_key: &str,
    driver_options: &[&str],
    runtime_options: Option<&[&str]>,
    include_thinking_level: bool,
) -> (Vec<CapabilityPackSectionObject>, Option<String>, bool) {
    let profile_owner_key = format!("profile:{profile_name}");
    let existing_owner = ownership.get(&profile_owner_key).cloned().or_else(|| {
        ownership.insert(profile_owner_key.clone(), title.to_string());
        None
    });
    let editable = existing_owner.is_none();
    let profile_fields = vec![
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "task"],
            "Task",
            "Inference task type for this profile.",
            "enum",
            current,
            proposed,
            vec![task.to_string()],
            true,
            false,
            editable,
            Some(json!(task)),
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "driver"],
            "Driver",
            "Driver implementation for this profile.",
            "enum",
            current,
            proposed,
            driver_options
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            true,
            false,
            editable,
            None,
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "runtime"],
            "Runtime",
            "Referenced runtime id for this profile.",
            "enum",
            current,
            proposed,
            runtime_options
                .map(|items| items.iter().map(|value| (*value).to_string()).collect())
                .unwrap_or_else(|| runtime_options_from_value(proposed)),
            matches!(task, STRUCTURED_GENERATION_TASK | TEXT_GENERATION_TASK),
            false,
            editable,
            None,
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "model"],
            "Model",
            "Model identifier for this profile.",
            "string",
            current,
            proposed,
            Vec::new(),
            false,
            false,
            editable,
            None,
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "api_key"],
            "API key",
            "API key or environment placeholder.",
            "string",
            current,
            proposed,
            Vec::new(),
            false,
            true,
            editable,
            None,
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "base_url"],
            "Base URL",
            "Gateway or provider URL override.",
            "string",
            current,
            proposed,
            Vec::new(),
            false,
            false,
            editable,
            None,
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "temperature"],
            "Temperature",
            "Sampling temperature for this profile.",
            "string",
            current,
            proposed,
            Vec::new(),
            false,
            false,
            editable,
            None,
        ),
        config_field(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", profile_name, "max_output_tokens"],
            "Max output tokens",
            "Maximum output tokens for this profile.",
            "integer",
            current,
            proposed,
            Vec::new(),
            false,
            false,
            editable,
            None,
        ),
    ];
    let mut sections = vec![CapabilityPackSectionObject {
        key: format!("profile-{slot_key}"),
        title: title.to_string(),
        description: format!("Config for profile `{profile_name}`."),
        fields: if include_thinking_level {
            let mut fields = profile_fields;
            fields.push(config_field(
                PlannerTargetKind::Daemon,
                &["inference", "profiles", profile_name, "thinking_level"],
                "Thinking level",
                "Structured-generation reasoning depth.",
                "enum",
                current,
                proposed,
                THINKING_LEVEL_OPTIONS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                false,
                false,
                editable,
                None,
            ));
            fields
        } else {
            profile_fields
        },
    }];
    let runtime_name = string_value_at_path(
        proposed,
        &["inference", "profiles", profile_name, "runtime"],
    );
    let mut runtime_warning = None;
    let ready = has_non_empty_value(proposed, &["inference", "profiles", profile_name, "driver"])
        && has_non_empty_value(
            proposed,
            &["inference", "profiles", profile_name, "runtime"],
        )
        && has_non_empty_value(proposed, &["inference", "profiles", profile_name, "model"])
        && has_non_empty_value(
            proposed,
            &["inference", "profiles", profile_name, "temperature"],
        )
        && has_non_empty_value(
            proposed,
            &["inference", "profiles", profile_name, "max_output_tokens"],
        );
    if let Some(runtime_name) = runtime_name.as_deref() {
        let runtime_owner_key = format!("runtime:{runtime_name}");
        let runtime_owner = ownership.get(&runtime_owner_key).cloned().or_else(|| {
            ownership.insert(runtime_owner_key.clone(), title.to_string());
            None
        });
        let runtime_editable = runtime_owner.is_none();
        let runtime_section = build_runtime_section(
            current,
            proposed,
            runtime_name,
            runtime_editable,
            runtime_owner.clone(),
        );
        if runtime_section.1.is_some() {
            runtime_warning = runtime_section.1;
        }
        sections.push(runtime_section.0);
    }
    (sections, runtime_warning, ready)
}

fn build_runtime_section(
    current: &Value,
    proposed: &Value,
    runtime_name: &str,
    editable: bool,
    shared_owner: Option<String>,
) -> (CapabilityPackSectionObject, Option<String>) {
    let warning = if matches!(runtime_name, "codex" | "claude")
        && !has_non_empty_value(
            proposed,
            &["inference", "runtimes", runtime_name, "command"],
        ) {
        Some(format!(
            "No executable is configured for `{runtime_name}` yet. Save + Run will need a valid local CLI installation."
        ))
    } else {
        None
    };
    (
        CapabilityPackSectionObject {
            key: format!("runtime-{runtime_name}"),
            title: format!("Runtime `{runtime_name}`"),
            description: "Runtime command and timeout settings.".to_string(),
            fields: vec![
                config_field(
                    PlannerTargetKind::Daemon,
                    &["inference", "runtimes", runtime_name, "command"],
                    "Command",
                    "Executable command for this runtime.",
                    "string",
                    current,
                    proposed,
                    Vec::new(),
                    true,
                    false,
                    editable,
                    None,
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &["inference", "runtimes", runtime_name, "args"],
                    "Args",
                    "Command-line arguments for this runtime.",
                    "json",
                    current,
                    proposed,
                    Vec::new(),
                    false,
                    false,
                    editable,
                    Some(json!([])),
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &[
                        "inference",
                        "runtimes",
                        runtime_name,
                        "startup_timeout_secs",
                    ],
                    "Startup timeout",
                    "Seconds to wait for the runtime to start.",
                    "integer",
                    current,
                    proposed,
                    Vec::new(),
                    true,
                    false,
                    editable,
                    Some(json!(60)),
                ),
                config_field(
                    PlannerTargetKind::Daemon,
                    &[
                        "inference",
                        "runtimes",
                        runtime_name,
                        "request_timeout_secs",
                    ],
                    "Request timeout",
                    "Seconds to wait for a response.",
                    "integer",
                    current,
                    proposed,
                    Vec::new(),
                    true,
                    false,
                    editable,
                    if matches!(runtime_name, "codex" | "claude") {
                        Some(json!(900))
                    } else {
                        Some(json!(300))
                    },
                ),
            ]
            .into_iter()
            .map(|mut field| {
                field.shared_owner = shared_owner.clone();
                field
            })
            .collect(),
        },
        warning,
    )
}

#[allow(clippy::too_many_arguments)]
fn config_field(
    target: PlannerTargetKind,
    path: &[&str],
    label: &str,
    description: &str,
    field_type: &str,
    current_root: &Value,
    proposed_root: &Value,
    allowed_values: Vec<String>,
    required: bool,
    secret: bool,
    editable: bool,
    default_value: Option<Value>,
) -> CapabilityPackFieldObject {
    let path_segments = path
        .iter()
        .map(|segment| (*segment).to_string())
        .collect::<Vec<_>>();
    let current_value = value_at_path(current_root, path)
        .cloned()
        .or_else(|| default_value.clone())
        .unwrap_or(Value::Null);
    let proposed_value = value_at_path(proposed_root, path)
        .cloned()
        .or(default_value)
        .unwrap_or(Value::Null);
    CapabilityPackFieldObject {
        key: format!("{}:{}", target.as_str(), path_segments.join(".")),
        target_kind: target.as_str().to_string(),
        config_path: path_segments,
        label: label.to_string(),
        description: description.to_string(),
        field_type: field_type.to_string(),
        current_value: Json(current_value),
        proposed_value: Json(proposed_value),
        allowed_values,
        required,
        secret,
        editable,
        shared_owner: None,
    }
}

fn build_review_groups(
    daemon_current: &Value,
    daemon_proposed: &Value,
) -> Vec<CapabilityPackReviewGroupObject> {
    let mut groups = Vec::new();
    let daemon_items = diff_value(String::new(), daemon_current, daemon_proposed);
    if !daemon_items.is_empty() {
        groups.push(CapabilityPackReviewGroupObject {
            key: "daemon".to_string(),
            title: "Daemon config changes".to_string(),
            items: daemon_items,
        });
    }
    groups
}

fn diff_value(prefix: String, current: &Value, proposed: &Value) -> Vec<String> {
    if current == proposed {
        return Vec::new();
    }
    match (current, proposed) {
        (Value::Object(current_map), Value::Object(proposed_map)) => {
            let mut keys = current_map
                .keys()
                .chain(proposed_map.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut items = Vec::new();
            for key in keys.split_off("") {
                let next_prefix = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                items.extend(diff_value(
                    next_prefix,
                    current_map.get(&key).unwrap_or(&Value::Null),
                    proposed_map.get(&key).unwrap_or(&Value::Null),
                ));
            }
            items
        }
        _ => vec![format!(
            "{} = {}",
            if prefix.is_empty() {
                "<root>"
            } else {
                prefix.as_str()
            },
            preview_value(proposed)
        )],
    }
}

fn preview_value(value: &Value) -> String {
    match value {
        Value::Null => "unset".to_string(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn build_combined_patches(
    input: &PlanCapabilityPackConfigInput,
    daemon_value: &Value,
) -> AnyhowResult<Vec<DraftPatch>> {
    let mut daemon_patches = Vec::new();

    daemon_patches.push(DraftPatch {
        target: PlannerTargetKind::Daemon,
        path: vec![
            "runtime".to_string(),
            "capability_policy".to_string(),
            "explicit_enabled".to_string(),
        ],
        value: Some(Value::Array(
            input
                .explicit_enabled
                .iter()
                .map(|value| Value::String(value.trim().to_string()))
                .collect(),
        )),
        unset: false,
    });
    daemon_patches.push(DraftPatch {
        target: PlannerTargetKind::Daemon,
        path: vec![
            "runtime".to_string(),
            "capability_policy".to_string(),
            "explicit_disabled".to_string(),
        ],
        value: Some(Value::Array(
            input
                .explicit_disabled
                .iter()
                .map(|value| Value::String(value.trim().to_string()))
                .collect(),
        )),
        unset: false,
    });

    if enabled_pack_ids(&input.explicit_enabled, daemon_value).contains("architecture_graph") {
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "architecture",
                "inference",
                ARCHITECTURE_FACT_SYNTHESIS_SLOT,
            ],
            json!("architecture_fact_synthesis_managed"),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "architecture",
                "inference",
                ARCHITECTURE_ROLE_ADJUDICATION_SLOT,
            ],
            json!("architecture_role_adjudication_managed"),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "profiles",
                "architecture_fact_synthesis_managed",
                "task",
            ],
            json!(STRUCTURED_GENERATION_TASK),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "profiles",
                "architecture_role_adjudication_managed",
                "task",
            ],
            json!(STRUCTURED_GENERATION_TASK),
            daemon_value,
        ));
    }
    if enabled_pack_ids(&input.explicit_enabled, daemon_value).contains("context_guidance") {
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "context_guidance",
                "inference",
                CONTEXT_GUIDANCE_GENERATION_SLOT,
            ],
            json!("guidance_llm"),
            daemon_value,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &["inference", "profiles", "guidance_llm", "task"],
            json!(TEXT_GENERATION_TASK),
            daemon_value,
        ));
    }

    daemon_patches.retain(|patch| !patch.unset || patch.value.is_some());
    for patch in &input.daemon_patches {
        daemon_patches.push(to_draft_patch(PlannerTargetKind::Daemon, patch)?);
    }

    let daemon_preview = apply_patches_to_json(daemon_value, &daemon_patches)?;
    for runtime_name in structured_runtime_names(&daemon_preview) {
        if matches!(runtime_name.as_str(), "codex" | "claude")
            && let Some((command, args)) = probe_local_runtime(runtime_name.as_str())
        {
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &["inference", "runtimes", runtime_name.as_str(), "command"],
                Value::String(command),
                &daemon_preview,
            ));
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &["inference", "runtimes", runtime_name.as_str(), "args"],
                Value::Array(args.into_iter().map(Value::String).collect()),
                &daemon_preview,
            ));
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &[
                    "inference",
                    "runtimes",
                    runtime_name.as_str(),
                    "startup_timeout_secs",
                ],
                json!(5),
                &daemon_preview,
            ));
            daemon_patches.push(seed_if_missing(
                PlannerTargetKind::Daemon,
                &[
                    "inference",
                    "runtimes",
                    runtime_name.as_str(),
                    "request_timeout_secs",
                ],
                json!(900),
                &daemon_preview,
            ));
        }
    }
    if requires_managed_bitloops_inference(&daemon_preview) {
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "command",
            ],
            json!("bitloops-inference"),
            &daemon_preview,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "args",
            ],
            json!([]),
            &daemon_preview,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "startup_timeout_secs",
            ],
            json!(60),
            &daemon_preview,
        ));
        daemon_patches.push(seed_if_missing(
            PlannerTargetKind::Daemon,
            &[
                "inference",
                "runtimes",
                BITLOOPS_INFERENCE_RUNTIME_ID,
                "request_timeout_secs",
            ],
            json!(300),
            &daemon_preview,
        ));
    }

    Ok(dedupe_patches(daemon_patches))
}

fn enabled_pack_ids(explicit_enabled: &[String], daemon_value: &Value) -> BTreeSet<String> {
    if explicit_enabled.is_empty() {
        read_string_list(
            daemon_value,
            &["runtime", "capability_policy", "explicit_enabled"],
        )
        .into_iter()
        .collect()
    } else {
        explicit_enabled
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }
}

fn structured_runtime_names(value: &Value) -> BTreeSet<String> {
    let Some(profiles) = value
        .get("inference")
        .and_then(|value| value.get("profiles"))
        .and_then(Value::as_object)
    else {
        return BTreeSet::new();
    };
    profiles
        .iter()
        .filter(|(_, profile)| {
            matches!(
                profile.get("driver").and_then(Value::as_str),
                Some("codex_exec" | "claude_code_print")
            )
        })
        .filter_map(|(_, profile)| profile.get("runtime").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn requires_managed_bitloops_inference(value: &Value) -> bool {
    let Some(profiles) = value
        .get("inference")
        .and_then(|value| value.get("profiles"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    profiles.values().any(|profile| {
        matches!(
            profile.get("driver").and_then(Value::as_str),
            Some("codex_exec" | "claude_code_print")
        )
    })
}

fn probe_local_runtime(runtime_name: &str) -> Option<(String, Vec<String>)> {
    let candidates = match runtime_name {
        "codex" => vec!["codex", "/opt/homebrew/bin/codex", "/usr/local/bin/codex"],
        "claude" => vec![
            "claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
        ],
        _ => Vec::new(),
    };
    for candidate in candidates {
        let path = if candidate.contains('/') {
            let path = PathBuf::from(candidate);
            if path.is_file() { Some(path) } else { None }
        } else {
            resolve_command_on_path(candidate)
        };
        if let Some(path) = path {
            return Some((path.display().to_string(), Vec::new()));
        }
    }
    None
}

fn resolve_command_on_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| {
        let direct = dir.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        executable_with_extensions(&dir, program)
            .into_iter()
            .find(|candidate| candidate.is_file())
    })
}

fn executable_with_extensions(dir: &Path, program: &str) -> [PathBuf; 3] {
    [
        dir.join(format!("{program}.exe")),
        dir.join(format!("{program}.cmd")),
        dir.join(format!("{program}.bat")),
    ]
}

fn to_draft_patch(
    target: PlannerTargetKind,
    patch: &CapabilityPackFieldPatchInput,
) -> AnyhowResult<DraftPatch> {
    if !patch.target.trim().is_empty() && patch.target.trim() != target.as_str() {
        bail!(
            "capability config patches are daemon-scoped; unsupported target `{}`",
            patch.target
        );
    }
    Ok(DraftPatch {
        target,
        path: patch.path.clone(),
        value: patch.value.as_ref().map(|value| value.0.clone()),
        unset: patch.unset.unwrap_or(false)
            || patch.value.as_ref().is_some_and(|value| value.0.is_null()),
    })
}

fn dedupe_patches(patches: Vec<DraftPatch>) -> Vec<DraftPatch> {
    let mut by_key = BTreeMap::<String, DraftPatch>::new();
    for patch in patches {
        let key = format!("{}:{}", patch.target.as_str(), patch.path.join("."));
        by_key.insert(key, patch);
    }
    by_key.into_values().collect()
}

fn seed_if_missing(
    target: PlannerTargetKind,
    path: &[&str],
    value: Value,
    current: &Value,
) -> DraftPatch {
    let path_segments = path
        .iter()
        .map(|segment| (*segment).to_string())
        .collect::<Vec<_>>();
    let exists = value_at_path(current, path).is_some_and(|value| !value.is_null());
    DraftPatch {
        target,
        path: path_segments,
        value: (!exists).then_some(value),
        unset: false,
    }
}

fn runtime_options_from_value(value: &Value) -> Vec<String> {
    value
        .get("inference")
        .and_then(|value| value.get("runtimes"))
        .and_then(Value::as_object)
        .map(|items| items.keys().cloned().collect())
        .unwrap_or_default()
}

fn profile_options_for_task(value: &Value, task: &str) -> Vec<String> {
    value
        .get("inference")
        .and_then(|value| value.get("profiles"))
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, profile)| {
                    (profile.get("task").and_then(Value::as_str) == Some(task))
                        .then_some(key.clone())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_value_at_path(value: &Value, path: &[&str]) -> Option<String> {
    value_at_path(value, path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn has_non_empty_value(value: &Value, path: &[&str]) -> bool {
    value_at_path(value, path)
        .map(|value| match value {
            Value::Null => false,
            Value::String(value) => !value.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(items) => !items.is_empty(),
            _ => true,
        })
        .unwrap_or(false)
}

fn has_truthy_value(value: &Value, path: &[&str]) -> bool {
    value_at_path(value, path)
        .map(|value| match value {
            Value::Bool(value) => *value,
            Value::Null => false,
            Value::String(value) => !value.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(items) => !items.is_empty(),
            _ => true,
        })
        .unwrap_or(false)
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn read_string_list(value: &Value, path: &[&str]) -> Vec<String> {
    value_at_path(value, path)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn load_daemon_file(path: PathBuf) -> AnyhowResult<LoadedConfigFile> {
    load_config_file(path, true)
}

fn load_config_file(path: PathBuf, require_exists: bool) -> AnyhowResult<LoadedConfigFile> {
    let raw_text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !require_exists => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading config {}", path.display()));
        }
    };
    let value = if raw_text.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        from_str::<Value>(&raw_text)
            .with_context(|| format!("parsing config {}", path.display()))?
    };
    let revision = (!raw_text.is_empty()).then(|| revision_for_bytes(raw_text.as_bytes()));
    Ok(LoadedConfigFile {
        path,
        raw_text,
        value,
        revision,
    })
}

fn apply_patches_to_json(root: &Value, patches: &[DraftPatch]) -> AnyhowResult<Value> {
    let mut next = root.clone();
    for patch in patches {
        if patch.unset || patch.value.as_ref().is_none_or(Value::is_null) {
            remove_json_path(&mut next, &patch.path);
            continue;
        }
        set_json_path(
            &mut next,
            &patch.path,
            patch
                .value
                .clone()
                .ok_or_else(|| anyhow!("patch value is required"))?,
        )?;
    }
    Ok(next)
}

fn set_json_path(root: &mut Value, path: &[String], value: Value) -> AnyhowResult<()> {
    if path.is_empty() {
        bail!("json patch path cannot be empty");
    }
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let mut current = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root must be an object"))?;
    for segment in &path[..path.len() - 1] {
        if current.get(segment).is_none_or(|value| !value.is_object()) {
            current.insert(segment.clone(), Value::Object(Map::new()));
        }
        current = current
            .get_mut(segment)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow!("failed to create nested config object for `{segment}`"))?;
    }
    current.insert(path.last().expect("path is non-empty").clone(), value);
    Ok(())
}

fn remove_json_path(root: &mut Value, path: &[String]) {
    if path.is_empty() {
        return;
    }
    let Some(mut current) = root.as_object_mut() else {
        return;
    };
    for segment in &path[..path.len() - 1] {
        let Some(next) = current.get_mut(segment).and_then(Value::as_object_mut) else {
            return;
        };
        current = next;
    }
    if let Some(last) = path.last() {
        current.remove(last);
    }
}

fn apply_patches_to_toml(original: &str, patches: &[DraftPatch]) -> AnyhowResult<String> {
    let mut doc = if original.trim().is_empty() {
        DocumentMut::new()
    } else {
        original.parse::<DocumentMut>()?
    };
    for patch in patches {
        apply_patch_to_document(&mut doc, patch.clone())?;
    }
    Ok(doc.to_string())
}

fn apply_patch_to_document(doc: &mut DocumentMut, patch: DraftPatch) -> AnyhowResult<()> {
    if patch.path.is_empty() {
        bail!("patch path cannot be empty");
    }
    if patch.unset || patch.value.as_ref().is_none_or(Value::is_null) {
        remove_path(doc, &patch.path);
        return Ok(());
    }
    set_path(
        doc,
        &patch.path,
        json_value_to_toml_item(
            &patch
                .value
                .ok_or_else(|| anyhow!("patch value is required"))?,
        )?,
    )
}

fn set_path(doc: &mut DocumentMut, path: &[String], value: Item) -> AnyhowResult<()> {
    if path.len() == 1 {
        doc[&path[0]] = value;
        return Ok(());
    }
    if doc.get(&path[0]).is_none_or(|item| !item.is_table()) {
        doc[&path[0]] = Item::Table(Table::new());
    }
    let mut table = doc[&path[0]]
        .as_table_mut()
        .ok_or_else(|| anyhow!("{} is not a table", path[0]))?;
    for segment in &path[1..path.len() - 1] {
        if table.get(segment).is_none_or(|item| !item.is_table()) {
            table[segment] = Item::Table(Table::new());
        }
        table = table[segment]
            .as_table_mut()
            .ok_or_else(|| anyhow!("{segment} is not a table"))?;
    }
    table[path.last().expect("path is non-empty")] = value;
    Ok(())
}

fn remove_path(doc: &mut DocumentMut, path: &[String]) {
    if path.len() == 1 {
        doc.as_table_mut().remove(&path[0]);
        return;
    }
    let Some(mut table) = doc.get_mut(&path[0]).and_then(Item::as_table_mut) else {
        return;
    };
    for segment in &path[1..path.len() - 1] {
        let Some(next) = table.get_mut(segment).and_then(Item::as_table_mut) else {
            return;
        };
        table = next;
    }
    if let Some(last) = path.last() {
        table.remove(last);
    }
}

fn json_value_to_toml_item(value: &Value) -> AnyhowResult<Item> {
    match value {
        Value::Null => Ok(Item::None),
        Value::Bool(value) => Ok(Item::Value(TomlValue::from(*value))),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            if let Some(value) = number.as_u64() {
                return i64::try_from(value)
                    .map(|value| Item::Value(TomlValue::from(value)))
                    .map_err(|_| anyhow!("TOML integer value is too large: {number}"));
            }
            if let Some(value) = number.as_f64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            bail!("unsupported numeric config value `{number}`")
        }
        Value::String(value) => Ok(Item::Value(TomlValue::from(value.as_str()))),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let Item::Value(value) = json_value_to_toml_item(value)? else {
                    bail!("TOML arrays may only contain scalar values");
                };
                array.push(value);
            }
            Ok(Item::Value(TomlValue::Array(array)))
        }
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, value) in map {
                table[key] = json_value_to_toml_item(value)?;
            }
            Ok(Item::Table(table))
        }
    }
}

fn hash_plan(
    input: &PlanCapabilityPackConfigInput,
    daemon_revision: &str,
    daemon_preview: &Value,
) -> AnyhowResult<String> {
    let payload = serde_json::to_vec(&json!({
        "explicit_enabled": input.explicit_enabled,
        "explicit_disabled": input.explicit_disabled,
        "daemon_patches": input.daemon_patches.iter().map(|patch| json!({
            "target": patch.target,
            "path": patch.path,
            "value": patch.value,
            "unset": patch.unset,
        })).collect::<Vec<_>>(),
        "daemon_revision": daemon_revision,
        "daemon_preview": daemon_preview,
    }))?;
    Ok(revision_for_bytes(&payload))
}

fn schedule_delayed_daemon_restart(config_path: &Path) -> AnyhowResult<()> {
    let executable = env::current_exe().context("resolving Bitloops executable for restart")?;
    let mut command = Command::new(executable);
    command
        .arg("__delayed-daemon-restart")
        .arg("--config")
        .arg(config_path)
        .arg("--delay-ms")
        .arg("750")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().with_context(|| {
        format!(
            "spawning delayed daemon restart for {}",
            config_path.display()
        )
    })?;
    Ok(())
}

fn revision_for_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn pack_metadata(repo_root: &Path) -> AnyhowResult<Vec<PackMetadata>> {
    builtin_packs(repo_root).map(|packs| {
        packs
            .into_iter()
            .map(|pack| {
                let descriptor = pack.descriptor();
                PackMetadata {
                    id: descriptor.id.to_string(),
                    display_name: descriptor.display_name.to_string(),
                    description: descriptor.description.to_string(),
                    version: descriptor.version.to_string(),
                    default_enabled: descriptor.default_enabled,
                    experimental: descriptor.experimental,
                    dependencies: descriptor
                        .dependencies
                        .iter()
                        .map(|dependency| CapabilityPackDependencyObject {
                            pack_id: dependency.capability_id.to_string(),
                            min_version: dependency.min_version.to_string(),
                        })
                        .collect(),
                    inference_slots: descriptor
                        .inference_slots
                        .iter()
                        .map(|slot| CapabilityPackInferenceSlotObject {
                            name: slot.name.to_string(),
                            task: slot.task.to_string(),
                        })
                        .collect(),
                }
            })
            .collect()
    })
}

fn ui_spec_for_pack(pack_id: &str) -> Option<PackUiSpec> {
    PACK_UI_SPECS
        .iter()
        .find(|spec| spec.pack_id == pack_id)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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

        let selection =
            resolve_selection(temp.path(), &daemon, &["context_guidance".to_string()], &[])
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

        let err =
            build_combined_patches(&input, &json!({})).expect_err("repo-local patch should fail");

        assert!(err.to_string().contains("daemon-scoped"));
    }
}
