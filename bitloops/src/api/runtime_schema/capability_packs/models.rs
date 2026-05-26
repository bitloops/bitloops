use std::collections::BTreeMap;
use std::path::PathBuf;

use async_graphql::{InputObject, SimpleObject, types::Json};
use serde_json::Value;

pub(super) type ConfigJsonScalar = Json<Value>;

pub(super) const STRUCTURED_GENERATION_TASK: &str = "structured_generation";
pub(super) const TEXT_GENERATION_TASK: &str = "text_generation";
pub(super) const EMBEDDINGS_TASK: &str = "embeddings";

pub(super) const ARCHITECTURE_ROLE_ADJUDICATION_SLOT: &str = "role_adjudication";
pub(super) const ARCHITECTURE_FACT_SYNTHESIS_SLOT: &str = "fact_synthesis";
pub(super) const CONTEXT_GUIDANCE_GENERATION_SLOT: &str = "guidance_generation";
pub(super) const SEMANTIC_CLONES_SUMMARY_SLOT: &str = "summary_generation";
pub(super) const SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT: &str = "code_embeddings";
pub(super) const SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT: &str = "summary_embeddings";

pub(super) const DRIVER_OPTIONS_TEXT_GENERATION: &[&str] = &[
    "bitloops_platform_chat",
    "openai_chat_completions",
    "ollama_chat",
];
pub(super) const DRIVER_OPTIONS_STRUCTURED_GENERATION: &[&str] =
    &["codex_exec", "claude_code_print"];
pub(super) const DRIVER_OPTIONS_EMBEDDINGS: &[&str] = &["bitloops_embeddings_ipc"];
pub(super) const THINKING_LEVEL_OPTIONS: &[&str] = &["low", "medium", "high"];
pub(super) const ARCHITECTURE_RUNTIME_OPTIONS: &[&str] = &["codex", "claude"];
pub(super) const TEST_HARNESS_COVERAGE_FORMAT_OPTIONS: &[&str] = &["lcov"];
pub(super) const SEMANTIC_SUMMARY_MODE_OPTIONS: &[&str] = &["auto", "off"];
pub(super) const SEMANTIC_EMBEDDING_MODE_OPTIONS: &[&str] = &[
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
pub(super) enum PlannerTargetKind {
    Daemon,
}

impl PlannerTargetKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PackMetadata {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) default_enabled: bool,
    pub(super) experimental: bool,
    pub(super) dependencies: Vec<CapabilityPackDependencyObject>,
    pub(super) inference_slots: Vec<CapabilityPackInferenceSlotObject>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PackUiSpec {
    pub(super) pack_id: &'static str,
    pub(super) dashboard_configurable: bool,
    pub(super) display_order: i32,
}

pub(super) struct LoadedConfigFile {
    pub(super) path: PathBuf,
    pub(super) raw_text: String,
    pub(super) value: Value,
    pub(super) revision: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct DraftPatch {
    pub(super) target: PlannerTargetKind,
    pub(super) path: Vec<String>,
    pub(super) value: Option<Value>,
    pub(super) unset: bool,
}

#[derive(Debug, Clone)]
pub(super) struct PackSelectionState {
    pub(super) enabled: bool,
    pub(super) selection_source: String,
    pub(super) dependency_reason: Option<String>,
}

pub(super) struct ResolvedSelection {
    pub(super) states: BTreeMap<String, PackSelectionState>,
    pub(super) blockers: Vec<String>,
}
