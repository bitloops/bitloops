use std::collections::BTreeMap;

use async_graphql::types::Json;
use serde_json::{Value, json};

use super::models::*;
use super::values::{
    has_non_empty_value, runtime_options_from_value, string_value_at_path, value_at_path,
};

pub(super) fn build_inference_profile_sections(
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
pub(super) fn config_field(
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
