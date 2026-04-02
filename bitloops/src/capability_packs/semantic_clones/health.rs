use std::env;
use std::path::{Path, PathBuf};

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, SemanticCloneEmbeddingMode, SemanticSummaryMode,
    default_daemon_config_path, resolve_embedding_capability_config_for_repo,
    resolve_store_semantic_config_for_repo,
};
use crate::host::capability_host::CapabilityHealthContext;
use crate::host::capability_host::health::{CapabilityHealthCheck, CapabilityHealthResult};

use super::embeddings::{EmbeddingProviderConfig, build_symbol_embedding_provider};
use super::extension_descriptor::build_semantic_summary_provider;
use super::features::SemanticSummaryProviderConfig;

fn check_semantic_clones_semantic_summaries(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let capability = resolve_embedding_capability_config_for_repo(ctx.repo_root());
    if capability.semantic_clones.summary_mode == SemanticSummaryMode::Off {
        return CapabilityHealthResult::ok("semantic summaries disabled");
    }

    let semantic_cfg = resolve_store_semantic_config_for_repo(ctx.repo_root());
    let provider = semantic_cfg
        .semantic_provider
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if provider.is_empty() || matches!(provider.as_str(), "none" | "disabled") {
        return CapabilityHealthResult::ok("semantic summaries use deterministic fallback only");
    }

    match build_semantic_summary_provider(&SemanticSummaryProviderConfig {
        semantic_provider: semantic_cfg.semantic_provider,
        semantic_model: semantic_cfg.semantic_model,
        semantic_api_key: semantic_cfg.semantic_api_key,
        semantic_base_url: semantic_cfg.semantic_base_url,
    }) {
        Ok(_) => CapabilityHealthResult::ok("semantic summary provider ready"),
        Err(err) => {
            CapabilityHealthResult::failed("semantic_clones.semantic_summaries", format!("{err:#}"))
        }
    }
}

fn check_semantic_clones_profile_resolution(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let capability = resolve_embedding_capability_config_for_repo(ctx.repo_root());
    if embeddings_disabled(&capability) {
        return CapabilityHealthResult::ok("semantic_clones embeddings disabled");
    }
    if let Some(profile_name) = capability.semantic_clones.embedding_profile.as_deref() {
        if capability.embeddings.profiles.contains_key(profile_name) {
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
    let capability = resolve_embedding_capability_config_for_repo(ctx.repo_root());
    if embeddings_disabled(&capability) {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime command not required)",
        );
    }
    let Some(profile_name) = capability.semantic_clones.embedding_profile.as_deref() else {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime command not required)",
        );
    };

    if command_exists(&capability.embeddings.runtime.command) {
        CapabilityHealthResult::ok(format!(
            "semantic_clones runtime command available for profile `{profile_name}`"
        ))
    } else {
        CapabilityHealthResult::failed(
            "semantic_clones.runtime_command",
            format!(
                "embedding runtime command `{}` was not found in PATH",
                capability.embeddings.runtime.command
            ),
        )
    }
}

fn check_semantic_clones_runtime_handshake(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let capability = resolve_embedding_capability_config_for_repo(ctx.repo_root());
    if embeddings_disabled(&capability) {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime handshake skipped)",
        );
    }
    let Some(profile_name) = capability.semantic_clones.embedding_profile.clone() else {
        return CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime handshake skipped)",
        );
    };

    let config_path = daemon_config_path_for_health(ctx.repo_root());
    let config = EmbeddingProviderConfig {
        daemon_config_path: config_path,
        embedding_profile: Some(profile_name.clone()),
        runtime_command: capability.embeddings.runtime.command,
        runtime_args: capability.embeddings.runtime.args,
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        warnings: capability.embeddings.warnings,
    };

    match build_symbol_embedding_provider(&config, Some(ctx.repo_root())) {
        Ok(Some(_provider)) => CapabilityHealthResult::ok(format!(
            "semantic_clones runtime describe succeeded for profile `{profile_name}`"
        )),
        Ok(None) => CapabilityHealthResult::ok(
            "semantic_clones embeddings disabled (runtime handshake skipped)",
        ),
        Err(err) => {
            CapabilityHealthResult::failed("semantic_clones.runtime_handshake", format!("{err:#}"))
        }
    }
}

fn embeddings_disabled(capability: &crate::config::EmbeddingCapabilityConfig) -> bool {
    capability.semantic_clones.embedding_mode == SemanticCloneEmbeddingMode::Off
        || capability.semantic_clones.embedding_profile.is_none()
}

fn daemon_config_path_for_health(repo_root: &Path) -> PathBuf {
    default_daemon_config_path().unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH))
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
