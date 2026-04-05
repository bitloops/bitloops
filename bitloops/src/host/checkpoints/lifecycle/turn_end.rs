use anyhow::{Result, anyhow};
use std::path::Path;

use super::adapter::LifecycleAgentAdapter;
use super::canonical::build_phase3_canonical_request;
use super::git_workspace::{
    detect_file_changes_for_turn_end, filter_and_normalize_paths_for_turn_end,
    filter_to_uncommitted_files_for_turn_end, merge_unique_for_turn_end,
};
use super::interaction::resolve_interaction_event_store;
use super::time_and_ids::{generate_interaction_event_id, now_rfc3339};
use super::transcript::{create_context_file, resolve_transcript_offset};
use super::types::{LifecycleEvent, PrePromptState, SessionIdPolicy, apply_session_id_policy};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};
use crate::host::interactions::store::InteractionEventStore;
use crate::host::interactions::types::{InteractionEvent, InteractionEventType};

pub fn handle_lifecycle_turn_end(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if !event.session_id.is_empty() {
        let _canonical_request = build_phase3_canonical_request(agent.agent_name(), event)?;
    }

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

    if crate::git::is_empty_repository()? {
        return Err(anyhow!("empty repository"));
    }

    let repo_root = crate::utils::paths::repo_root()?;
    let session_id = apply_session_id_policy(&event.session_id, SessionIdPolicy::FallbackUnknown)?;
    let meta_rel = crate::utils::paths::session_metadata_dir_from_session_id(&session_id);
    let meta_dir_abs = {
        let path = repo_root.join(&meta_rel);
        std::fs::create_dir_all(&path)
            .map_err(|e| anyhow!("failed to create session directory: {e}"))?;
        Some(path)
    };

    let transcript_data =
        std::fs::read(&event.session_ref).map_err(|e| anyhow!("failed to read transcript: {e}"))?;
    if let Some(meta_dir_abs) = meta_dir_abs.as_ref() {
        let log_path = meta_dir_abs.join(crate::utils::paths::TRANSCRIPT_FILE_NAME);
        std::fs::write(&log_path, &transcript_data)
            .map_err(|e| anyhow!("failed to write transcript: {e}"))?;
    }

    let transcript_ref_canon = Path::new(&event.session_ref)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&event.session_ref).to_path_buf());
    let transcript_ref_str = transcript_ref_canon.to_string_lossy().to_string();

    let backend = create_session_backend_or_local(&repo_root);
    let pre_prompt = backend.load_pre_prompt(&session_id).ok().flatten();
    let lifecycle_pre = pre_prompt.as_ref().map(|p| PrePromptState {
        transcript_offset: p.transcript_offset as usize,
    });
    let transcript_offset = resolve_transcript_offset(lifecycle_pre.as_ref(), &session_id);

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
    if let Ok(t) = crate::adapters::agents::gemini::transcript::parse_transcript(&transcript_data) {
        let from_transcript =
            crate::adapters::agents::gemini::transcript::extract_all_user_prompts_from_transcript(
                &t,
            );
        if !from_transcript.is_empty() {
            all_prompts = from_transcript;
        }
        for msg in t.messages.iter().rev() {
            if msg.r#type == crate::adapters::agents::gemini::transcript::MESSAGE_TYPE_GEMINI
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
        && let Ok(s) = crate::adapters::agents::gemini::transcript::extract_last_assistant_message(
            &transcript_data,
        )
    {
        summary = s;
    }

    let prompt_content = all_prompts.join("\n\n---\n\n");
    if let Some(meta_dir_abs) = meta_dir_abs.as_ref() {
        let prompt_file = meta_dir_abs.join(crate::utils::paths::PROMPT_FILE_NAME);
        std::fs::write(&prompt_file, &prompt_content)
            .map_err(|e| anyhow!("failed to write prompt file: {e}"))?;

        let summary_file = meta_dir_abs.join(crate::utils::paths::SUMMARY_FILE_NAME);
        std::fs::write(&summary_file, &summary)
            .map_err(|e| anyhow!("failed to write summary file: {e}"))?;
    }

    let last_prompt = all_prompts.last().cloned().unwrap_or_default();
    let commit_message = if last_prompt.len() > 72 {
        format!("{}...", &last_prompt[..69])
    } else {
        last_prompt.clone()
    };

    if let Some(meta_dir_abs) = meta_dir_abs.as_ref() {
        let context_path = meta_dir_abs.join(crate::utils::paths::CONTEXT_FILE_NAME);
        create_context_file(
            &context_path,
            &commit_message,
            &session_id,
            &all_prompts,
            &summary,
        )?;
    }

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

    let metadata_dir_abs_str = meta_dir_abs
        .as_ref()
        .and_then(|path| path.to_str())
        .unwrap_or("")
        .to_string();

    let ctx = crate::host::checkpoints::strategy::StepContext {
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

    let registry = crate::host::checkpoints::strategy::registry::StrategyRegistry::builtin();
    let strategy = registry.get(
        crate::host::checkpoints::strategy::registry::STRATEGY_NAME_MANUAL_COMMIT,
        &repo_root,
    )?;
    strategy.save_step(&ctx)?;

    // ── interaction event persistence ────────────────────────────────────────
    if let Some(store) = resolve_interaction_event_store(&repo_root) {
        let turn_id = backend
            .load_session(&session_id)
            .ok()
            .flatten()
            .map(|s| s.turn_id.clone());
        if let Some(turn_id) = turn_id.filter(|id| !id.trim().is_empty()) {
            let now = now_rfc3339();
            let all_files: Vec<String> = ctx
                .modified_files
                .iter()
                .chain(ctx.new_files.iter())
                .chain(ctx.deleted_files.iter())
                .cloned()
                .collect();
            let token_meta = ctx.token_usage.as_ref().map(|t| {
                crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata {
                    input_tokens: t.input_tokens as u64,
                    cache_creation_tokens: t.cache_creation_tokens as u64,
                    cache_read_tokens: t.cache_read_tokens as u64,
                    output_tokens: t.output_tokens as u64,
                    api_call_count: t.api_call_count as u64,
                    subagent_tokens: None,
                }
            });
            if let Err(err) = store.record_turn_end(&turn_id, &now, token_meta.as_ref(), &all_files)
            {
                eprintln!("[bitloops] Warning: failed to record interaction turn end: {err}");
            }
            let payload = serde_json::json!({
                "files_count": all_files.len(),
                "token_usage": token_meta,
            });
            if let Err(err) = store.record_event(&InteractionEvent {
                event_id: generate_interaction_event_id(),
                session_id: session_id.to_string(),
                turn_id: Some(turn_id),
                repo_id: store.repo_id().to_string(),
                event_type: InteractionEventType::TurnEnd,
                event_time: now,
                agent_type: ctx.agent_type.clone(),
                model: event.model.clone(),
                payload,
            }) {
                eprintln!("[bitloops] Warning: failed to record turn_end event: {err}");
            }
        } else {
            eprintln!(
                "[bitloops] Warning: skipping interaction persistence for session {session_id} \
                 because turn_id is missing or empty"
            );
        }
    }

    if let Ok(Some(mut state)) = backend.load_session(&session_id) {
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

    let _ = backend.delete_pre_prompt(&session_id);

    Ok(())
}
