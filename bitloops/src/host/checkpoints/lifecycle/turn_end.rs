use anyhow::{Context, Result, anyhow};
use std::path::Path;

use super::adapter::LifecycleAgentAdapter;
use super::canonical::build_phase3_canonical_request;
use super::git_workspace::{
    detect_file_changes_for_turn_end, filter_and_normalize_paths_for_turn_end,
    filter_to_uncommitted_files_for_turn_end, merge_unique_for_turn_end,
};
use super::interaction::{flush_interaction_spool_best_effort, resolve_interaction_spool};
use super::time_and_ids::{generate_interaction_event_id, generate_lifecycle_turn_id, now_rfc3339};
use super::transcript::resolve_transcript_offset;
use super::types::{LifecycleEvent, PrePromptState, SessionIdPolicy, apply_session_id_policy};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};
use crate::host::checkpoints::session::state::PRE_PROMPT_SOURCE_CURSOR_SHELL;
use crate::host::checkpoints::transcript::metadata::{
    build_session_metadata_bundle, extract_prompts_from_transcript_bytes,
};
use crate::host::interactions::model::resolve_interaction_model_from_bytes;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::transcript_fragment::{
    transcript_fragment_from_bytes, transcript_position_from_bytes,
};
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};
use crate::host::runtime_store::{RepoSqliteRuntimeStore, SessionMetadataSnapshot};

pub fn handle_lifecycle_turn_end(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let session_id = apply_session_id_policy(&event.session_id, SessionIdPolicy::FallbackUnknown)?;
    let repo_root = crate::utils::paths::repo_root()?;
    let backend = create_session_backend_or_local(&repo_root);
    let pre_prompt = backend.load_pre_prompt(&session_id).ok().flatten();
    let session_before_capture = backend.load_session(&session_id).ok().flatten();
    let (transcript_ref, attempted_sources) = resolve_turn_end_transcript_ref(
        &event.session_ref,
        pre_prompt
            .as_ref()
            .map(|state| state.transcript_path.as_str()),
        session_before_capture
            .as_ref()
            .map(|state| state.transcript_path.as_str()),
    );

    if !event.session_id.is_empty() {
        let mut resolved_event = event.clone();
        resolved_event.session_ref = transcript_ref.clone();
        let _canonical_request =
            build_phase3_canonical_request(agent.agent_name(), &resolved_event)?;
    }

    if event.source == PRE_PROMPT_SOURCE_CURSOR_SHELL
        && pre_prompt.as_ref().map(|state| state.source.as_str())
            != Some(PRE_PROMPT_SOURCE_CURSOR_SHELL)
    {
        return Ok(());
    }

    if transcript_ref.is_empty() {
        return Err(anyhow!(
            "transcript file not specified (checked: {})",
            attempted_sources.join(", ")
        ));
    }

    // Agents flush transcripts asynchronously; retry briefly before giving up.
    let transcript_data = read_transcript_with_retry(&transcript_ref)?;

    if crate::git::is_empty_repository()? {
        return Err(anyhow!("empty repository"));
    }

    let transcript_ref_canon = Path::new(&transcript_ref)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&transcript_ref).to_path_buf());
    let transcript_ref_str = transcript_ref_canon.to_string_lossy().to_string();

    let lifecycle_pre = pre_prompt.as_ref().map(|p| PrePromptState {
        transcript_offset: p.transcript_offset as usize,
    });
    let transcript_offset = resolve_transcript_offset(lifecycle_pre.as_ref(), &session_id);

    let mut transcript_modified_files: Vec<String> = Vec::new();
    let mut new_transcript_position = transcript_offset;

    if let Some(analyzer) = agent.as_transcript_analyzer()
        && let Ok((files, pos)) =
            analyzer.extract_modified_files_from_offset(&transcript_ref_str, transcript_offset)
    {
        transcript_modified_files = filter_and_normalize_paths_for_turn_end(&files, &repo_root);
        new_transcript_position = pos;
    }

    if new_transcript_position <= transcript_offset && !transcript_data.is_empty() {
        new_transcript_position = transcript_position_from_bytes(&transcript_data);
    }

    let derived_prompts = extract_prompts_from_transcript_bytes(&transcript_data);
    let last_prompt = derived_prompts.last().cloned().unwrap_or_default();
    let commit_message = crate::utils::strings::truncate_runes(&last_prompt, 72, "...");
    let metadata = build_session_metadata_bundle(&session_id, &commit_message, &transcript_data)?;
    let all_prompts = metadata.prompts.clone();
    let summary = metadata.summary.clone();

    let author = crate::git::get_git_author().unwrap_or(crate::git::GitAuthor {
        name: "Unknown".to_string(),
        email: "unknown@local".to_string(),
    });

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

    let turn_id = session_before_capture
        .as_ref()
        .map(|state| state.turn_id.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(generate_lifecycle_turn_id);
    let turn_number = session_before_capture
        .as_ref()
        .map(|state| state.pending.step_count + 1)
        .unwrap_or(1);

    let runtime_store = RepoSqliteRuntimeStore::open(&repo_root)
        .context("opening runtime store for lifecycle turn-end metadata")?;
    let mut snapshot = SessionMetadataSnapshot::new(session_id.clone(), metadata.clone());
    snapshot.turn_id = turn_id.clone();
    snapshot.transcript_identifier = session_id.clone();
    snapshot.transcript_path = transcript_ref.clone();
    runtime_store
        .save_session_metadata_snapshot(&snapshot)
        .context("saving lifecycle turn-end metadata snapshot")?;

    let ctx = crate::host::checkpoints::strategy::StepContext {
        session_id: session_id.to_string(),
        modified_files: rel_modified,
        new_files: rel_new,
        deleted_files: rel_deleted,
        metadata: Some(metadata.clone()),
        commit_message,
        transcript_path: transcript_ref.clone(),
        author_name: author.name,
        author_email: author.email,
        agent_type: agent.agent_name().to_string(),
        step_transcript_identifier: session_id.to_string(),
        step_transcript_start: new_transcript_position as i64,
        token_usage,
    };

    let interaction_now = now_rfc3339();
    let all_files: Vec<String> = ctx
        .modified_files
        .iter()
        .chain(ctx.new_files.iter())
        .chain(ctx.deleted_files.iter())
        .cloned()
        .collect();
    let total_changes = all_files.len();
    let token_meta = ctx.token_usage.as_ref().map(|t| {
        crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata {
            input_tokens: t.input_tokens.max(0) as u64,
            cache_creation_tokens: t.cache_creation_tokens.max(0) as u64,
            cache_read_tokens: t.cache_read_tokens.max(0) as u64,
            output_tokens: t.output_tokens.max(0) as u64,
            api_call_count: t.api_call_count.max(0) as u64,
            subagent_tokens: None,
        }
    });
    let transcript_fragment = transcript_fragment_from_bytes(
        &transcript_data,
        transcript_offset,
        new_transcript_position,
    );
    let model = resolve_interaction_model_from_bytes(&event.model, &transcript_data);

    if let Some(spool) = resolve_interaction_spool(&repo_root) {
        let session = session_before_capture
            .as_ref()
            .map(|state| InteractionSession {
                session_id: session_id.clone(),
                repo_id: spool.repo_id().to_string(),
                agent_type: state.agent_type.clone(),
                model: model.clone(),
                first_prompt: state.first_prompt.clone(),
                transcript_path: state.transcript_path.clone(),
                worktree_path: state.worktree_path.clone(),
                worktree_id: state.worktree_id.clone(),
                started_at: if state.started_at.trim().is_empty() {
                    interaction_now.clone()
                } else {
                    state.started_at.clone()
                },
                ended_at: state.ended_at.clone(),
                last_event_at: interaction_now.clone(),
                updated_at: interaction_now.clone(),
                ..Default::default()
            })
            .unwrap_or(InteractionSession {
                session_id: session_id.clone(),
                repo_id: spool.repo_id().to_string(),
                agent_type: ctx.agent_type.clone(),
                model: model.clone(),
                first_prompt: last_prompt.clone(),
                transcript_path: transcript_ref.clone(),
                worktree_path: repo_root.to_string_lossy().to_string(),
                worktree_id: crate::utils::paths::get_worktree_id(&repo_root).unwrap_or_default(),
                started_at: interaction_now.clone(),
                ended_at: None,
                last_event_at: interaction_now.clone(),
                updated_at: interaction_now.clone(),
                ..Default::default()
            });
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }

        let turn = InteractionTurn {
            turn_id: turn_id.clone(),
            session_id: session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            turn_number,
            prompt: all_prompts
                .last()
                .cloned()
                .unwrap_or_else(|| last_prompt.clone()),
            agent_type: ctx.agent_type.clone(),
            model: model.clone(),
            started_at: session_before_capture
                .as_ref()
                .and_then(|state| {
                    (!state
                        .last_interaction_time
                        .clone()
                        .unwrap_or_default()
                        .is_empty())
                    .then(|| state.last_interaction_time.clone().unwrap_or_default())
                })
                .unwrap_or_else(|| interaction_now.clone()),
            ended_at: Some(interaction_now.clone()),
            token_usage: token_meta.clone(),
            summary: summary.clone(),
            prompt_count: all_prompts.len().min(u32::MAX as usize) as u32,
            transcript_offset_start: Some(transcript_offset as i64),
            transcript_offset_end: Some(new_transcript_position as i64),
            transcript_fragment: transcript_fragment.clone(),
            files_modified: all_files.clone(),
            checkpoint_id: None,
            updated_at: interaction_now.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_turn(&turn) {
            eprintln!("[bitloops] Warning: failed to spool interaction turn end: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: Some(turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::TurnEnd,
            event_time: interaction_now.clone(),
            agent_type: ctx.agent_type.clone(),
            model,
            payload: serde_json::json!({
                "files_modified": all_files,
                "files_count": all_files.len(),
                "prompt_count": all_prompts.len(),
                "summary": summary,
                "transcript_offset_start": transcript_offset,
                "transcript_offset_end": new_transcript_position,
                "transcript_fragment": transcript_fragment,
                "token_usage": token_meta,
            }),
            ..Default::default()
        }) {
            eprintln!("[bitloops] Warning: failed to spool turn_end event: {err}");
        }
    }
    flush_interaction_spool_best_effort(&repo_root);

    if total_changes > 0 {
        let strategy = super::resolve_configured_strategy(&repo_root)?;
        strategy.save_step(&ctx)?;
    }

    if let Ok(Some(mut state)) = backend.load_session(&session_id) {
        let context = SessionTransitionContext {
            has_files_touched: !state.pending.files_touched.is_empty(),
            is_rebase_in_progress: false,
        };
        let transition =
            transition_session_with_context(state.phase, SessionEvent::TurnEnd, context);
        if apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler).is_ok() {
            let _ = backend.save_session(&state);
        }
    }

    let _ = backend.delete_pre_prompt(&session_id);

    Ok(())
}

fn resolve_turn_end_transcript_ref(
    raw_path: &str,
    pre_prompt_path: Option<&str>,
    session_path: Option<&str>,
) -> (String, Vec<&'static str>) {
    let mut attempted_sources = vec!["hook payload transcript_path"];
    if !raw_path.trim().is_empty() {
        return (raw_path.to_string(), attempted_sources);
    }

    attempted_sources.push("pre-prompt state");
    if let Some(path) = pre_prompt_path
        && !path.trim().is_empty()
    {
        return (path.to_string(), attempted_sources);
    }

    attempted_sources.push("session state");
    if let Some(path) = session_path
        && !path.trim().is_empty()
    {
        return (path.to_string(), attempted_sources);
    }

    (String::new(), attempted_sources)
}

/// Reads the transcript file with a brief retry window to handle agents that
/// flush transcripts asynchronously (e.g., Claude Code writes entries after
/// firing the hook).  Retries for up to 3 seconds at 50 ms intervals, but
/// only when the parent directory exists (indicating the file may still be
/// flushing).  If the parent directory itself is missing the error is immediate.
fn read_transcript_with_retry(path: &str) -> Result<Vec<u8>> {
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    let p = Path::new(path);
    let parent_exists = p.parent().is_some_and(|d| d.exists());

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match std::fs::read(path) {
            Ok(data) => return Ok(data),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    && parent_exists
                    && Instant::now() < deadline =>
            {
                sleep(Duration::from_millis(50));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow!("transcript file not found: {path}"));
            }
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        }
    }
}
