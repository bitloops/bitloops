use std::env;
use std::path::Path;

use crate::config::{SemanticCloneEmbeddingMode, SemanticSummaryMode};
use crate::host::capability_host::CapabilityHealthContext;
use crate::host::capability_host::health::{CapabilityHealthCheck, CapabilityHealthResult};

use super::runtime_config::{resolve_selected_summary_profile, resolve_semantic_clones_config};
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

    let Some(profile_name) = resolve_selected_summary_profile(&config, ctx.inference()) else {
        return CapabilityHealthResult::ok("semantic summaries use deterministic fallback only");
    };

    match ctx.inference().text_generation(&profile_name) {
        Ok(_) => {
            CapabilityHealthResult::ok(format!("semantic summary provider `{profile_name}` ready"))
        }
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
    if let Some(profile_name) = config.embedding_profile.as_deref() {
        if ctx.inference().has_embedding_profile(profile_name) {
            CapabilityHealthResult::ok(format!(
                "semantic_clones embedding profile `{profile_name}` resolved"
            ))
        } else {
            CapabilityHealthResult::failed(
                "semantic_clones.profile_resolution",
                format!("embedding profile `{profile_name}` is not defined"),
            )
        }
    } else {
        CapabilityHealthResult::ok("semantic_clones embeddings disabled")
    }
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
    let command = view
        .root()
        .pointer("/embeddings/runtime/command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if let Some(profile_name) = config.embedding_profile.as_deref() {
        if command_exists(command) {
            CapabilityHealthResult::ok(format!(
                "semantic_clones runtime command available for profile `{profile_name}`"
            ))
        } else {
            CapabilityHealthResult::failed(
                "semantic_clones.runtime_command",
                format!("embedding runtime command `{command}` was not found in PATH"),
            )
        }
    } else {
        CapabilityHealthResult::ok("semantic_clones embeddings disabled")
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
    let Some(profile_name) = config.embedding_profile.as_deref() else {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime handshake skipped)",
        );
    };

    match ctx.inference().embeddings(profile_name) {
        Ok(_) => CapabilityHealthResult::ok(format!(
            "semantic_clones runtime describe succeeded for profile `{profile_name}`"
        )),
        Err(err) => {
            CapabilityHealthResult::failed("semantic_clones.runtime_handshake", format!("{err:#}"))
        }
    }
}

fn embeddings_disabled(config: &crate::config::SemanticClonesConfig) -> bool {
    config.embedding_mode == SemanticCloneEmbeddingMode::Off || config.embedding_profile.is_none()
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
