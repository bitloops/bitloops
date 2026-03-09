use anyhow::{Result, anyhow};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use crate::engine::agent::TranscriptPositionProvider;
use crate::engine::session::backend::SessionBackend;
use crate::engine::session::local_backend::LocalFileBackend;
use crate::engine::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};

pub mod adapters;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEventType {
    SessionStart,
    TurnStart,
    TurnEnd,
    Compaction,
    SessionEnd,
    SubagentStart,
    SubagentEnd,
    Unknown(i32),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub event_type: Option<LifecycleEventType>,
    pub session_id: String,
    pub session_ref: String,
    pub prompt: String,
    pub tool_use_id: String,
    pub subagent_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrePromptState {
    pub transcript_offset: usize,
}

pub trait LifecycleAgentAdapter: Send + Sync {
    fn agent_name(&self) -> &'static str;
    fn parse_hook_event(
        &self,
        _hook_name: &str,
        _stdin: &mut dyn Read,
    ) -> Result<Option<LifecycleEvent>>;
    fn hook_names(&self) -> Vec<&'static str>;
    fn format_resume_command(&self, _session_id: &str) -> String;

    /// When present, used by handle_lifecycle_turn_end to extract prompts, summary, and modified files.
    fn as_transcript_analyzer(&self) -> Option<&dyn crate::engine::agent::TranscriptAnalyzer> {
        None
    }

    /// When present, used by handle_lifecycle_turn_end to include token usage in the saved step.
    fn as_token_calculator(&self) -> Option<&dyn crate::engine::agent::TokenCalculator> {
        None
    }
}

pub fn dispatch_lifecycle_event(
    agent: Option<&dyn LifecycleAgentAdapter>,
    event: Option<&LifecycleEvent>,
) -> Result<()> {
    let Some(agent) = agent else {
        return Err(anyhow!("agent is required"));
    };

    let Some(event) = event else {
        return Err(anyhow!("event is required"));
    };

    match event.event_type.as_ref() {
        Some(LifecycleEventType::SessionStart) => handle_lifecycle_session_start(agent, event),
        Some(LifecycleEventType::TurnStart) => handle_lifecycle_turn_start(agent, event),
        Some(LifecycleEventType::TurnEnd) => handle_lifecycle_turn_end(agent, event),
        Some(LifecycleEventType::Compaction) => handle_lifecycle_compaction(agent, event),
        Some(LifecycleEventType::SessionEnd) => handle_lifecycle_session_end(agent, event),
        Some(LifecycleEventType::SubagentStart) => handle_lifecycle_subagent_start(agent, event),
        Some(LifecycleEventType::SubagentEnd) => handle_lifecycle_subagent_end(agent, event),
        Some(LifecycleEventType::Unknown(_)) | None => Err(anyhow!("unknown lifecycle event type")),
    }
}

pub fn handle_lifecycle_session_start(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if event.session_id.is_empty() {
        return Err(anyhow!("no session_id in SessionStart event"));
    }
    Ok(())
}

pub fn handle_lifecycle_turn_start(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if event.session_id.is_empty() {
        return Err(anyhow!("no session_id in TurnStart event"));
    }
    Ok(())
}

/// Returns (modified, new_files, deleted) relative to repo_root. Used by handle_lifecycle_turn_end.
fn detect_file_changes_for_turn_end(
    repo_root: &Path,
    previously_untracked: Option<&[String]>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    use std::collections::{BTreeSet, HashSet};
    use std::process::Command;

    let output = match Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo_root)
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return (Vec::new(), Vec::new(), Vec::new()),
    };

    let pre: HashSet<&str> = previously_untracked
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .collect();
    let mut modified = BTreeSet::new();
    let mut new_files = BTreeSet::new();
    let mut deleted = BTreeSet::new();

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let mut path = line[3..].trim().to_string();
        if let Some(idx) = path.rfind(" -> ") {
            path = path[idx + 4..].to_string();
        }
        if path.is_empty()
            || path.ends_with('/')
            || crate::engine::paths::is_infrastructure_path(&path)
        {
            continue;
        }
        if status == "??" {
            if previously_untracked.is_none() || !pre.contains(path.as_str()) {
                new_files.insert(path);
            }
            continue;
        }
        let x = status.as_bytes().first().copied().unwrap_or(b' ');
        let y = status.as_bytes().get(1).copied().unwrap_or(b' ');
        if x == b'D' || y == b'D' {
            deleted.insert(path);
            continue;
        }
        if x != b' ' || y != b' ' {
            modified.insert(path);
        }
    }

    let base = repo_root.to_string_lossy();
    let normalize = |paths: BTreeSet<String>| {
        paths
            .into_iter()
            .map(|p| crate::engine::paths::to_relative_path(&p, &base))
            .filter(|p| !p.is_empty() && !p.starts_with(".."))
            .collect::<Vec<_>>()
    };
    (
        normalize(modified),
        normalize(new_files),
        normalize(deleted),
    )
}

fn filter_and_normalize_paths_for_turn_end(files: &[String], repo_root: &Path) -> Vec<String> {
    let base = repo_root.to_string_lossy();
    files
        .iter()
        .map(|p| crate::engine::paths::to_relative_path(p, &base))
        .filter(|p| {
            !p.is_empty()
                && !p.starts_with("..")
                && !crate::engine::paths::is_infrastructure_path(p)
        })
        .collect()
}

fn merge_unique_for_turn_end(mut base: Vec<String>, extra: Vec<String>) -> Vec<String> {
    if extra.is_empty() {
        return base;
    }
    let mut seen: HashSet<String> = base.iter().cloned().collect();
    for path in extra {
        if seen.insert(path.clone()) {
            base.push(path);
        }
    }
    base
}

fn filter_to_uncommitted_files_for_turn_end(repo_root: &Path, files: Vec<String>) -> Vec<String> {
    if files.is_empty() {
        return files;
    }

    let head_probe = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo_root)
        .output();
    let Ok(head_probe) = head_probe else {
        return files;
    };
    if !head_probe.status.success() {
        return files;
    }

    let mut filtered = Vec::with_capacity(files.len());
    for rel_path in files {
        let head_spec = format!("HEAD:{rel_path}");
        let head_has_file = std::process::Command::new("git")
            .args(["cat-file", "-e", &head_spec])
            .current_dir(repo_root)
            .output();
        let Ok(head_has_file) = head_has_file else {
            filtered.push(rel_path);
            continue;
        };
        if !head_has_file.status.success() {
            filtered.push(rel_path);
            continue;
        }

        let working_content = std::fs::read(repo_root.join(&rel_path));
        let Ok(working_content) = working_content else {
            filtered.push(rel_path);
            continue;
        };

        let head_content = std::process::Command::new("git")
            .args(["show", &head_spec])
            .current_dir(repo_root)
            .output();
        let Ok(head_content) = head_content else {
            filtered.push(rel_path);
            continue;
        };
        if !head_content.status.success() {
            filtered.push(rel_path);
            continue;
        }

        if working_content != head_content.stdout {
            filtered.push(rel_path);
        }
    }

    filtered
}

pub fn handle_lifecycle_turn_end(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if event.session_ref.is_empty() {
        return Err(anyhow!("transcript file not specified"));
    }

    let transcript_path = Path::new(&event.session_ref);
    if !transcript_path.exists() {
        match transcript_path.parent() {
            Some(parent) if parent.exists() => {}
            _ => return Err(anyhow!("transcript file not found: {}", event.session_ref)),
        }
    }

    if crate::engine::git_operations::is_empty_repository()? {
        return Err(anyhow!("empty repository"));
    }

    let repo_root = crate::engine::paths::repo_root()?;
    let session_id = if event.session_id.is_empty() {
        "unknown-session"
    } else {
        &event.session_id
    };

    let meta_rel = crate::engine::paths::session_metadata_dir_from_session_id(session_id);
    let meta_dir_abs = repo_root.join(&meta_rel);
    std::fs::create_dir_all(&meta_dir_abs)
        .map_err(|e| anyhow!("failed to create session directory: {e}"))?;

    let transcript_data =
        std::fs::read(&event.session_ref).map_err(|e| anyhow!("failed to read transcript: {e}"))?;
    let log_path = meta_dir_abs.join(crate::engine::paths::TRANSCRIPT_FILE_NAME);
    std::fs::write(&log_path, &transcript_data)
        .map_err(|e| anyhow!("failed to write transcript: {e}"))?;

    let transcript_ref_canon = Path::new(&event.session_ref)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&event.session_ref).to_path_buf());
    let transcript_ref_str = transcript_ref_canon.to_string_lossy().to_string();

    let backend = LocalFileBackend::new(&repo_root);
    let pre_prompt = backend.load_pre_prompt(session_id).ok().flatten();
    let lifecycle_pre = pre_prompt.as_ref().map(|p| PrePromptState {
        transcript_offset: p.transcript_offset as usize,
    });
    let transcript_offset = resolve_transcript_offset(lifecycle_pre.as_ref(), session_id);

    let mut all_prompts: Vec<String> = Vec::new();
    let mut summary = String::new();
    let mut transcript_modified_files: Vec<String> = Vec::new();
    let mut new_transcript_position = transcript_offset;

    if let Some(analyzer) = agent.as_transcript_analyzer() {
        if let Ok(prompts) = analyzer.extract_prompts(&transcript_ref_str, transcript_offset) {
            all_prompts = prompts;
        }
        if let Ok(s) = analyzer.extract_summary(&transcript_ref_str) {
            summary = s;
        }
        if let Ok((files, pos)) =
            analyzer.extract_modified_files_from_offset(&transcript_ref_str, transcript_offset)
        {
            transcript_modified_files = filter_and_normalize_paths_for_turn_end(&files, &repo_root);
            new_transcript_position = pos;
        }
    }
    // Use transcript we already read (same bytes we copied to metadata); parse with Gemini or raw JSON
    if let Ok(t) = crate::engine::agent::gemini_cli::transcript::parse_transcript(&transcript_data)
    {
        let from_transcript =
            crate::engine::agent::gemini_cli::transcript::extract_all_user_prompts_from_transcript(
                &t,
            );
        if !from_transcript.is_empty() {
            all_prompts = from_transcript;
        }
        for msg in t.messages.iter().rev() {
            if msg.r#type == crate::engine::agent::gemini_cli::transcript::MESSAGE_TYPE_GEMINI
                && !msg.content.is_empty()
            {
                summary = msg.content.clone();
                break;
            }
        }
    }
    // Raw JSON fallback for {"messages":[{"type":"user","content":"..."}, ...]}
    if all_prompts.is_empty()
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&transcript_data)
        && let Some(arr) = v.get("messages").and_then(|m| m.as_array())
    {
        for msg in arr {
            if msg.get("type").and_then(|t| t.as_str()) == Some("user")
                && let Some(c) = msg.get("content").and_then(|c| c.as_str())
            {
                all_prompts.push(c.to_string());
            }
        }
    }
    if summary.is_empty()
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&transcript_data)
        && let Some(arr) = v.get("messages").and_then(|m| m.as_array())
    {
        for msg in arr.iter().rev() {
            if msg.get("type").and_then(|t| t.as_str()) == Some("gemini")
                && let Some(c) = msg.get("content").and_then(|c| c.as_str())
            {
                summary = c.to_string();
                break;
            }
        }
    }
    if summary.is_empty()
        && let Ok(s) = crate::engine::agent::gemini_cli::transcript::extract_last_assistant_message(
            &transcript_data,
        )
    {
        summary = s;
    }

    let prompt_file = meta_dir_abs.join(crate::engine::paths::PROMPT_FILE_NAME);
    let prompt_content = all_prompts.join("\n\n---\n\n");
    std::fs::write(&prompt_file, &prompt_content)
        .map_err(|e| anyhow!("failed to write prompt file: {e}"))?;

    let summary_file = meta_dir_abs.join(crate::engine::paths::SUMMARY_FILE_NAME);
    std::fs::write(&summary_file, &summary)
        .map_err(|e| anyhow!("failed to write summary file: {e}"))?;

    let last_prompt = all_prompts.last().cloned().unwrap_or_default();
    let commit_message = if last_prompt.len() > 72 {
        format!("{}...", &last_prompt[..69])
    } else {
        last_prompt.clone()
    };

    let context_path = meta_dir_abs.join(crate::engine::paths::CONTEXT_FILE_NAME);
    create_context_file(
        &context_path,
        &commit_message,
        session_id,
        &all_prompts,
        &summary,
    )?;

    let author = crate::engine::git_operations::get_git_author().unwrap_or(
        crate::engine::git_operations::GitAuthor {
            name: "Unknown".to_string(),
            email: "unknown@local".to_string(),
        },
    );

    let pre_untracked: Vec<String> = pre_prompt
        .as_ref()
        .map(|p| p.untracked_files.clone())
        .unwrap_or_default();
    let (git_modified, rel_new, rel_deleted) =
        detect_file_changes_for_turn_end(&repo_root, Some(&pre_untracked));
    // Transcript parsing is primary, git modified files are fallback for
    // unrecognized tools/transcript parsing misses.
    let mut rel_modified = merge_unique_for_turn_end(transcript_modified_files, git_modified);
    // Remove files that are already committed to HEAD.
    rel_modified = filter_to_uncommitted_files_for_turn_end(&repo_root, rel_modified);

    let token_usage = agent.as_token_calculator().and_then(|calc| {
        calc.calculate_token_usage(&transcript_ref_str, transcript_offset)
            .ok()
    });

    let metadata_dir_abs_str = meta_dir_abs.to_str().unwrap_or("").to_string();

    let ctx = crate::engine::strategy::StepContext {
        session_id: session_id.to_string(),
        modified_files: rel_modified,
        new_files: rel_new,
        deleted_files: rel_deleted,
        metadata_dir: meta_rel,
        metadata_dir_abs: metadata_dir_abs_str,
        commit_message,
        transcript_path: event.session_ref.clone(),
        author_name: author.name,
        author_email: author.email,
        agent_type: agent.agent_name().to_string(),
        step_transcript_identifier: session_id.to_string(),
        step_transcript_start: new_transcript_position as i64,
        token_usage,
    };

    let registry = crate::engine::strategy::registry::StrategyRegistry::builtin();
    let strategy = registry.get(
        crate::engine::strategy::registry::STRATEGY_NAME_MANUAL_COMMIT,
        &repo_root,
    )?;
    strategy.save_step(&ctx)?;

    if let Ok(Some(mut state)) = backend.load_session(session_id) {
        let context = SessionTransitionContext {
            has_files_touched: !state.files_touched.is_empty(),
            is_rebase_in_progress: false,
        };
        let transition =
            transition_session_with_context(state.phase, SessionEvent::TurnEnd, context);
        if apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler).is_ok() {
            let _ = backend.save_session(&state);
        }
    }

    let _ = backend.delete_pre_prompt(session_id);

    Ok(())
}

pub fn handle_lifecycle_compaction(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if event.session_id.is_empty() {
        eprintln!("Context compaction: transcript offset reset");
        return Ok(());
    }

    let repo_root = match crate::engine::paths::repo_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!(
                "[bitloops] Warning: failed to resolve repository root for compaction: {err}"
            );
            eprintln!("Context compaction: transcript offset reset");
            return Ok(());
        }
    };

    let backend = LocalFileBackend::new(&repo_root);
    match backend.load_session(&event.session_id) {
        Ok(Some(mut state)) => {
            let context = SessionTransitionContext {
                has_files_touched: !state.files_touched.is_empty(),
                is_rebase_in_progress: false,
            };
            let transition =
                transition_session_with_context(state.phase, SessionEvent::Compaction, context);
            let mut handler = SessionNoOpActionHandler;
            if let Err(err) = apply_session_transition(&mut state, transition, &mut handler) {
                eprintln!("[bitloops] Warning: compaction transition failed: {err}");
            }

            // Compaction resets transcript offset after transition.
            state.checkpoint_transcript_start = 0;
            if let Err(err) = backend.save_session(&state) {
                eprintln!(
                    "[bitloops] Warning: failed to save session state after compaction: {err}"
                );
            }
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("[bitloops] Warning: failed to load session state for compaction: {err}");
        }
    }

    eprintln!("Context compaction: transcript offset reset");
    Ok(())
}

pub fn handle_lifecycle_session_end(
    _agent: &dyn LifecycleAgentAdapter,
    _event: &LifecycleEvent,
) -> Result<()> {
    Ok(())
}

pub fn handle_lifecycle_subagent_start(
    _agent: &dyn LifecycleAgentAdapter,
    _event: &LifecycleEvent,
) -> Result<()> {
    Ok(())
}

pub fn handle_lifecycle_subagent_end(
    _agent: &dyn LifecycleAgentAdapter,
    _event: &LifecycleEvent,
) -> Result<()> {
    Ok(())
}

/// Captures pre-prompt state (including transcript position from the agent) for consumption at turn end.
///
/// **Orchestration stub:** currently saves transcript_offset 0 without calling the agent.
/// Implement by calling `agent.get_transcript_position(session_ref)` and persisting that offset.
pub fn capture_pre_prompt_state(
    agent: &dyn TranscriptPositionProvider,
    session_id: &str,
    session_ref: &str,
    repo_root: &Path,
) -> Result<()> {
    use crate::engine::session::state::PrePromptState as SessionPrePromptState;
    use std::time::{SystemTime, UNIX_EPOCH};

    if session_id.is_empty() {
        return Err(anyhow!(
            "session_id is required for capture_pre_prompt_state"
        ));
    }

    let transcript_offset = agent.get_transcript_position(session_ref)?;
    let backend = LocalFileBackend::new(repo_root);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let state = SessionPrePromptState {
        session_id: session_id.to_string(),
        timestamp: format!("{}", timestamp),
        transcript_path: session_ref.to_string(),
        transcript_offset: transcript_offset as i64,
        ..SessionPrePromptState::default()
    };
    backend.save_pre_prompt(&state)
}

pub fn resolve_transcript_offset(
    pre_prompt_state: Option<&PrePromptState>,
    _session_id: &str,
) -> usize {
    if let Some(pre_prompt_state) = pre_prompt_state
        && pre_prompt_state.transcript_offset > 0
    {
        return pre_prompt_state.transcript_offset;
    }
    0
}

pub fn create_context_file(
    path: &std::path::Path,
    commit_message: &str,
    session_id: &str,
    prompts: &[String],
    summary: &str,
) -> Result<()> {
    let mut output = String::new();
    output.push_str("# Session Context\n\n");
    output.push_str(&format!("Session ID: {session_id}\n"));
    output.push_str(&format!("Commit Message: {commit_message}\n\n"));

    if !prompts.is_empty() {
        output.push_str("## Prompts\n\n");
        for (idx, prompt) in prompts.iter().enumerate() {
            output.push_str(&format!("### Prompt {}\n\n{prompt}\n\n", idx + 1));
        }
    }

    if !summary.is_empty() {
        output.push_str("## Summary\n\n");
        output.push_str(summary);
        output.push('\n');
    }

    std::fs::write(path, output).map_err(|err| anyhow!("failed to write context file: {err}"))
}

pub fn read_and_parse_hook_input<T: DeserializeOwned>(stdin: &mut dyn Read) -> Result<T> {
    let mut raw = String::new();
    stdin.read_to_string(&mut raw)?;
    if raw.trim().is_empty() {
        return Err(anyhow!("empty hook input"));
    }

    let mut parsed: Value =
        serde_json::from_str(&raw).map_err(|err| anyhow!("failed to parse hook input: {err}"))?;

    for _ in 0..16 {
        match serde_json::from_value::<T>(parsed.clone()) {
            Ok(result) => return Ok(result),
            Err(err) => {
                let Some(missing_field) = extract_missing_field_name(&err) else {
                    return Err(anyhow!("failed to parse hook input: {err}"));
                };

                let Some(object) = parsed.as_object_mut() else {
                    return Err(anyhow!("failed to parse hook input: {err}"));
                };

                if object.contains_key(&missing_field) {
                    return Err(anyhow!("failed to parse hook input: {err}"));
                }
                object.insert(missing_field, Value::String(String::new()));
            }
        }
    }

    Err(anyhow!(
        "failed to parse hook input: exceeded missing-field fallback attempts"
    ))
}

fn extract_missing_field_name(err: &serde_json::Error) -> Option<String> {
    let message = err.to_string();
    let prefix = "missing field `";
    let start = message.find(prefix)? + prefix.len();
    let tail = &message[start..];
    let end = tail.find('`')?;
    Some(tail[..end].to_string())
}

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod orchestration_tests;
