//! Auto-commit strategy adapter.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::json;

use crate::adapters::agents::canonical_agent_key;
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::state::SessionState;
use crate::host::checkpoints::trailers::{
    AGENT_TRAILER_KEY, CHECKPOINT_TRAILER_KEY, METADATA_TASK_TRAILER_KEY, METADATA_TRAILER_KEY,
    SESSION_TRAILER_KEY, STRATEGY_TRAILER_KEY,
};
use crate::utils::paths;
use crate::utils::strings;

use super::manual_commit::ManualCommitStrategy;
use super::manual_commit::commit_files_to_metadata_branch;
use super::manual_commit::run_git;
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
        "Auto-commits code to active branch with metadata on bitloops/checkpoints/v1"
    }

    pub fn ensure_setup(&self) -> Result<()> {
        if run_git(
            &self.repo_root,
            &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
        )
        .is_ok()
        {
            return Ok(());
        }

        let head = run_git(&self.repo_root, &["rev-parse", "HEAD"])?;
        run_git(
            &self.repo_root,
            &[
                "update-ref",
                &format!("refs/heads/{}", paths::METADATA_BRANCH_NAME),
                &head,
            ],
        )?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
        let checkpoint_ids = list_checkpoint_ids(&self.repo_root)?;

        for checkpoint_id in checkpoint_ids {
            let session_count = checkpoint_session_count(&self.repo_root, &checkpoint_id);
            for idx in 0..session_count {
                let Some(session_id) = checkpoint_session_id(&self.repo_root, &checkpoint_id, idx)
                else {
                    continue;
                };

                let description =
                    checkpoint_session_description(&self.repo_root, &checkpoint_id, idx);
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
                    checkpoint_id: checkpoint_id.clone(),
                });
            }
        }

        Ok(sessions.into_values().collect())
    }

    pub fn get_session_context(&self, session_id: &str) -> String {
        let Ok(checkpoint_ids) = list_checkpoint_ids(&self.repo_root) else {
            return String::new();
        };

        let mut found: Option<(String, usize)> = None;
        for checkpoint_id in checkpoint_ids {
            let session_count = checkpoint_session_count(&self.repo_root, &checkpoint_id);
            for idx in 0..session_count {
                if checkpoint_session_id(&self.repo_root, &checkpoint_id, idx).as_deref()
                    == Some(session_id)
                {
                    found = Some((checkpoint_id.clone(), idx));
                }
            }
        }

        let Some((checkpoint_id, idx)) = found else {
            return String::new();
        };

        read_checkpoint_file(
            &self.repo_root,
            &checkpoint_id,
            idx,
            paths::CONTEXT_FILE_NAME,
        )
        .unwrap_or_default()
    }

    pub fn get_checkpoint_log(&self, checkpoint: &Checkpoint) -> Result<Vec<u8>> {
        let session_count = checkpoint_session_count(&self.repo_root, &checkpoint.checkpoint_id);
        if session_count == 0 {
            anyhow::bail!("checkpoint has no sessions");
        }

        let transcript = read_checkpoint_file(
            &self.repo_root,
            &checkpoint.checkpoint_id,
            session_count - 1,
            paths::TRANSCRIPT_FILE_NAME,
        )?;
        Ok(transcript.into_bytes())
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

    fn write_checkpoint_metadata_commit(&self, input: &MetadataCommitInput<'_>) -> Result<()> {
        let checkpoint_id = input.checkpoint_id;
        let (a, b) = checkpoint_id.split_at(2);
        let checkpoint_root = format!("{a}/{b}");
        let session_root = format!("{checkpoint_root}/0");

        let sessions = vec![json!({
            "metadata": format!("/{session_root}/{}", paths::METADATA_FILE_NAME),
            "transcript": format!("/{session_root}/{}", paths::TRANSCRIPT_FILE_NAME),
            "context": format!("/{session_root}/{}", paths::CONTEXT_FILE_NAME),
            "content_hash": format!("/{session_root}/{}", paths::CONTENT_HASH_FILE_NAME),
            "prompt": format!("/{session_root}/{}", paths::PROMPT_FILE_NAME),
        })];

        let branch = run_git(
            &self.repo_root,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .unwrap_or_default();

        let mut top_metadata = json!({
            "checkpoint_id": checkpoint_id,
            "cli_version": env!("CARGO_PKG_VERSION"),
            "strategy": "auto-commit",
            "branch": branch,
            "sessions": sessions,
            "checkpoints_count": 1,
            "files_touched": input.files_touched,
        });
        if branch.trim().is_empty() {
            top_metadata
                .as_object_mut()
                .expect("top metadata should be object")
                .remove("branch");
        }

        let created_at = now_rfc3339();
        let canonical_agent = canonical_agent_key(input.agent_type);
        let mut session_metadata = json!({
            "checkpoint_id": checkpoint_id,
            "session_id": input.session_id,
            "checkpoints_count": 1,
            "strategy": "auto-commit",
            "agent": canonical_agent.clone(),
            "created_at": created_at,
            "cli_version": env!("CARGO_PKG_VERSION"),
            "files_touched": input.files_touched,
            "branch": branch,
        });
        if canonical_agent.is_empty() {
            session_metadata
                .as_object_mut()
                .expect("session metadata should be object")
                .remove("agent");
        }
        if branch.trim().is_empty() {
            session_metadata
                .as_object_mut()
                .expect("session metadata should be object")
                .remove("branch");
        }

        let staging_dir = self
            .repo_root
            .join(paths::BITLOOPS_TMP_DIR)
            .join(format!("auto-commit-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&staging_dir).context("creating auto-commit staging directory")?;

        let top_metadata_disk = staging_dir.join("metadata.json");
        let session_metadata_disk = staging_dir.join("session-metadata.json");
        let transcript_disk = staging_dir.join("transcript.jsonl");
        let prompt_disk = staging_dir.join("prompt.txt");
        let context_disk = staging_dir.join("context.md");
        let content_hash_disk = staging_dir.join(paths::CONTENT_HASH_FILE_NAME);

        fs::write(
            &top_metadata_disk,
            serde_json::to_string_pretty(&top_metadata).context("serializing top metadata")?,
        )
        .context("writing top metadata")?;
        fs::write(
            &session_metadata_disk,
            serde_json::to_string_pretty(&session_metadata)
                .context("serializing session metadata")?,
        )
        .context("writing session metadata")?;
        fs::write(&transcript_disk, input.transcript).context("writing transcript")?;
        fs::write(&prompt_disk, input.prompt).context("writing prompt")?;
        fs::write(&context_disk, input.context).context("writing context")?;
        fs::write(
            &content_hash_disk,
            format!("sha256:{}", sha256_hex(input.transcript)),
        )
        .context("writing content hash")?;

        let mut files: Vec<(PathBuf, String)> = vec![
            (
                top_metadata_disk,
                format!("{checkpoint_root}/{}", paths::METADATA_FILE_NAME),
            ),
            (
                session_metadata_disk,
                format!("{session_root}/{}", paths::METADATA_FILE_NAME),
            ),
            (
                transcript_disk,
                format!("{session_root}/{}", paths::TRANSCRIPT_FILE_NAME),
            ),
            (
                prompt_disk,
                format!("{session_root}/{}", paths::PROMPT_FILE_NAME),
            ),
            (
                context_disk,
                format!("{session_root}/{}", paths::CONTEXT_FILE_NAME),
            ),
            (
                content_hash_disk,
                format!("{session_root}/{}", paths::CONTENT_HASH_FILE_NAME),
            ),
        ];

        let metadata_pointer = if input.is_task {
            let task_root = format!("{checkpoint_root}/tasks/{}", input.tool_use_id);
            let task_checkpoint_disk = staging_dir.join("task-checkpoint.json");
            fs::write(
                &task_checkpoint_disk,
                serde_json::to_string_pretty(&json!({
                    "session_id": input.session_id,
                    "tool_use_id": input.tool_use_id,
                    "agent_id": input.agent_id,
                }))
                .context("serializing task checkpoint metadata")?,
            )
            .context("writing task checkpoint metadata")?;
            files.push((
                task_checkpoint_disk,
                format!("{task_root}/{}", paths::CHECKPOINT_FILE_NAME),
            ));
            task_root
        } else {
            checkpoint_root.clone()
        };

        let mut message = format!(
            "Checkpoint: {checkpoint_id}\n\n{SESSION_TRAILER_KEY}: {}\n{STRATEGY_TRAILER_KEY}: auto-commit",
            input.session_id
        );
        if input.is_task {
            message.push_str(&format!(
                "\n{METADATA_TASK_TRAILER_KEY}: {metadata_pointer}"
            ));
        } else {
            message.push_str(&format!("\n{METADATA_TRAILER_KEY}: {metadata_pointer}"));
        }
        if !canonical_agent.is_empty() {
            message.push_str(&format!("\n{AGENT_TRAILER_KEY}: {canonical_agent}"));
        }

        let author_name = if input.author_name.trim().is_empty() {
            "Bitloops"
        } else {
            input.author_name
        };
        let author_email = if input.author_email.trim().is_empty() {
            "bitloops@localhost"
        } else {
            input.author_email
        };

        let result = commit_files_to_metadata_branch(
            &self.repo_root,
            &files,
            &message,
            author_name,
            author_email,
        );
        let _ = fs::remove_dir_all(&staging_dir);
        result
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
        let commit_message = format!("{subject}\n\n{CHECKPOINT_TRAILER_KEY}: {checkpoint_id}");
        run_git(
            &self.repo_root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                &commit_message,
            ],
        )
        .context("creating auto-commit on active branch")?;
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

fn sha256_hex(input: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(input);
    format!("{hash:x}")
}

fn list_checkpoint_ids(repo_root: &Path) -> Result<Vec<String>> {
    if run_git(
        repo_root,
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .is_err()
    {
        return Ok(vec![]);
    }

    let mut checkpoint_ids = vec![];
    let buckets = run_git(
        repo_root,
        &["ls-tree", "--name-only", paths::METADATA_BRANCH_NAME],
    )?;
    for bucket in buckets.lines() {
        if bucket.len() != 2 || !bucket.chars().all(|ch| ch.is_ascii_hexdigit()) {
            continue;
        }
        let children = run_git(
            repo_root,
            &[
                "ls-tree",
                "--name-only",
                &format!("{}:{bucket}", paths::METADATA_BRANCH_NAME),
            ],
        )?;
        for child in children.lines() {
            if !child.chars().all(|ch| ch.is_ascii_hexdigit()) {
                continue;
            }
            let checkpoint_id = format!("{bucket}{child}");
            if checkpoint_id.len() == 12 {
                checkpoint_ids.push(checkpoint_id);
            }
        }
    }
    checkpoint_ids.sort();
    Ok(checkpoint_ids)
}

fn checkpoint_session_count(repo_root: &Path, checkpoint_id: &str) -> usize {
    let Ok(summary) = read_checkpoint_summary(repo_root, checkpoint_id) else {
        return 0;
    };

    summary
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .map(std::vec::Vec::len)
        .unwrap_or(0)
}

fn checkpoint_session_id(
    repo_root: &Path,
    checkpoint_id: &str,
    session_idx: usize,
) -> Option<String> {
    let raw = read_checkpoint_file(
        repo_root,
        checkpoint_id,
        session_idx,
        paths::METADATA_FILE_NAME,
    )
    .ok()?;
    let json = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    json.get("session_id")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn checkpoint_session_description(
    repo_root: &Path,
    checkpoint_id: &str,
    session_idx: usize,
) -> String {
    let Ok(prompt) = read_checkpoint_file(
        repo_root,
        checkpoint_id,
        session_idx,
        paths::PROMPT_FILE_NAME,
    ) else {
        return NO_DESCRIPTION.to_string();
    };
    let first_line = prompt.lines().next().unwrap_or_default().trim().to_string();
    if first_line.is_empty() {
        NO_DESCRIPTION.to_string()
    } else {
        first_line
    }
}

fn read_checkpoint_summary(repo_root: &Path, checkpoint_id: &str) -> Result<serde_json::Value> {
    let summary_raw =
        read_checkpoint_root_file(repo_root, checkpoint_id, paths::METADATA_FILE_NAME)?;
    Ok(serde_json::from_str(&summary_raw)?)
}

fn read_checkpoint_root_file(repo_root: &Path, checkpoint_id: &str, name: &str) -> Result<String> {
    let (a, b) = checkpoint_id.split_at(2);
    let spec = format!("{}:{a}/{b}/{name}", paths::METADATA_BRANCH_NAME);
    run_git(repo_root, &["show", &spec])
}

fn read_checkpoint_file(
    repo_root: &Path,
    checkpoint_id: &str,
    session_idx: usize,
    name: &str,
) -> Result<String> {
    let (a, b) = checkpoint_id.split_at(2);
    let spec = format!(
        "{}:{a}/{b}/{session_idx}/{name}",
        paths::METADATA_BRANCH_NAME
    );
    run_git(repo_root, &["show", &spec])
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

        self.write_checkpoint_metadata_commit(&MetadataCommitInput {
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

        self.write_checkpoint_metadata_commit(&MetadataCommitInput {
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
}

#[cfg(test)]
#[path = "auto_commit_tests.rs"]
mod tests;
