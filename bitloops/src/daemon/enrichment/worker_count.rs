use std::path::Path;

#[cfg(test)]
use crate::config::SemanticClonesConfig;
use crate::config::{
    DEFAULT_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS, DEFAULT_SEMANTIC_CLONES_EMBEDDING_WORKERS,
    DEFAULT_SEMANTIC_CLONES_SUMMARY_WORKERS, resolve_inference_capability_config_for_repo,
    resolve_semantic_clones_worker_settings_for_repo,
};
use crate::host::inference::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID,
    BITLOOPS_PLATFORM_CHAT_DRIVER, DEFAULT_REMOTE_TEXT_GENERATION_CONCURRENCY,
    OPENAI_CHAT_COMPLETIONS_DRIVER,
};

const SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV: &str = "BITLOOPS_SEMANTIC_CLONES_SUMMARY_WORKERS";
const SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV: &str =
    "BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS";
const SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV: &str =
    "BITLOOPS_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS";
const SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV: &str =
    "BITLOOPS_SEMANTIC_CLONES_ENRICHMENT_WORKERS";
const MAX_ENRICHMENT_WORKER_COUNT: usize = 32;
const DEFAULT_REMOTE_SUMMARY_WORKERS: usize = DEFAULT_REMOTE_TEXT_GENERATION_CONCURRENCY;
const DEFAULT_REMOTE_EMBEDDING_WORKERS: usize = 4;
const OLLAMA_CHAT_DRIVER: &str = "ollama_chat";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnrichmentWorkerPool {
    SummaryRefresh,
    Embeddings,
    CloneRebuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EnrichmentWorkerBudgets {
    pub summary_refresh: usize,
    pub embeddings: usize,
    pub clone_rebuild: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct WorkerCountSource<'a> {
    raw: Option<&'a str>,
    configured: Option<usize>,
}

impl<'a> WorkerCountSource<'a> {
    const fn new(raw: Option<&'a str>, configured: Option<usize>) -> Self {
        Self { raw, configured }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct WorkerBudgetSources<'a> {
    summary: WorkerCountSource<'a>,
    embeddings: WorkerCountSource<'a>,
    clone_rebuild: WorkerCountSource<'a>,
    legacy_embeddings: WorkerCountSource<'a>,
    summary_remote: bool,
    embeddings_remote: bool,
    multiple_embedding_representations_active: bool,
}

impl EnrichmentWorkerBudgets {
    pub(crate) fn for_pool(self, pool: EnrichmentWorkerPool) -> usize {
        match pool {
            EnrichmentWorkerPool::SummaryRefresh => self.summary_refresh,
            EnrichmentWorkerPool::Embeddings => self.embeddings,
            EnrichmentWorkerPool::CloneRebuild => self.clone_rebuild,
        }
    }

    pub(crate) fn set_for_pool(&mut self, pool: EnrichmentWorkerPool, value: usize) {
        match pool {
            EnrichmentWorkerPool::SummaryRefresh => self.summary_refresh = value,
            EnrichmentWorkerPool::Embeddings => self.embeddings = value,
            EnrichmentWorkerPool::CloneRebuild => self.clone_rebuild = value,
        }
    }
}

impl EnrichmentWorkerPool {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SummaryRefresh => "summary_refresh",
            Self::Embeddings => "embeddings",
            Self::CloneRebuild => "clone_rebuild",
        }
    }
}

#[cfg(test)]
pub(crate) fn configured_enrichment_worker_budgets() -> EnrichmentWorkerBudgets {
    std::env::current_dir()
        .ok()
        .as_deref()
        .map(configured_enrichment_worker_budgets_for_repo)
        .unwrap_or_else(|| {
            configured_enrichment_worker_budgets_for_config(&SemanticClonesConfig::default())
        })
}

pub(crate) fn configured_enrichment_worker_budgets_for_repo(
    repo_root: &Path,
) -> EnrichmentWorkerBudgets {
    let workers = resolve_semantic_clones_worker_settings_for_repo(repo_root);
    let capability = resolve_inference_capability_config_for_repo(repo_root);
    resolve_worker_budgets_from_sources(WorkerBudgetSources {
        summary: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            workers.summary_workers,
        ),
        embeddings: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            workers.embedding_workers,
        ),
        clone_rebuild: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            workers.clone_rebuild_workers,
        ),
        legacy_embeddings: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            workers.legacy_enrichment_workers,
        ),
        summary_remote: summary_provider_is_remote(&capability),
        embeddings_remote: embeddings_provider_is_remote(&capability),
        multiple_embedding_representations_active: multiple_embedding_representations_active(
            &capability,
        ),
    })
}

#[cfg(test)]
pub(crate) fn configured_enrichment_worker_budgets_for_config(
    config: &SemanticClonesConfig,
) -> EnrichmentWorkerBudgets {
    resolve_worker_budgets_from_sources(WorkerBudgetSources {
        summary: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            Some(config.summary_workers),
        ),
        embeddings: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            Some(config.embedding_workers),
        ),
        clone_rebuild: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            Some(config.clone_rebuild_workers),
        ),
        legacy_embeddings: WorkerCountSource::new(
            std::env::var(SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV)
                .ok()
                .as_deref(),
            Some(config.enrichment_workers),
        ),
        ..WorkerBudgetSources::default()
    })
}

fn resolve_worker_budgets_from_sources(
    sources: WorkerBudgetSources<'_>,
) -> EnrichmentWorkerBudgets {
    let legacy_embedding_override = resolve_worker_count_override(sources.legacy_embeddings.raw);
    let default_summary_workers = if sources.summary_remote {
        DEFAULT_REMOTE_SUMMARY_WORKERS
    } else {
        DEFAULT_SEMANTIC_CLONES_SUMMARY_WORKERS
    };
    let default_embedding_workers = if sources.embeddings_remote {
        DEFAULT_REMOTE_EMBEDDING_WORKERS
    } else if sources.multiple_embedding_representations_active {
        2
    } else {
        DEFAULT_SEMANTIC_CLONES_EMBEDDING_WORKERS
    };
    EnrichmentWorkerBudgets {
        summary_refresh: resolve_worker_count_override(sources.summary.raw)
            .or(sources.summary.configured)
            .unwrap_or(default_summary_workers)
            .clamp(1, MAX_ENRICHMENT_WORKER_COUNT),
        embeddings: resolve_worker_count_override(sources.embeddings.raw)
            .or(sources.embeddings.configured)
            .or(legacy_embedding_override)
            .or(sources.legacy_embeddings.configured)
            .unwrap_or(default_embedding_workers)
            .clamp(1, MAX_ENRICHMENT_WORKER_COUNT),
        clone_rebuild: resolve_worker_count_override(sources.clone_rebuild.raw)
            .or(sources.clone_rebuild.configured)
            .unwrap_or(DEFAULT_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS)
            .clamp(1, MAX_ENRICHMENT_WORKER_COUNT),
    }
}

fn summary_provider_is_remote(capability: &crate::config::InferenceCapabilityConfig) -> bool {
    let Some(profile_name) = capability
        .semantic_clones
        .inference
        .summary_generation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(profile) = capability.inference.profiles.get(profile_name) else {
        return false;
    };
    let driver = profile.driver.trim();
    if driver == OLLAMA_CHAT_DRIVER {
        return false;
    }
    driver == BITLOOPS_PLATFORM_CHAT_DRIVER
        || driver == OPENAI_CHAT_COMPLETIONS_DRIVER
        || profile
            .base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

fn embeddings_provider_is_remote(capability: &crate::config::InferenceCapabilityConfig) -> bool {
    [
        capability
            .semantic_clones
            .inference
            .code_embeddings
            .as_deref(),
        capability
            .semantic_clones
            .inference
            .summary_embeddings
            .as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .any(|profile_name| {
        capability
            .inference
            .profiles
            .get(profile_name)
            .is_some_and(embedding_profile_is_remote)
    })
}

fn multiple_embedding_representations_active(
    capability: &crate::config::InferenceCapabilityConfig,
) -> bool {
    let code_embeddings_configured = capability
        .semantic_clones
        .inference
        .code_embeddings
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let summary_generation_configured = capability
        .semantic_clones
        .inference
        .summary_generation
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let summary_embeddings_configured = capability
        .semantic_clones
        .inference
        .summary_embeddings
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    code_embeddings_configured && summary_generation_configured && summary_embeddings_configured
}

fn embedding_profile_is_remote(profile: &crate::config::InferenceProfileConfig) -> bool {
    let driver = profile.driver.trim();
    if driver == BITLOOPS_EMBEDDINGS_IPC_DRIVER {
        return profile.runtime.as_deref().map(str::trim)
            != Some(BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID);
    }
    profile
        .base_url
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || !driver.is_empty()
}

fn resolve_worker_count_override(raw_value: Option<&str>) -> Option<usize> {
    raw_value
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|count| *count > 0)
}

#[cfg(test)]
mod tests {
    use super::{
        EnrichmentWorkerBudgets, SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV,
        SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV, SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV,
        SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV, WorkerBudgetSources, WorkerCountSource,
        configured_enrichment_worker_budgets, resolve_worker_budgets_from_sources,
    };
    use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
    use crate::test_support::process_state::enter_process_state;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn worker_budgets_default_to_one_for_missing_or_invalid_values() {
        assert_eq!(
            resolve_worker_budgets_from_sources(WorkerBudgetSources::default()),
            EnrichmentWorkerBudgets {
                summary_refresh: 1,
                embeddings: 1,
                clone_rebuild: 1,
            }
        );
        assert_eq!(
            resolve_worker_budgets_from_sources(WorkerBudgetSources {
                summary: WorkerCountSource::new(Some(""), None),
                embeddings: WorkerCountSource::new(Some("0"), None),
                clone_rebuild: WorkerCountSource::new(Some("-1"), None),
                legacy_embeddings: WorkerCountSource::new(Some("nope"), None),
                ..WorkerBudgetSources::default()
            }),
            EnrichmentWorkerBudgets {
                summary_refresh: 1,
                embeddings: 1,
                clone_rebuild: 1,
            }
        );
    }

    #[test]
    fn worker_budgets_respect_valid_values_and_cap_large_values() {
        assert_eq!(
            resolve_worker_budgets_from_sources(WorkerBudgetSources {
                summary: WorkerCountSource::new(Some("4"), None),
                embeddings: WorkerCountSource::new(Some("8"), None),
                clone_rebuild: WorkerCountSource::new(Some("999"), None),
                ..WorkerBudgetSources::default()
            }),
            EnrichmentWorkerBudgets {
                summary_refresh: 4,
                embeddings: 8,
                clone_rebuild: 32,
            }
        );
    }

    #[test]
    fn worker_budgets_use_legacy_override_for_embeddings_only() {
        assert_eq!(
            resolve_worker_budgets_from_sources(WorkerBudgetSources {
                legacy_embeddings: WorkerCountSource::new(Some("7"), None),
                ..WorkerBudgetSources::default()
            }),
            EnrichmentWorkerBudgets {
                summary_refresh: 1,
                embeddings: 7,
                clone_rebuild: 1,
            }
        );
    }

    #[test]
    fn worker_budgets_default_remote_summary_to_eight_and_embeddings_to_four() {
        assert_eq!(
            resolve_worker_budgets_from_sources(WorkerBudgetSources {
                summary_remote: true,
                embeddings_remote: true,
                ..WorkerBudgetSources::default()
            }),
            EnrichmentWorkerBudgets {
                summary_refresh: 8,
                embeddings: 4,
                clone_rebuild: 1,
            }
        );
    }

    #[test]
    fn configured_worker_budgets_prefer_env_over_repo_config() {
        let temp = tempdir().expect("temp dir");
        let _guard = enter_process_state(
            Some(temp.path()),
            &[
                (SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV, Some("2")),
                (SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV, Some("7")),
                (SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV, Some("3")),
            ],
        );
        assert_eq!(
            configured_enrichment_worker_budgets(),
            EnrichmentWorkerBudgets {
                summary_refresh: 2,
                embeddings: 7,
                clone_rebuild: 3,
            }
        );
    }

    #[test]
    fn configured_worker_budgets_read_repo_config_and_keep_legacy_fallback() {
        let temp = tempdir().expect("temp dir");
        let config_path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).expect("create config dir");
        }
        fs::write(
            &config_path,
            "[semantic_clones]\nsummary_workers = 2\nembedding_workers = 5\nclone_rebuild_workers = 3\nenrichment_workers = 9\n",
        )
        .expect("write semantic clones config");

        let _guard = enter_process_state(
            Some(temp.path()),
            &[
                (SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, None),
            ],
        );

        assert_eq!(
            configured_enrichment_worker_budgets(),
            EnrichmentWorkerBudgets {
                summary_refresh: 2,
                embeddings: 5,
                clone_rebuild: 3,
            }
        );
    }

    #[test]
    fn configured_worker_budgets_use_legacy_repo_value_for_embeddings_when_new_field_is_missing() {
        let temp = tempdir().expect("temp dir");
        let config_path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).expect("create config dir");
        }
        fs::write(
            &config_path,
            "[semantic_clones]\nsummary_workers = 2\nclone_rebuild_workers = 3\nenrichment_workers = 9\n",
        )
        .expect("write semantic clones config");

        let _guard = enter_process_state(
            Some(temp.path()),
            &[
                (SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, None),
            ],
        );

        assert_eq!(
            configured_enrichment_worker_budgets(),
            EnrichmentWorkerBudgets {
                summary_refresh: 2,
                embeddings: 9,
                clone_rebuild: 3,
            }
        );
    }

    #[test]
    fn configured_worker_budgets_promote_local_embeddings_when_code_and_summary_embeddings_are_active()
     {
        let temp = tempdir().expect("temp dir");
        let config_path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).expect("create config dir");
        }
        fs::write(
            &config_path,
            r#"[semantic_clones]

[semantic_clones.inference]
summary_generation = "summary_local"
code_embeddings = "local_code"
summary_embeddings = "local_summary"

[inference.profiles.summary_local]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_inference"
model = "ministral-3:3b"

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-code"

[inference.profiles.local_summary]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-summary"
"#,
        )
        .expect("write semantic clones config with local code and summary embeddings");

        let _guard = enter_process_state(
            Some(temp.path()),
            &[
                (SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, None),
            ],
        );

        assert_eq!(
            configured_enrichment_worker_budgets(),
            EnrichmentWorkerBudgets {
                summary_refresh: 1,
                embeddings: 2,
                clone_rebuild: 1,
            }
        );
    }

    #[test]
    fn configured_worker_budgets_keep_single_local_embedding_worker_when_only_code_embeddings_are_active()
     {
        let temp = tempdir().expect("temp dir");
        let config_path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).expect("create config dir");
        }
        fs::write(
            &config_path,
            r#"[semantic_clones]

[semantic_clones.inference]
code_embeddings = "local_code"

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-code"
"#,
        )
        .expect("write semantic clones config with local code embeddings only");

        let _guard = enter_process_state(
            Some(temp.path()),
            &[
                (SEMANTIC_CLONES_SUMMARY_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_EMBEDDING_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_CLONE_REBUILD_WORKER_COUNT_ENV, None),
                (SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, None),
            ],
        );

        assert_eq!(
            configured_enrichment_worker_budgets(),
            EnrichmentWorkerBudgets {
                summary_refresh: 1,
                embeddings: 1,
                clone_rebuild: 1,
            }
        );
    }
}
