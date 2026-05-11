use serde::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::daemon::types::EnrichmentQueueState;

#[cfg(test)]
pub(crate) const MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS: usize = 32;
#[cfg(test)]
pub(crate) const WORKPLANE_PENDING_COMPACTION_MIN_COUNT: u64 = 10_000;
pub(crate) const WORKPLANE_TERMINAL_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
pub(crate) const WORKPLANE_TERMINAL_ROW_LIMIT: u64 = 1_000;

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkplaneMailboxReadiness {
    pub blocked: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentJobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrichmentJobKind {
    SemanticSummaries {
        #[serde(alias = "inputs", deserialize_with = "deserialize_job_artefact_ids")]
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
    },
    SymbolEmbeddings {
        #[serde(alias = "inputs", deserialize_with = "deserialize_job_artefact_ids")]
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
        #[serde(default)]
        representation_kind: EmbeddingRepresentationKind,
    },
    CloneEdgesRebuild {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrichmentJob {
    pub id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub branch: String,
    pub status: EnrichmentJobStatus,
    pub attempts: u32,
    pub error: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub job: EnrichmentJobKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnrichmentControlState {
    pub version: u8,
    pub paused_semantic: bool,
    pub paused_embeddings: bool,
    pub active_branch_by_repo: BTreeMap<String, String>,
    pub jobs: Vec<EnrichmentJob>,
    pub retried_failed_jobs: u64,
    pub last_action: Option<String>,
    pub paused_reason: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone)]
pub struct EnrichmentControlResult {
    pub message: String,
    pub state: EnrichmentQueueState,
}

#[derive(Debug, Clone)]
pub struct EnrichmentJobTarget {
    pub(crate) config_root: PathBuf,
    pub(crate) repo_root: PathBuf,
    pub(crate) repo_id: Option<String>,
    pub(crate) init_session_id: Option<String>,
}

impl EnrichmentJobTarget {
    pub fn new(config_root: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            config_root,
            repo_root,
            repo_id: None,
            init_session_id: None,
        }
    }

    pub fn with_repo_id(mut self, repo_id: impl Into<String>) -> Self {
        self.repo_id = Some(repo_id.into());
        self
    }

    pub fn with_init_session_id(mut self, init_session_id: Option<String>) -> Self {
        self.init_session_id = init_session_id;
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FollowUpJob {
    RepoBackfillSummaries {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
    },
    RepoBackfillEmbeddings {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        representation_kind: EmbeddingRepresentationKind,
    },
    SymbolEmbeddings {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    },
    CloneEdgesRebuild {
        target: EnrichmentJobTarget,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct JobExecutionOutcome {
    pub error: Option<String>,
    pub follow_ups: Vec<FollowUpJob>,
}

impl JobExecutionOutcome {
    pub(crate) fn ok() -> Self {
        Self {
            error: None,
            follow_ups: Vec::new(),
        }
    }

    pub(crate) fn failed(err: anyhow::Error) -> Self {
        Self {
            error: Some(format!("{err:#}")),
            follow_ups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PersistedEnrichmentJobInput {
    ArtefactId(String),
    LegacyInput(Box<semantic_features::SemanticFeatureInput>),
}

fn deserialize_job_artefact_ids<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let inputs = Vec::<PersistedEnrichmentJobInput>::deserialize(deserializer)?;
    Ok(inputs
        .into_iter()
        .map(|input| match input {
            PersistedEnrichmentJobInput::ArtefactId(artefact_id) => artefact_id,
            PersistedEnrichmentJobInput::LegacyInput(input) => input.artefact_id.clone(),
        })
        .collect())
}
