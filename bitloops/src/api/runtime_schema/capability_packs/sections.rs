use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result as AnyhowResult;
use serde_json::{Value, json};

use super::fields::{build_inference_profile_sections, config_field};
use super::models::*;
use super::values::{
    has_non_empty_value, has_truthy_value, profile_options_for_task, string_value_at_path,
};

pub(super) fn build_pack_sections(
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
