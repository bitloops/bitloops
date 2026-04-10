use std::env;
use std::path::Path;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::runtime_config::embedding_slot_for_representation;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
};
use crate::config::{InferenceTask, SemanticCloneEmbeddingMode, SemanticSummaryMode};
use crate::host::capability_host::CapabilityHealthContext;
use crate::host::capability_host::health::{CapabilityHealthCheck, CapabilityHealthResult};

use super::runtime_config::{resolve_selected_summary_slot, resolve_semantic_clones_config};
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
        return CapabilityHealthResult::ok("semantic summaries use deterministic fallback only");
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
    if embeddings_disabled(&config) {
        return CapabilityHealthResult::ok("semantic_clones embeddings disabled");
    }

    let mut resolved = Vec::new();
    for (representation, slot_name) in configured_embedding_slots(&config) {
        if !ctx.inference().has_slot(slot_name) {
            return CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!("{representation} embedding slot `{slot_name}` is not bound"),
            );
        }
        let Some(slot) = ctx.inference().describe(slot_name) else {
            return CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!("{representation} embedding slot `{slot_name}` is unresolved"),
            );
        };
        if slot.task != Some(InferenceTask::Embeddings) {
            return CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!(
                    "{representation} embedding slot `{slot_name}` points to non-embedding profile `{}`",
                    slot.profile_name
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
    if embeddings_disabled(&config) {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime command not required)",
        );
    }

    let mut checked = Vec::new();
    for (_, slot_name) in configured_embedding_slots(&config) {
        let Some(slot) = ctx.inference().describe(slot_name) else {
            return CapabilityHealthResult::failed(
                "semantic_clones.runtime_command",
                format!("embedding slot `{slot_name}` is unresolved"),
            );
        };
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
                format!("embedding runtime command `{command}` was not found in PATH"),
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
    if embeddings_disabled(&config) {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime handshake skipped)",
        );
    }

    let mut ready = Vec::new();
    for (representation, slot_name) in configured_embedding_slots(&config) {
        match ctx.inference().embeddings(slot_name) {
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
                    format!("{representation} embedding slot `{slot_name}` failed: {err:#}"),
                );
            }
        }
    }

    CapabilityHealthResult::ok(format!(
        "semantic_clones embedding slots ready: {}",
        ready.join(", ")
    ))
}

fn embeddings_disabled(config: &crate::config::SemanticClonesConfig) -> bool {
    config.embedding_mode == SemanticCloneEmbeddingMode::Off
        || configured_embedding_slots(config).is_empty()
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
