use std::env;
use std::path::Path;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::runtime_config::embedding_slot_for_representation;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
    SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
};
use crate::config::{InferenceTask, SemanticSummaryMode};
use crate::host::capability_host::CapabilityHealthContext;
use crate::host::capability_host::health::{CapabilityHealthCheck, CapabilityHealthResult};

use super::runtime_config::{
    embeddings_enabled, resolve_selected_summary_slot, resolve_semantic_clones_config,
};
use super::types::SEMANTIC_CLONES_CAPABILITY_ID;

fn check_semantic_clones_semantic_summaries(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let Ok(view) = ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID) else {
        return CapabilityHealthResult::failed(
            "semantic_clones.semantic_summaries",
            "semantic_clones config is unavailable",
        );
    };
    let config = resolve_semantic_clones_config(&view);
    if config.summary_mode == SemanticSummaryMode::Off {
        return CapabilityHealthResult::ok("semantic summaries disabled");
    }

    let Some(slot_name) = resolve_selected_summary_slot(&config) else {
        return CapabilityHealthResult::ok("semantic summaries not configured");
    };
    let profile_name = ctx
        .inference()
        .describe(&slot_name)
        .map(|slot| slot.profile_name)
        .unwrap_or_else(|| "<unresolved>".to_string());

    match ctx.inference().text_generation(&slot_name) {
        Ok(_) => CapabilityHealthResult::ok(format!(
            "semantic summary slot `{slot_name}` ready (profile `{profile_name}`)"
        )),
        Err(err) => {
            CapabilityHealthResult::failed("semantic_clones.semantic_summaries", format!("{err:#}"))
        }
    }
}

fn check_semantic_clones_profile_resolution(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let Ok(view) = ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID) else {
        return CapabilityHealthResult::failed(
            "semantic_clones.profile_resolution",
            "semantic_clones config is unavailable",
        );
    };
    let config = resolve_semantic_clones_config(&view);
    let configured_slots = configured_inference_slots(&config);
    if configured_slots.is_empty() {
        if !embeddings_enabled(&config) {
            return CapabilityHealthResult::ok("semantic_clones embeddings disabled");
        }
        return CapabilityHealthResult::ok("semantic_clones inference slots not configured");
    }

    let mut resolved = Vec::new();
    for (representation, slot_name, expected_task) in configured_slots {
        if !ctx.inference().has_slot(slot_name) {
            return CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!("{representation} slot `{slot_name}` is not bound"),
            );
        }
        let Some(slot) = ctx.inference().describe(slot_name) else {
            return CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!("{representation} slot `{slot_name}` is unresolved"),
            );
        };
        if slot.task != Some(expected_task) {
            return CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!(
                    "{representation} slot `{slot_name}` points to profile `{}` with task `{}` instead of `{}`",
                    slot.profile_name,
                    slot.task
                        .map(|task| task.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    expected_task
                ),
            );
        }
        resolved.push(format!("{representation} -> {}", slot.profile_name));
    }

    CapabilityHealthResult::ok(format!(
        "semantic_clones embedding slots resolved: {}",
        resolved.join(", ")
    ))
}

fn check_semantic_clones_runtime_command(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let Ok(view) = ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID) else {
        return CapabilityHealthResult::failed(
            "semantic_clones.runtime_command",
            "semantic_clones config is unavailable",
        );
    };
    let config = resolve_semantic_clones_config(&view);
    let configured_slots = configured_inference_slots(&config);
    if configured_slots.is_empty() {
        return CapabilityHealthResult::ok(
            "semantic_clones inference slots not configured (runtime command not required)",
        );
    }

    let mut checked = Vec::new();
    for (representation, slot_name, _) in configured_slots {
        let Some(slot) = ctx.inference().describe(slot_name) else {
            return CapabilityHealthResult::failed(
                "semantic_clones.runtime_command",
                format!("{representation} slot `{slot_name}` is unresolved"),
            );
        };
        if slot.task == Some(InferenceTask::TextGeneration) && slot.runtime.is_none() {
            return CapabilityHealthResult::failed(
                "semantic_clones.runtime_command",
                format!(
                    "text-generation profile `{}` for slot `{slot_name}` has no runtime configured",
                    slot.profile_name
                ),
            );
        }
        let Some(runtime_name) = slot.runtime.as_deref() else {
            continue;
        };
        let Some(command) = runtime_command(view.root(), runtime_name) else {
            return CapabilityHealthResult::failed(
                "semantic_clones.runtime_command",
                format!(
                    "runtime `{runtime_name}` for profile `{}` has no command configured",
                    slot.profile_name
                ),
            );
        };
        if !command_exists(&command) {
            return CapabilityHealthResult::failed(
                "semantic_clones.runtime_command",
                format!("{representation} runtime command `{command}` was not found in PATH"),
            );
        }
        checked.push(format!("{} -> {}", slot.profile_name, command));
    }

    if checked.is_empty() {
        CapabilityHealthResult::ok(
            "semantic_clones embedding slots do not require a local runtime command",
        )
    } else {
        CapabilityHealthResult::ok(format!(
            "semantic_clones runtime commands available: {}",
            checked.join(", ")
        ))
    }
}

fn check_semantic_clones_runtime_handshake(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let Ok(view) = ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID) else {
        return CapabilityHealthResult::failed(
            "semantic_clones.runtime_handshake",
            "semantic_clones config is unavailable",
        );
    };
    let config = resolve_semantic_clones_config(&view);
    let configured_slots = configured_inference_slots(&config);
    if configured_slots.is_empty() {
        return CapabilityHealthResult::ok(
            "semantic_clones inference slots not configured (runtime handshake skipped)",
        );
    }

    let mut ready = Vec::new();
    for (representation, slot_name, task) in configured_slots {
        let resolution = match task {
            InferenceTask::Embeddings => ctx.inference().embeddings(slot_name).map(|_| ()),
            InferenceTask::TextGeneration => ctx.inference().text_generation(slot_name).map(|_| ()),
        };
        match resolution {
            Ok(_) => {
                let profile_name = ctx
                    .inference()
                    .describe(slot_name)
                    .map(|slot| slot.profile_name)
                    .unwrap_or_else(|| "<unresolved>".to_string());
                ready.push(format!("{representation} -> {profile_name}"));
            }
            Err(err) => {
                return CapabilityHealthResult::failed(
                    "semantic_clones.runtime_handshake",
                    format!("{representation} slot `{slot_name}` failed: {err:#}"),
                );
            }
        }
    }

    CapabilityHealthResult::ok(format!(
        "semantic_clones inference slots ready: {}",
        ready.join(", ")
    ))
}

fn configured_embedding_slots(
    config: &crate::config::SemanticClonesConfig,
) -> Vec<(&'static str, &'static str)> {
    let mut slots = Vec::new();
    if embedding_slot_for_representation(config, EmbeddingRepresentationKind::Code).as_deref()
        == Some(SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT)
    {
        slots.push(("code", SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT));
    }
    if embedding_slot_for_representation(config, EmbeddingRepresentationKind::Summary).as_deref()
        == Some(SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT)
    {
        slots.push(("summary", SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT));
    }
    slots
}

fn configured_summary_slot(
    config: &crate::config::SemanticClonesConfig,
) -> Option<(&'static str, &'static str, InferenceTask)> {
    (resolve_selected_summary_slot(config).as_deref()
        == Some(SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT))
    .then_some((
        "summary generation",
        SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
        InferenceTask::TextGeneration,
    ))
}

fn configured_inference_slots(
    config: &crate::config::SemanticClonesConfig,
) -> Vec<(&'static str, &'static str, InferenceTask)> {
    let mut slots = Vec::new();
    if let Some(summary_slot) = configured_summary_slot(config) {
        slots.push(summary_slot);
    }
    slots.extend(
        configured_embedding_slots(config)
            .into_iter()
            .map(|(representation, slot_name)| {
                (representation, slot_name, InferenceTask::Embeddings)
            }),
    );
    slots
}

fn runtime_command(root: &serde_json::Value, runtime_name: &str) -> Option<String> {
    root.get("inference")?
        .get("runtimes")?
        .get(runtime_name)?
        .get("command")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn command_exists(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    let candidate = Path::new(command);
    if candidate.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return candidate.exists();
    }

    env::var_os("PATH")
        .map(|path| env::split_paths(&path).any(|dir| dir.join(command).exists()))
        .unwrap_or(false)
}

pub static SEMANTIC_CLONES_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "semantic_clones.semantic_summaries",
        run: check_semantic_clones_semantic_summaries,
    },
    CapabilityHealthCheck {
        name: "semantic_clones.profile_resolution",
        run: check_semantic_clones_profile_resolution,
    },
    CapabilityHealthCheck {
        name: "semantic_clones.runtime_command",
        run: check_semantic_clones_runtime_command,
    },
    CapabilityHealthCheck {
        name: "semantic_clones.runtime_handshake",
        run: check_semantic_clones_runtime_handshake,
    },
];
