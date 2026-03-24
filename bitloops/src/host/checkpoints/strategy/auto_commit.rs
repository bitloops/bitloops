//! Auto-commit strategy adapter.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::adapters::agents::canonical_agent_key;
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::state::SessionState;
use crate::host::checkpoints::strategy::manual_commit::{
    CommittedMetadata, WriteCommittedOptions, insert_commit_checkpoint_mapping,
    persist_committed_checkpoint_db_and_blobs, redact_bytes, redact_jsonl_bytes_with_fallback,
    redact_text, run_git,
};
use crate::utils::paths;
use crate::utils::strings;

use super::manual_commit::ManualCommitStrategy;
use super::{StepContext, Strategy, TaskStepContext};

pub const NO_DESCRIPTION: &str = "No description";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkpoint {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub description: String,
    pub checkpoints: Vec<Checkpoint>,
}

pub trait SessionInitializer {
    fn initialize_session(
        &self,
        session_id: &str,
        agent_type: &str,
        transcript_path: &str,
        user_prompt: &str,
    ) -> Result<()>;
}

pub struct AutoCommitStrategy {
    repo_root: PathBuf,
    inner: ManualCommitStrategy,
}

struct MetadataCommitInput<'a> {
    checkpoint_id: &'a str,
    session_id: &'a str,
    agent_type: &'a str,
    transcript: &'a [u8],
    prompt: &'a str,
    context: &'a str,
    files_touched: &'a [String],
    author_name: &'a str,
    author_email: &'a str,
    is_task: bool,
    tool_use_id: &'a str,
    agent_id: &'a str,
}

impl AutoCommitStrategy {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        let repo_root = repo_root.into();
        Self {
            inner: ManualCommitStrategy::new(repo_root.clone()),
            repo_root,
        }
    }

    pub fn description(&self) -> &'static str {
        "Auto-commits code to active branch with checkpoint metadata in DB/blob storage"
    }

    pub fn ensure_setup(&self) -> Result<()> {
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        use super::manual_commit::{list_committed, read_session_content};
        let mut sessions: BTreeMap<String, Session> = BTreeMap::new();

        let committed = match list_committed(&self.repo_root) {
            Ok(v) => v,
            Err(_) => return Ok(vec![]),
        };
        for info in committed {
            let session_id = if info.session_id.is_empty() {
                info.checkpoint_id.clone()
            } else {
                info.session_id.clone()
            };

            let description = read_session_content(&self.repo_root, &info.checkpoint_id, 0)
                .ok()
                .and_then(|c| {
                    c.prompts
                        .lines()
                        .find(|l| !l.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| NO_DESCRIPTION.to_string());

            let entry = sessions
                .entry(session_id.clone())
                .or_insert_with(|| Session {
                    id: session_id,
                    description: NO_DESCRIPTION.to_string(),
                    checkpoints: vec![],
                });
            if description != NO_DESCRIPTION {
                entry.description = description;
            }
            entry.checkpoints.push(Checkpoint {
                checkpoint_id: info.checkpoint_id.clone(),
            });
        }

        Ok(sessions.into_values().collect())
    }

    pub fn get_session_context(&self, session_id: &str) -> String {
        use super::manual_commit::{list_committed, read_session_content_by_id};
        let Ok(committed) = list_committed(&self.repo_root) else {
            return String::new();
        };

        for info in committed.iter().rev() {
            if let Ok(content) =
                read_session_content_by_id(&self.repo_root, &info.checkpoint_id, session_id)
            {
                return content.context;
            }
        }
        String::new()
    }

    pub fn get_checkpoint_log(&self, checkpoint: &Checkpoint) -> Result<Vec<u8>> {
        use super::manual_commit::read_latest_session_content;
        let content = read_latest_session_content(&self.repo_root, &checkpoint.checkpoint_id)?;
        Ok(content.transcript.into_bytes())
    }

    fn has_worktree_changes(&self, files: &[String]) -> bool {
        if files.is_empty() {
            return false;
        }

        let mut args: Vec<String> = vec![
            "status".to_string(),
            "--porcelain=v1".to_string(),
            "--untracked-files=all".to_string(),
            "--".to_string(),
        ];
        for file in files {
            if !file.trim().is_empty() {
                args.push(file.clone());
            }
        }
        if args.len() == 4 {
            return false;
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_git(&self.repo_root, &refs)
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false)
    }

    fn write_checkpoint_to_db(&self, input: &MetadataCommitInput<'_>) -> Result<()> {
        let checkpoint_id = input.checkpoint_id;
        let canonical_agent = canonical_agent_key(input.agent_type);
        let branch = run_git(
            &self.repo_root,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .unwrap_or_default();

        let redacted_transcript = redact_jsonl_bytes_with_fallback(input.transcript);
        let redacted_prompts = redact_text(input.prompt);
        let redacted_context = redact_bytes(input.context.as_bytes());

        let session_meta = CommittedMetadata {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: input.session_id.to_string(),
            checkpoints_count: 1,
            strategy: "auto-commit".to_string(),
            agent: canonical_agent,
            created_at: now_rfc3339(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            turn_id: String::new(),
            files_touched: input.files_touched.to_vec(),
            is_task: input.is_task,
            tool_use_id: input.tool_use_id.to_string(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            transcript_lines_at_start: 0,
            branch: branch.trim().to_string(),
            summary: None,
            token_usage: None,
            initial_attribution: None,
            transcript_path: String::new(),
        };

        let opts = WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: input.session_id.to_string(),
            strategy: "auto-commit".to_string(),
            agent: session_meta.agent.clone(),
            transcript: input.transcript.to_vec(),
            prompts: None,
            context: Some(input.context.as_bytes().to_vec()),
            checkpoints_count: 1,
            files_touched: input.files_touched.to_vec(),
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: input.author_name.to_string(),
            author_email: input.author_email.to_string(),
            summary: None,
            is_task: input.is_task,
            tool_use_id: input.tool_use_id.to_string(),
            agent_id: input.agent_id.to_string(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        };

        persist_committed_checkpoint_db_and_blobs(
            &self.repo_root,
            &opts,
            &session_meta,
            &redacted_transcript,
            &redacted_prompts,
            &redacted_context,
        )
    }

    fn commit_code_to_active_branch(&self, ctx: &StepContext, checkpoint_id: &str) -> Result<bool> {
        let mut add_args = vec!["add".to_string(), "--".to_string()];
        for path in ctx.modified_files.iter().chain(ctx.new_files.iter()) {
            if !path.trim().is_empty() {
                add_args.push(path.clone());
            }
        }
        if add_args.len() > 2 {
            let add_refs: Vec<&str> = add_args.iter().map(String::as_str).collect();
            run_git(&self.repo_root, &add_refs).context("staging modified/new files")?;
        }

        let mut remove_args = vec!["add".to_string(), "-u".to_string(), "--".to_string()];
        for path in &ctx.deleted_files {
            if !path.trim().is_empty() {
                remove_args.push(path.clone());
            }
        }
        if remove_args.len() > 3 {
            let remove_refs: Vec<&str> = remove_args.iter().map(String::as_str).collect();
            run_git(&self.repo_root, &remove_refs).context("staging deleted files")?;
        }

        let staged = run_git(&self.repo_root, &["diff", "--cached", "--name-only"])
            .context("checking staged files before auto-commit")?;
        if staged.trim().is_empty() {
            return Ok(false);
        }

        let subject = if ctx.commit_message.trim().is_empty() {
            "Bitloops checkpoint".to_string()
        } else {
            ctx.commit_message.trim().to_string()
        };
        run_git(
            &self.repo_root,
            &["-c", "core.hooksPath=/dev/null", "commit", "-m", &subject],
        )
        .context("creating auto-commit on active branch")?;

        let head_sha = run_git(&self.repo_root, &["rev-parse", "HEAD"])
            .context("reading HEAD after auto-commit")?;
        insert_commit_checkpoint_mapping(&self.repo_root, head_sha.trim(), checkpoint_id)
            .context("recording checkpoint mapping in DB")?;

        Ok(true)
    }
}

impl SessionInitializer for AutoCommitStrategy {
    fn initialize_session(
        &self,
        session_id: &str,
        agent_type: &str,
        transcript_path: &str,
        user_prompt: &str,
    ) -> Result<()> {
        let backend = create_session_backend_or_local(&self.repo_root);

        if let Some(mut existing) = backend.load_session(session_id)? {
            existing.last_interaction_time = Some(now_string());
            if existing.first_prompt.is_empty() && !user_prompt.is_empty() {
                existing.first_prompt = truncate_prompt(user_prompt);
            }
            backend.save_session(&existing)?;
            return Ok(());
        }

        let base_commit = run_git(&self.repo_root, &["rev-parse", "HEAD"]).unwrap_or_default();
        let now = now_string();

        let state = SessionState {
            session_id: session_id.to_string(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            base_commit,
            started_at: now.clone(),
            last_interaction_time: Some(now),
            step_count: 0,
            checkpoint_transcript_start: 0,
            files_touched: vec![],
            agent_type: canonical_agent_key(agent_type),
            transcript_path: transcript_path.to_string(),
            first_prompt: truncate_prompt(user_prompt),
            ..Default::default()
        };
        backend.save_session(&state)?;
        Ok(())
    }
}

fn truncate_prompt(prompt: &str) -> String {
    strings::truncate_runes(&strings::collapse_whitespace(prompt), 100, "...")
}

fn now_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();
    let (y, mo, d, h, mi, s) = unix_to_ymdhms(secs);
    if nanos == 0 {
        return format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");
    }
    let mut frac = format!("{nanos:09}");
    while frac.ends_with('0') {
        frac.pop();
    }
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{frac}Z")
}

fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let days = hours / 24;
    let mut year = 1970u64;
    let mut remaining = days;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if remaining < diy {
            break;
        }
        remaining -= diy;
        year += 1;
    }
    let months = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for (idx, mut dim) in months.into_iter().enumerate() {
        if idx == 1 && is_leap(year) {
            dim = 29;
        }
        if remaining < dim {
            break;
        }
        remaining -= dim;
        month += 1;
    }
    let day = remaining + 1;
    (year, month, day, h, mi, s)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn generate_checkpoint_id() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    id[..12].to_string()
}

fn read_optional_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn read_optional_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_default()
}

fn merge_files_touched(
    modified: &[String],
    new_files: &[String],
    deleted: &[String],
) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for file in modified
        .iter()
        .chain(new_files.iter())
        .chain(deleted.iter())
    {
        if !file.trim().is_empty() {
            seen.insert(file.clone());
        }
    }
    seen.into_iter().collect()
}

impl Strategy for AutoCommitStrategy {
    fn name(&self) -> &str {
        "auto-commit"
    }

    fn save_step(&self, ctx: &StepContext) -> Result<()> {
        let files_touched =
            merge_files_touched(&ctx.modified_files, &ctx.new_files, &ctx.deleted_files);
        if !self.has_worktree_changes(&files_touched) {
            return Ok(());
        }

        let metadata_dir_abs = if !ctx.metadata_dir_abs.trim().is_empty() {
            PathBuf::from(&ctx.metadata_dir_abs)
        } else if !ctx.metadata_dir.trim().is_empty() {
            self.repo_root.join(&ctx.metadata_dir)
        } else {
            self.repo_root
                .join(paths::session_metadata_dir_from_session_id(&ctx.session_id))
        };

        let transcript = {
            let from_metadata = metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME);
            if from_metadata.exists() {
                read_optional_bytes(&from_metadata)
            } else if !ctx.transcript_path.trim().is_empty() {
                read_optional_bytes(Path::new(&ctx.transcript_path))
            } else {
                vec![]
            }
        };
        let prompt = read_optional_file(&metadata_dir_abs.join(paths::PROMPT_FILE_NAME));
        let context = read_optional_file(&metadata_dir_abs.join(paths::CONTEXT_FILE_NAME));
        let checkpoint_id = generate_checkpoint_id();

        if !self.commit_code_to_active_branch(ctx, &checkpoint_id)? {
            return Ok(());
        }

        self.write_checkpoint_to_db(&MetadataCommitInput {
            checkpoint_id: &checkpoint_id,
            session_id: &ctx.session_id,
            agent_type: &ctx.agent_type,
            transcript: &transcript,
            prompt: &prompt,
            context: &context,
            files_touched: &files_touched,
            author_name: &ctx.author_name,
            author_email: &ctx.author_email,
            is_task: false,
            tool_use_id: "",
            agent_id: "",
        })
    }

    fn save_task_step(&self, ctx: &TaskStepContext) -> Result<()> {
        let files_touched =
            merge_files_touched(&ctx.modified_files, &ctx.new_files, &ctx.deleted_files);
        let transcript = if !ctx.transcript_path.trim().is_empty() {
            read_optional_bytes(Path::new(&ctx.transcript_path))
        } else {
            vec![]
        };
        let checkpoint_id = generate_checkpoint_id();

        self.write_checkpoint_to_db(&MetadataCommitInput {
            checkpoint_id: &checkpoint_id,
            session_id: &ctx.session_id,
            agent_type: &ctx.agent_type,
            transcript: &transcript,
            prompt: "",
            context: "",
            files_touched: &files_touched,
            author_name: &ctx.author_name,
            author_email: &ctx.author_email,
            is_task: true,
            tool_use_id: &ctx.tool_use_id,
            agent_id: &ctx.agent_id,
        })
    }

    fn prepare_commit_msg(&self, commit_msg_file: &Path, source: Option<&str>) -> Result<()> {
        self.inner.prepare_commit_msg(commit_msg_file, source)
    }

    fn commit_msg(&self, commit_msg_file: &Path) -> Result<()> {
        self.inner.commit_msg(commit_msg_file)
    }

    fn post_commit(&self) -> Result<()> {
        self.inner.post_commit()
    }

    fn pre_push(&self, remote: &str) -> Result<()> {
        self.inner.pre_push(remote)
    }

    fn post_checkout(
        &self,
        previous_head: &str,
        new_head: &str,
        is_branch_checkout: bool,
    ) -> Result<()> {
        self.inner
            .post_checkout(previous_head, new_head, is_branch_checkout)
    }
}

#[cfg(test)]
#[path = "auto_commit_tests.rs"]
mod tests;
