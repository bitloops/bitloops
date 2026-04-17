use super::helpers::KnowledgeStubServer;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct QatRunConfig {
    pub binary_path: PathBuf,
    pub suite_root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepresentationKindCounts {
    pub code: usize,
    pub summary: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticCloneHistoricalTableSnapshot {
    pub artefacts_historical: usize,
    pub symbol_features: usize,
    pub symbol_semantics: usize,
    pub symbol_embeddings: usize,
    pub symbol_clone_edges: usize,
    pub commit_ingest_ledger: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticCloneCurrentTableSnapshot {
    pub artefacts_current: usize,
    pub symbol_features_current: usize,
    pub symbol_semantics_current: usize,
    pub symbol_embeddings_current: usize,
    pub symbol_clone_edges_current: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticCloneTableSnapshot {
    pub historical: SemanticCloneHistoricalTableSnapshot,
    pub current: SemanticCloneCurrentTableSnapshot,
    pub historical_representation_counts: RepresentationKindCounts,
    pub current_representation_counts: RepresentationKindCounts,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnrichmentStatusSnapshot {
    pub mode: String,
    pub pending_jobs: u64,
    pub pending_semantic_jobs: u64,
    pub pending_embedding_jobs: u64,
    pub pending_clone_edges_rebuild_jobs: u64,
    pub running_jobs: u64,
    pub running_semantic_jobs: u64,
    pub running_embedding_jobs: u64,
    pub running_clone_edges_rebuild_jobs: u64,
    pub failed_jobs: u64,
    pub failed_semantic_jobs: u64,
    pub failed_embedding_jobs: u64,
    pub failed_clone_edges_rebuild_jobs: u64,
    pub retried_failed_jobs: u64,
    pub last_action: Option<String>,
    pub paused_reason: Option<String>,
    pub persisted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticCloneProgressObservation {
    pub status_samples: usize,
    pub max_pending_embedding_jobs: u64,
    pub max_pending_clone_edges_rebuild_jobs: u64,
    pub embedding_pending_decreased: bool,
    pub first_code_embedding_count: usize,
    pub first_summary_embedding_count: usize,
    pub code_embeddings_appeared_before_drain: bool,
    pub summary_embeddings_appeared_before_drain: bool,
    pub embedding_activity_observed: bool,
    pub clone_edges_rebuild_observed: bool,
    pub parallel_progress_observed: bool,
}

#[derive(Debug, Default, cucumber::World)]
pub struct QatWorld {
    pub scenario_name: Option<String>,
    pub scenario_slug: Option<String>,
    pub flow_name: Option<String>,
    pub run_config: Option<Arc<QatRunConfig>>,
    pub run_dir: Option<PathBuf>,
    pub repo_dir: Option<PathBuf>,
    pub terminal_log_path: Option<PathBuf>,
    pub metadata_path: Option<PathBuf>,
    pub daemon_url: Option<String>,
    pub daemon_process: Option<ScenarioDaemonProcess>,
    pub daemon_runtime_state_path: Option<PathBuf>,
    pub daemon_stderr_log_path: Option<PathBuf>,
    pub last_command_stdout: Option<String>,
    pub last_command_exit_code: Option<i32>,
    pub last_query_result_count: Option<usize>,
    pub captured_commit_shas: Vec<String>,
    pub expected_commit_shas: Vec<String>,
    pub expected_paths: Vec<String>,
    pub pre_rewrite_shas: Vec<String>,
    pub post_rewrite_shas: Vec<String>,
    pub rewrite_new_shas: Vec<String>,
    pub completed_ledger_shas_snapshot: Vec<String>,
    pub completed_ledger_count_snapshot: Option<usize>,
    pub artefacts_current_count_snapshot: Option<usize>,
    pub semantic_clones_fallback_active: bool,
    pub semantic_clone_table_snapshot: Option<SemanticCloneTableSnapshot>,
    pub last_enrichment_status_snapshot: Option<EnrichmentStatusSnapshot>,
    pub semantic_clone_progress_observation: Option<SemanticCloneProgressObservation>,
    pub current_file_state_content_id_snapshots: HashMap<String, Option<String>>,
    pub knowledge_items_by_url: HashMap<String, String>,
    pub knowledge_versions_by_ref: HashMap<String, usize>,
    pub knowledge_fixture_urls: HashMap<String, String>,
    pub knowledge_stub_server: Option<KnowledgeStubServer>,
    pub last_knowledge_add_had_commit_association: Option<bool>,
    pub last_test_harness_target_generation: Option<u64>,
    pub last_task_id: Option<String>,
    pub agent_name: Option<String>,
}

pub struct ScenarioDaemonProcess {
    pub child: Child,
    pub requested_port: String,
    pub stderr_log_path: PathBuf,
    pub runtime_state_path: PathBuf,
}

impl std::fmt::Debug for ScenarioDaemonProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScenarioDaemonProcess")
            .field("pid", &self.child.id())
            .field("requested_port", &self.requested_port)
            .field("stderr_log_path", &self.stderr_log_path)
            .field("runtime_state_path", &self.runtime_state_path)
            .finish()
    }
}

impl QatWorld {
    pub fn reset(&mut self) {
        self.flow_name = None;
        self.run_dir = None;
        self.repo_dir = None;
        self.terminal_log_path = None;
        self.metadata_path = None;
        self.daemon_url = None;
        self.daemon_process = None;
        self.daemon_runtime_state_path = None;
        self.daemon_stderr_log_path = None;
        self.last_command_stdout = None;
        self.last_command_exit_code = None;
        self.last_query_result_count = None;
        self.captured_commit_shas = Vec::new();
        self.expected_commit_shas = Vec::new();
        self.expected_paths = Vec::new();
        self.pre_rewrite_shas = Vec::new();
        self.post_rewrite_shas = Vec::new();
        self.rewrite_new_shas = Vec::new();
        self.completed_ledger_shas_snapshot = Vec::new();
        self.completed_ledger_count_snapshot = None;
        self.artefacts_current_count_snapshot = None;
        self.semantic_clones_fallback_active = false;
        self.semantic_clone_table_snapshot = None;
        self.last_enrichment_status_snapshot = None;
        self.semantic_clone_progress_observation = None;
        self.current_file_state_content_id_snapshots = HashMap::new();
        self.knowledge_items_by_url = HashMap::new();
        self.knowledge_versions_by_ref = HashMap::new();
        self.knowledge_fixture_urls = HashMap::new();
        self.knowledge_stub_server = None;
        self.last_knowledge_add_had_commit_association = None;
        self.last_test_harness_target_generation = None;
        self.last_task_id = None;
        self.agent_name = None;
    }

    pub fn prepare(
        &mut self,
        config: Arc<QatRunConfig>,
        scenario_name: &str,
        scenario_slug: String,
    ) {
        self.run_config = Some(config);
        self.scenario_name = Some(scenario_name.to_string());
        self.scenario_slug = Some(scenario_slug);
        self.reset();
    }

    pub fn run_config(&self) -> &Arc<QatRunConfig> {
        self.run_config
            .as_ref()
            .expect("qat run config should be initialized before step execution")
    }

    pub fn run_dir(&self) -> &Path {
        self.run_dir
            .as_deref()
            .expect("qat run directory should be initialized by CleanStart")
    }

    pub fn repo_dir(&self) -> &Path {
        self.repo_dir
            .as_deref()
            .expect("qat repo directory should be initialized by CleanStart")
    }

    pub fn terminal_log_path(&self) -> &Path {
        self.terminal_log_path
            .as_deref()
            .expect("qat terminal log should be initialized by CleanStart")
    }

    pub fn metadata_path(&self) -> &Path {
        self.metadata_path
            .as_deref()
            .expect("qat run metadata should be initialized by CleanStart")
    }
}
