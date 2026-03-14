use crate::engine::paths;
use crate::engine::strategy::manual_commit::{
    get_checkpoint_author, list_committed, parse_checkpoint_id, read_commit_checkpoint_mappings,
    read_committed, read_latest_session_content, read_session_content_by_id, run_git,
};
use crate::engine::trailers::{CHECKPOINT_TRAILER_KEY, is_valid_checkpoint_id};
use anyhow::{Result, anyhow, bail};
use clap::Args;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

const MAX_INTENT_DISPLAY_LENGTH: usize = 80;
const MAX_MESSAGE_DISPLAY_LENGTH: usize = 80;
const MAX_PROMPT_DISPLAY_LENGTH: usize = 60;
const CHECKPOINT_ID_DISPLAY_LENGTH: usize = 12;

#[derive(Debug, Clone, Args, Default)]
pub struct ExplainArgs {
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub commit: Option<String>,
    #[arg(long, short = 'c')]
    pub checkpoint: Option<String>,
    #[arg(long)]
    pub no_pager: bool,
    #[arg(long, short = 's', conflicts_with_all = ["full", "raw_transcript"])]
    pub short: bool,
    #[arg(long, conflicts_with_all = ["short", "raw_transcript"])]
    pub full: bool,
    #[arg(long = "raw-transcript", conflicts_with_all = ["short", "full", "generate"], requires = "checkpoint")]
    pub raw_transcript: bool,
    #[arg(long, requires = "checkpoint", conflicts_with = "raw_transcript")]
    pub generate: bool,
    #[arg(long, requires = "generate")]
    pub force: bool,
    #[arg(long = "search-all")]
    pub search_all: bool,
}

pub async fn run(args: ExplainArgs) -> Result<()> {
    let opts = ExplainExecutionOptions {
        no_pager: args.no_pager,
        verbose: !args.short,
        full: args.full,
        raw_transcript: args.raw_transcript,
        generate: args.generate,
        force: args.force,
        search_all: args.search_all,
    };

    let session = args.session.as_deref().unwrap_or("");
    let commit = args.commit.as_deref().unwrap_or("");
    let checkpoint = args.checkpoint.as_deref().unwrap_or("");

    match run_explain(session, commit, checkpoint, &opts)? {
        ExplainRoute::BranchList { session_filter } => {
            if let Some(filter) = session_filter {
                print!(
                    "{}",
                    run_explain_branch_with_filter(&filter, opts.no_pager)?
                );
            } else {
                print!("{}", run_explain_branch_default(opts.no_pager)?);
            }
        }
        ExplainRoute::Commit { commit_ref } => {
            print!(
                "{}",
                run_explain_commit(
                    &commit_ref,
                    opts.no_pager,
                    opts.verbose,
                    opts.full,
                    opts.search_all,
                )?
            );
        }
        ExplainRoute::Checkpoint { checkpoint_id } => {
            print!("{}", run_explain_checkpoint(&checkpoint_id, &opts)?);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplainRoute {
    BranchList { session_filter: Option<String> },
    Commit { commit_ref: String },
    Checkpoint { checkpoint_id: String },
}

#[derive(Debug, Clone, Default)]
pub struct ExplainExecutionOptions {
    pub no_pager: bool,
    pub verbose: bool,
    pub full: bool,
    pub raw_transcript: bool,
    pub generate: bool,
    pub force: bool,
    pub search_all: bool,
}

pub fn run_explain(
    session_id: &str,
    commit_ref: &str,
    checkpoint_id: &str,
    _opts: &ExplainExecutionOptions,
) -> Result<ExplainRoute> {
    let mut count = 0;
    if !commit_ref.is_empty() {
        count += 1;
    }
    if !checkpoint_id.is_empty() {
        count += 1;
    }

    if !session_id.is_empty() && count > 0 {
        bail!("cannot specify multiple of --session, --commit, --checkpoint")
    }
    if count > 1 {
        bail!("cannot specify multiple of --session, --commit, --checkpoint")
    }

    if !commit_ref.is_empty() {
        return Ok(ExplainRoute::Commit {
            commit_ref: commit_ref.to_string(),
        });
    }

    if !checkpoint_id.is_empty() {
        return Ok(ExplainRoute::Checkpoint {
            checkpoint_id: checkpoint_id.to_string(),
        });
    }

    if session_id.is_empty() {
        Ok(ExplainRoute::BranchList {
            session_filter: None,
        })
    } else {
        Ok(ExplainRoute::BranchList {
            session_filter: Some(session_id.to_string()),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interaction {
    pub prompt: String,
    pub responses: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssociatedCommit {
    pub sha: String,
    pub short_sha: String,
    pub message: String,
    pub author: String,
    pub date: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointDetail {
    pub index: usize,
    pub short_id: String,
    pub timestamp: String,
    pub is_task_checkpoint: bool,
    pub message: String,
    pub interactions: Vec<Interaction>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionCheckpoint {
    pub checkpoint_id: String,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub description: String,
    pub strategy: String,
    pub start_time: String,
    pub checkpoints: Vec<SessionCheckpoint>,
}

pub trait SessionSource {
    fn get_additional_sessions(&self) -> Result<Vec<SessionInfo>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManualCommitStrategy;

pub fn new_manual_commit_strategy() -> ManualCommitStrategy {
    ManualCommitStrategy
}

impl SessionSource for ManualCommitStrategy {
    fn get_additional_sessions(&self) -> Result<Vec<SessionInfo>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SummaryDetails {
    pub intent: String,
    pub outcome: String,
    pub repo_learnings: Vec<String>,
    pub code_learnings: Vec<CodeLearning>,
    pub workflow_learnings: Vec<String>,
    pub friction: Vec<String>,
    pub open_items: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeLearning {
    pub path: String,
    pub line: usize,
    pub end_line: usize,
    pub finding: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointMetadata {
    pub checkpoint_id: String,
    pub session_id: String,
    pub created_at: String,
    pub files_touched: Vec<String>,
    pub checkpoints_count: usize,
    pub checkpoint_transcript_start: usize,
    pub has_token_usage: bool,
    pub token_input: u64,
    pub token_output: u64,
    pub summary: Option<SummaryDetails>,
    pub agent_type: AgentType,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionContent {
    pub metadata: CheckpointMetadata,
    pub prompts: String,
    pub transcript: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointSummary {
    pub checkpoint_id: String,
    pub checkpoints_count: usize,
    pub files_touched: Vec<String>,
    pub has_token_usage: bool,
    pub token_input: u64,
    pub token_output: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentType {
    #[default]
    ClaudeCode,
    Codex,
    Cursor,
    Gemini,
    OpenCode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewindPoint {
    pub id: String,
    pub message: String,
    pub date: String,
    pub checkpoint_id: String,
    pub session_id: String,
    pub session_prompt: String,
    pub is_logs_only: bool,
    pub is_task_checkpoint: bool,
    pub tool_use_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitNode {
    pub sha: String,
    pub message: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub trailers: HashMap<String, String>,
    pub files_changed: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointGroup {
    pub checkpoint_id: String,
    pub points: Vec<RewindPoint>,
}

const COMMIT_SCAN_LIMIT: usize = 500;

include!("explain/core.rs");
include!("explain/branch.rs");
include!("explain/commit.rs");
