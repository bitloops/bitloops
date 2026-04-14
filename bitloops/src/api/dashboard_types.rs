use async_graphql::SimpleObject;

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardTokenUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) api_call_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, SimpleObject)]
pub(crate) struct DashboardCommitFileDiff {
    pub(crate) filepath: String,
    pub(crate) additions_count: u64,
    pub(crate) deletions_count: u64,
    pub(crate) change_kind: Option<String>,
    pub(crate) copied_from_path: Option<String>,
    pub(crate) copied_from_blob_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCommit {
    pub(crate) sha: String,
    pub(crate) parents: Vec<String>,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
    pub(crate) timestamp: i64,
    pub(crate) message: String,
    pub(crate) files_touched: Vec<DashboardCommitFileDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCheckpoint {
    pub(crate) checkpoint_id: String,
    pub(crate) strategy: String,
    pub(crate) branch: String,
    pub(crate) checkpoints_count: u32,
    pub(crate) files_touched: Vec<DashboardCommitFileDiff>,
    pub(crate) session_count: usize,
    pub(crate) token_usage: Option<DashboardTokenUsage>,
    pub(crate) session_id: String,
    pub(crate) agents: Vec<String>,
    pub(crate) first_prompt_preview: String,
    pub(crate) created_at: String,
    pub(crate) is_task: bool,
    pub(crate) tool_use_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCommitRow {
    pub(crate) commit: DashboardCommit,
    pub(crate) checkpoint: DashboardCheckpoint,
    pub(crate) checkpoints: Vec<DashboardCheckpoint>,
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub(crate) struct DashboardKpis {
    pub(crate) total_commits: usize,
    pub(crate) total_checkpoints: usize,
    pub(crate) total_agents: usize,
    pub(crate) total_sessions: usize,
    pub(crate) files_touched_count: usize,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) api_call_count: u64,
    pub(crate) average_tokens_per_checkpoint: f64,
    pub(crate) average_sessions_per_checkpoint: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardBranchSummary {
    pub(crate) branch: String,
    pub(crate) checkpoint_commits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardUser {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardAgent {
    pub(crate) key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardRepository {
    pub(crate) repo_id: String,
    pub(crate) identity: String,
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) organization: String,
    pub(crate) default_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCheckpointSessionDetail {
    pub(crate) session_index: usize,
    pub(crate) session_id: String,
    pub(crate) agent: String,
    pub(crate) created_at: String,
    pub(crate) is_task: bool,
    pub(crate) tool_use_id: String,
    pub(crate) metadata_json: String,
    pub(crate) transcript_jsonl: String,
    pub(crate) prompts_text: String,
    pub(crate) context_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCheckpointDetail {
    pub(crate) checkpoint_id: String,
    pub(crate) strategy: String,
    pub(crate) branch: String,
    pub(crate) checkpoints_count: u32,
    pub(crate) files_touched: Vec<DashboardCommitFileDiff>,
    pub(crate) session_count: usize,
    pub(crate) token_usage: Option<DashboardTokenUsage>,
    pub(crate) sessions: Vec<DashboardCheckpointSessionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardBundleVersion {
    pub(crate) current_version: Option<String>,
    pub(crate) latest_applicable_version: Option<String>,
    pub(crate) install_available: bool,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardFetchBundleResult {
    pub(crate) installed_version: String,
    pub(crate) bundle_dir: String,
    pub(crate) status: String,
    pub(crate) checksum_verified: bool,
}
