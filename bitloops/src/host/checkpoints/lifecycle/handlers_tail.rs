use anyhow::{Context, Result};

use super::adapter::LifecycleAgentAdapter;
use super::interaction::{flush_interaction_spool_best_effort, resolve_interaction_spool};
use super::time_and_ids::{generate_interaction_event_id, now_rfc3339};
use super::types::{LifecycleEvent, SessionIdPolicy, apply_session_id_policy};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};
use crate::host::checkpoints::session::state::PreTaskState;
use crate::host::checkpoints::strategy::TaskStepContext;
use crate::host::checkpoints::transcript::metadata::{
    TaskCheckpointMetadataBundle, build_incremental_checkpoint_payload,
    build_session_metadata_bundle, build_task_checkpoint_payload,
    extract_prompts_from_transcript_bytes,
};
use crate::host::hooks::runtime::agent_runtime::helpers::{
    count_todos_from_tool_input, detect_file_changes, detect_untracked_files,
    extract_last_completed_todo_from_tool_input, next_incremental_sequence,
    parse_subagent_type_and_description, resolve_subagent_transcript_path,
};
use crate::host::interactions::model::resolve_interaction_model;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession,
};
use crate::host::runtime_store::{
    RepoSqliteRuntimeStore, RuntimeMetadataBlobType, SessionMetadataSnapshot,
    TaskCheckpointArtefact,
};

pub fn handle_lifecycle_compaction(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let session_id = apply_session_id_policy(&event.session_id, SessionIdPolicy::PreserveEmpty)?;
    if session_id.is_empty() {
        eprintln!("Context compaction: transcript offset reset");
        return Ok(());
    }

    let repo_root = match crate::utils::paths::repo_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!(
                "[bitloops] Warning: failed to resolve repository root for compaction: {err}"
            );
            eprintln!("Context compaction: transcript offset reset");
            return Ok(());
        }
    };

    let backend = create_session_backend_or_local(&repo_root);
    match backend.load_session(&session_id) {
        Ok(Some(mut state)) => {
            let context = SessionTransitionContext {
                has_files_touched: !state.pending.files_touched.is_empty(),
                is_rebase_in_progress: false,
            };
            let transition =
                transition_session_with_context(state.phase, SessionEvent::Compaction, context);
            let mut handler = SessionNoOpActionHandler;
            if let Err(err) = apply_session_transition(&mut state, transition, &mut handler) {
                eprintln!("[bitloops] Warning: compaction transition failed: {err}");
            }

            // Compaction resets transcript offset after transition.
            state.pending.checkpoint_transcript_start = 0;
            if let Err(err) = backend.save_session(&state) {
                eprintln!(
                    "[bitloops] Warning: failed to save session state after compaction: {err}"
                );
            }
            let model = resolve_interaction_model(&event.model, &state.transcript_path);
            if let Some(spool) = resolve_interaction_spool(&repo_root)
                && let Err(err) = spool.record_event(&InteractionEvent {
                    event_id: generate_interaction_event_id(),
                    session_id: session_id.clone(),
                    turn_id: None,
                    repo_id: spool.repo_id().to_string(),
                    event_type: InteractionEventType::Compaction,
                    event_time: now_rfc3339(),
                    agent_type: state.agent_type.clone(),
                    model,
                    payload: serde_json::Value::Object(Default::default()),
                    ..Default::default()
                })
            {
                eprintln!("[bitloops] Warning: failed to spool compaction event: {err}");
            }
            flush_interaction_spool_best_effort(&repo_root);
            eprintln!("Context compaction: transcript offset reset");
            return Ok(());
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("[bitloops] Warning: failed to load session state for compaction: {err}");
        }
    }

    // ── interaction event persistence ────────────────────────────────────────
    if let Some(spool) = resolve_interaction_spool(&repo_root)
        && let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::Compaction,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: resolve_interaction_model(&event.model, &event.session_ref),
            payload: serde_json::Value::Object(Default::default()),
            ..Default::default()
        })
    {
        eprintln!("[bitloops] Warning: failed to spool compaction event: {err}");
    }
    flush_interaction_spool_best_effort(&repo_root);

    eprintln!("Context compaction: transcript offset reset");
    Ok(())
}

pub fn handle_lifecycle_session_end(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let session_id = apply_session_id_policy(&event.session_id, SessionIdPolicy::PreserveEmpty)?;
    if session_id.is_empty() {
        return Ok(());
    }

    let repo_root = crate::utils::paths::repo_root()?;
    let backend = create_session_backend_or_local(&repo_root);
    if event.finalize_open_turn {
        let pre_prompt = backend.load_pre_prompt(&session_id)?;
        let session = backend.load_session(&session_id)?;
        let should_finalize_turn = !session_id.is_empty()
            && (pre_prompt.is_some()
                || session.is_none()
                || session.as_ref().is_some_and(|state| {
                    state.phase == crate::host::checkpoints::session::phase::SessionPhase::Active
                        || (state.phase
                            == crate::host::checkpoints::session::phase::SessionPhase::Idle
                            && state.pending.step_count == 0)
                }));
        if should_finalize_turn {
            let mut turn_end = event.clone();
            turn_end.event_type = Some(super::types::LifecycleEventType::TurnEnd);
            super::turn_end::handle_lifecycle_turn_end(agent, &turn_end)?;
        }
    }
    let ended_at = now_rfc3339();
    let maybe_state = backend.load_session(&session_id)?;
    if let Some(mut state) = maybe_state.clone() {
        let context = SessionTransitionContext {
            has_files_touched: !state.pending.files_touched.is_empty(),
            is_rebase_in_progress: false,
        };
        let transition =
            transition_session_with_context(state.phase, SessionEvent::SessionStop, context);
        apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler)?;
        state.ended_at = Some(ended_at.clone());
        state.last_interaction_time = Some(ended_at.clone());
        backend.save_session(&state)?;
    }
    let transcript_path = maybe_state
        .as_ref()
        .map(|state| state.transcript_path.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(event.session_ref.as_str());
    let model = resolve_interaction_model(&event.model, transcript_path);
    if let Some(spool) = resolve_interaction_spool(&repo_root) {
        let session = maybe_state
            .map(|state| InteractionSession {
                session_id: session_id.clone(),
                repo_id: spool.repo_id().to_string(),
                agent_type: state.agent_type.clone(),
                model: model.clone(),
                first_prompt: state.first_prompt.clone(),
                transcript_path: state.transcript_path.clone(),
                worktree_path: state.worktree_path.clone(),
                worktree_id: state.worktree_id.clone(),
                started_at: state.started_at.clone(),
                ended_at: Some(ended_at.clone()),
                last_event_at: ended_at.clone(),
                updated_at: ended_at.clone(),
                ..Default::default()
            })
            .unwrap_or(InteractionSession {
                session_id: session_id.clone(),
                repo_id: spool.repo_id().to_string(),
                model: model.clone(),
                ended_at: Some(ended_at.clone()),
                last_event_at: ended_at.clone(),
                updated_at: ended_at.clone(),
                ..Default::default()
            });
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session end: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SessionEnd,
            event_time: ended_at.clone(),
            agent_type: session.agent_type.clone(),
            model,
            payload: serde_json::Value::Object(Default::default()),
            ..Default::default()
        }) {
            eprintln!("[bitloops] Warning: failed to spool session_end event: {err}");
        }
    }
    flush_interaction_spool_best_effort(&repo_root);
    Ok(())
}

pub fn handle_lifecycle_subagent_start(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if let Ok(repo_root) = crate::utils::paths::repo_root() {
        let backend = create_session_backend_or_local(&repo_root);
        if let Some(mut state) = backend.load_session(&event.session_id)? {
            state.last_interaction_time = Some(now_rfc3339());
            backend.save_session(&state)?;
        }

        let marker = PreTaskState {
            tool_use_id: event.tool_use_id.clone(),
            session_id: event.session_id.clone(),
            timestamp: now_rfc3339(),
            untracked_files: detect_untracked_files(Some(&repo_root)),
        };
        backend.create_pre_task_marker(&marker)?;

        if let Some(spool) = resolve_interaction_spool(&repo_root) {
            if let Err(err) = spool.record_event(&InteractionEvent {
                event_id: generate_interaction_event_id(),
                session_id: event.session_id.clone(),
                turn_id: None,
                repo_id: spool.repo_id().to_string(),
                event_type: InteractionEventType::SubagentStart,
                event_time: now_rfc3339(),
                agent_type: String::new(),
                model: resolve_interaction_model(&event.model, &event.session_ref),
                payload: serde_json::json!({
                    "subagent_id": event.subagent_id,
                    "tool_use_id": event.tool_use_id,
                }),
                ..Default::default()
            }) {
                eprintln!("[bitloops] Warning: failed to spool subagent_start event: {err}");
            }
            flush_interaction_spool_best_effort(&repo_root);
        }
    }
    Ok(())
}

pub fn handle_lifecycle_subagent_end(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if let Ok(repo_root) = crate::utils::paths::repo_root() {
        let backend = create_session_backend_or_local(&repo_root);
        let subagent_transcript_path = resolve_subagent_transcript_path(
            &event.session_ref,
            &event.session_id,
            &event.subagent_id,
        );
        let pre_untracked = backend
            .load_pre_task_marker(&event.tool_use_id)?
            .map(|s| s.untracked_files)
            .unwrap_or_default();
        let changes = detect_file_changes(Some(&repo_root), Some(&pre_untracked));
        let total_changes =
            changes.modified.len() + changes.new_files.len() + changes.deleted.len();
        let (subagent_type, task_description) =
            parse_subagent_type_and_description(event.tool_input.as_ref());

        if let Some(spool) = resolve_interaction_spool(&repo_root) {
            if let Err(err) = spool.record_event(&InteractionEvent {
                event_id: generate_interaction_event_id(),
                session_id: event.session_id.clone(),
                turn_id: None,
                repo_id: spool.repo_id().to_string(),
                event_type: InteractionEventType::SubagentEnd,
                event_time: now_rfc3339(),
                agent_type: String::new(),
                model: resolve_interaction_model(&event.model, &event.session_ref),
                payload: serde_json::json!({
                    "tool_use_id": event.tool_use_id,
                    "subagent_id": event.subagent_id,
                    "subagent_type": subagent_type,
                    "task_description": task_description,
                    "subagent_transcript_path": subagent_transcript_path,
                }),
                tool_use_id: event.tool_use_id.clone(),
                tool_kind: subagent_type.clone(),
                task_description: task_description.clone(),
                subagent_id: event.subagent_id.clone(),
                ..Default::default()
            }) {
                eprintln!("[bitloops] Warning: failed to spool subagent_end event: {err}");
            }
            flush_interaction_spool_best_effort(&repo_root);
        }

        if total_changes > 0 {
            let session_metadata = std::fs::read(&event.session_ref).ok().and_then(|transcript| {
                let prompts = extract_prompts_from_transcript_bytes(&transcript);
                let commit_message = crate::host::hooks::runtime::agent_runtime::helpers::generate_commit_message(
                    prompts.last().map(String::as_str).unwrap_or_default(),
                );
                let metadata =
                    build_session_metadata_bundle(&event.session_id, &commit_message, &transcript)
                        .ok()?;
                let runtime_store = RepoSqliteRuntimeStore::open(&repo_root).ok()?;
                let mut snapshot =
                    SessionMetadataSnapshot::new(event.session_id.clone(), metadata.clone());
                snapshot.transcript_identifier = event.session_id.clone();
                snapshot.transcript_path = event.session_ref.clone();
                runtime_store.save_session_metadata_snapshot(&snapshot).ok()?;
                Some(metadata)
            });
            let checkpoint_json = build_task_checkpoint_payload(
                &event.session_id,
                &event.tool_use_id,
                "",
                &event.subagent_id,
            )?;
            let subagent_transcript = if subagent_transcript_path.trim().is_empty() {
                None
            } else {
                std::fs::read(&subagent_transcript_path).ok()
            };
            let runtime_store = RepoSqliteRuntimeStore::open(&repo_root)
                .context("opening runtime store for lifecycle task checkpoint artefacts")?;
            let mut checkpoint_artefact = TaskCheckpointArtefact::new(
                event.session_id.clone(),
                event.tool_use_id.clone(),
                RuntimeMetadataBlobType::TaskCheckpoint,
                checkpoint_json.clone(),
            );
            checkpoint_artefact.agent_id = event.subagent_id.clone();
            runtime_store.save_task_checkpoint_artefact(&checkpoint_artefact)?;
            if let Some(payload) = subagent_transcript.as_ref() {
                let mut transcript_artefact = TaskCheckpointArtefact::new(
                    event.session_id.clone(),
                    event.tool_use_id.clone(),
                    RuntimeMetadataBlobType::SubagentTranscript,
                    payload.clone(),
                );
                transcript_artefact.agent_id = event.subagent_id.clone();
                runtime_store.save_task_checkpoint_artefact(&transcript_artefact)?;
            }

            let strategy = super::resolve_configured_strategy(&repo_root)?;
            strategy.save_task_step(&TaskStepContext {
                session_id: event.session_id.clone(),
                tool_use_id: event.tool_use_id.clone(),
                agent_id: event.subagent_id.clone(),
                modified_files: changes.modified,
                new_files: changes.new_files,
                deleted_files: changes.deleted,
                session_metadata,
                task_metadata: Some(TaskCheckpointMetadataBundle {
                    checkpoint_json: Some(checkpoint_json),
                    subagent_transcript,
                    incremental_checkpoint: None,
                    prompt: None,
                }),
                transcript_path: event.session_ref.clone(),
                subagent_transcript_path,
                checkpoint_uuid: String::new(),
                author_name: String::new(),
                author_email: String::new(),
                subagent_type,
                task_description,
                agent_type: agent.agent_name().to_string(),
                is_incremental: false,
                incremental_sequence: 0,
                incremental_type: String::new(),
                incremental_data: String::new(),
                todo_content: String::new(),
                commit_message: String::new(),
            })?;
        }

        backend.delete_pre_task_marker(&event.tool_use_id)?;
        if let Some(mut state) = backend.load_session(&event.session_id)? {
            state.last_interaction_time = Some(now_rfc3339());
            backend.save_session(&state)?;
        }
    }
    Ok(())
}

pub fn handle_lifecycle_todo_checkpoint(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let Ok(repo_root) = crate::utils::paths::repo_root() else {
        return Ok(());
    };
    let backend = create_session_backend_or_local(&repo_root);
    let active_task = backend.find_active_pre_task()?;
    let task_tool_use_id = match active_task {
        Some(id) => id,
        None => return Ok(()),
    };

    let (skip, branch_name) = crate::git::should_skip_on_default_branch();
    if skip {
        eprintln!("Bitloops: skipping incremental checkpoint on branch '{branch_name}'");
        return Ok(());
    }

    let changes = detect_file_changes(Some(&repo_root), None);
    let total_changes = changes.modified.len() + changes.new_files.len() + changes.deleted.len();
    if total_changes == 0 {
        return Ok(());
    }

    let mut todo_content = extract_last_completed_todo_from_tool_input(event.tool_input.as_ref());
    if todo_content.is_empty() {
        let todo_count = count_todos_from_tool_input(event.tool_input.as_ref());
        if todo_count > 0 {
            todo_content = format!("Planning: {todo_count} todos");
        }
    }

    let incremental_sequence = RepoSqliteRuntimeStore::open(&repo_root)
        .and_then(|store| {
            store.next_task_incremental_sequence(&event.session_id, &task_tool_use_id)
        })
        .unwrap_or_else(|_| {
            next_incremental_sequence(Some(&repo_root), &event.session_id, &task_tool_use_id)
        });
    let session_metadata = std::fs::read(&event.session_ref)
        .ok()
        .and_then(|transcript| {
            let prompts = extract_prompts_from_transcript_bytes(&transcript);
            let commit_message =
                crate::host::hooks::runtime::agent_runtime::helpers::generate_commit_message(
                    prompts.last().map(String::as_str).unwrap_or_default(),
                );
            let metadata =
                build_session_metadata_bundle(&event.session_id, &commit_message, &transcript)
                    .ok()?;
            let runtime_store = RepoSqliteRuntimeStore::open(&repo_root).ok()?;
            let mut snapshot =
                SessionMetadataSnapshot::new(event.session_id.clone(), metadata.clone());
            snapshot.transcript_identifier = event.session_id.clone();
            snapshot.transcript_path = event.session_ref.clone();
            runtime_store
                .save_session_metadata_snapshot(&snapshot)
                .ok()?;
            Some(metadata)
        });
    let incremental_payload = build_incremental_checkpoint_payload(
        &task_tool_use_id,
        &event.tool_name,
        &now_rfc3339(),
        event
            .tool_input
            .as_ref()
            .unwrap_or(&serde_json::Value::Null),
    )?;
    let runtime_store = RepoSqliteRuntimeStore::open(&repo_root)
        .context("opening runtime store for lifecycle incremental checkpoint artefacts")?;
    let mut artefact = TaskCheckpointArtefact::new(
        event.session_id.clone(),
        task_tool_use_id.clone(),
        RuntimeMetadataBlobType::IncrementalCheckpoint,
        incremental_payload.clone(),
    );
    artefact.incremental_sequence = Some(incremental_sequence);
    artefact.incremental_type = event.tool_name.clone();
    artefact.is_incremental = true;
    runtime_store.save_task_checkpoint_artefact(&artefact)?;

    let strategy = super::resolve_configured_strategy(&repo_root)?;
    strategy.save_task_step(&TaskStepContext {
        session_id: event.session_id.clone(),
        tool_use_id: task_tool_use_id.clone(),
        agent_id: String::new(),
        modified_files: changes.modified,
        new_files: changes.new_files,
        deleted_files: changes.deleted,
        session_metadata,
        task_metadata: Some(TaskCheckpointMetadataBundle {
            checkpoint_json: None,
            subagent_transcript: None,
            incremental_checkpoint: Some(incremental_payload),
            prompt: None,
        }),
        transcript_path: event.session_ref.clone(),
        subagent_transcript_path: String::new(),
        checkpoint_uuid: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        subagent_type: String::new(),
        task_description: String::new(),
        agent_type: agent.agent_name().to_string(),
        is_incremental: true,
        incremental_sequence,
        incremental_type: event.tool_name.clone(),
        incremental_data: event
            .tool_input
            .as_ref()
            .map_or_else(String::new, |v| v.to_string()),
        todo_content,
        commit_message: String::new(),
    })?;

    if let Some(mut state) = backend.load_session(&event.session_id)? {
        state.last_interaction_time = Some(now_rfc3339());
        backend.save_session(&state)?;
    }

    Ok(())
}
