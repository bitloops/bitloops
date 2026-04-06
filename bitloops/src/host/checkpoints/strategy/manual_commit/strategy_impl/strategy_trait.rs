use super::*;
use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::host::interactions::interaction_repository::create_interaction_repository;
use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};

// ── Strategy trait impl ───────────────────────────────────────────────────────

impl Strategy for ManualCommitStrategy {
    fn name(&self) -> &str {
        "manual-commit"
    }

    fn initialize_session(
        &self,
        session_id: &str,
        agent_type: &str,
        transcript_path: &str,
        user_prompt: &str,
    ) -> Result<()> {
        self.initialize_or_refresh_session(session_id, agent_type, transcript_path, user_prompt)
    }

    /// Persists a temporary checkpoint tree capturing the current working tree state.
    ///
    fn save_step(&self, ctx: &StepContext) -> Result<()> {
        // If the repo has no commits yet, HEAD doesn't exist — skip silently.
        let Some(head) = try_head_hash(&self.repo_root)? else {
            return Ok(());
        };

        // Load or initialise session state.
        let mut state = match self.backend.load_session(&ctx.session_id)? {
            Some(s) if !s.base_commit.is_empty() => s,
            _ => {
                let agent_type = if ctx.agent_type.trim().is_empty() {
                    AGENT_TYPE_CLAUDE_CODE.to_string()
                } else {
                    ctx.agent_type.clone()
                };
                let s = SessionState {
                    session_id: ctx.session_id.clone(),
                    base_commit: head.clone(),
                    transcript_path: ctx.transcript_path.clone(),
                    started_at: now_rfc3339(),
                    agent_type,
                    cli_version: CLI_VERSION.to_string(),
                    turn_id: generate_checkpoint_id(),
                    ..Default::default()
                };
                self.backend.save_session(&s)?;
                s
            }
        };
        if state.agent_type.trim().is_empty() {
            state.agent_type = if ctx.agent_type.trim().is_empty() {
                AGENT_TYPE_CLAUDE_CODE.to_string()
            } else {
                ctx.agent_type.clone()
            };
        }
        if state.turn_id.trim().is_empty() {
            state.turn_id = generate_checkpoint_id();
        }
        if !ctx.transcript_path.trim().is_empty() {
            state.transcript_path = ctx.transcript_path.clone();
        }

        let default_prompt_attr = SessionPromptAttribution {
            checkpoint_number: state.pending.step_count as i32 + 1,
            ..Default::default()
        };
        let prompt_attr = state
            .pending_prompt_attribution
            .clone()
            .unwrap_or(default_prompt_attr);

        // Determine files to snapshot — use context lists, fall back to git status.
        let (modified, new_files, deleted) = if ctx.modified_files.is_empty()
            && ctx.new_files.is_empty()
            && ctx.deleted_files.is_empty()
        {
            working_tree_changes(&self.repo_root).unwrap_or_default()
        } else {
            (
                ctx.modified_files.clone(),
                ctx.new_files.clone(),
                ctx.deleted_files.clone(),
            )
        };

        let transcript_path = if ctx.transcript_path.is_empty() {
            state.transcript_path.clone()
        } else {
            ctx.transcript_path.clone()
        };
        let mut session_metadata = ctx.metadata.clone();
        if session_metadata.is_none() && !transcript_path.trim().is_empty() {
            let _ = write_session_metadata(&self.repo_root, &ctx.session_id, &transcript_path);
            session_metadata = read_session_metadata_bundle(&self.repo_root, &ctx.session_id);
        }

        let author_name = if ctx.author_name.trim().is_empty() {
            "Bitloops".to_string()
        } else {
            ctx.author_name.clone()
        };
        let author_email = if ctx.author_email.trim().is_empty() {
            "bitloops@localhost".to_string()
        } else {
            ctx.author_email.clone()
        };

        // Persist plain subjects for temporary checkpoints; metadata lives in DB/blobs.
        let subject = if ctx.commit_message.is_empty() {
            "Bitloops checkpoint".to_string()
        } else {
            ctx.commit_message.clone()
        };
        let commit_msg = subject;

        let result = write_temporary(
            &self.repo_root,
            WriteTemporaryOptions {
                session_id: ctx.session_id.clone(),
                base_commit: state.base_commit.clone(),
                step_number: state.pending.step_count + 1,
                modified_files: modified.clone(),
                new_files: new_files.clone(),
                deleted_files: deleted.clone(),
                session_metadata,
                metadata_entries: vec![],
                commit_message: commit_msg,
                author_name,
                author_email,
                is_first_checkpoint: state.pending.step_count == 0,
            },
        )?;
        if !result.skipped && result.commit_hash.is_empty() {
            anyhow::bail!("temporary checkpoint commit hash is empty");
        }

        // Dedup: skip if tree is identical to the latest temporary checkpoint tree.
        // Still persist token_usage when provided so turn-end can record usage without a new commit.
        if result.skipped {
            if let Some(usage) = &ctx.token_usage {
                state.pending.token_usage = Some(accumulate_token_usage(
                    state.pending.token_usage.take(),
                    usage,
                ));
                self.backend.save_session(&state)?;
            }
            return Ok(());
        }

        // Update session state.
        state.base_commit = head;
        state.pending.step_count += 1;
        state.cli_version = CLI_VERSION.to_string();
        state.pending_prompt_attribution = None;
        state.prompt_attributions.push(prompt_attr);
        // Record transcript identifier at the first step.
        if state.pending.step_count == 1 && state.pending.transcript_identifier_at_start.is_empty()
        {
            state.pending.transcript_identifier_at_start = ctx.step_transcript_identifier.clone();
        }
        if let Some(usage) = &ctx.token_usage {
            state.pending.token_usage = Some(accumulate_token_usage(
                state.pending.token_usage.take(),
                usage,
            ));
        }
        let all_files: Vec<String> = modified
            .iter()
            .chain(new_files.iter())
            .chain(deleted.iter())
            .cloned()
            .collect();
        merge_files_touched(&mut state.pending.files_touched, &all_files);
        self.backend.save_session(&state)?;

        Ok(())
    }

    /// Persists a task checkpoint as a temporary checkpoint tree.
    ///
    fn save_task_step(&self, ctx: &TaskStepContext) -> Result<()> {
        use crate::host::checkpoints::strategy::messages::{
            format_incremental_subject, format_subagent_end_message,
        };

        // Format commit message subject.
        let short_id = if ctx.tool_use_id.len() > 12 {
            &ctx.tool_use_id[..12]
        } else {
            &ctx.tool_use_id
        };
        let subject = if !ctx.commit_message.is_empty() {
            ctx.commit_message.clone()
        } else if ctx.is_incremental {
            format_incremental_subject(
                &ctx.incremental_type,
                &ctx.subagent_type,
                &ctx.task_description,
                &ctx.todo_content,
                ctx.incremental_sequence,
                short_id,
            )
        } else {
            format_subagent_end_message(&ctx.subagent_type, &ctx.task_description, short_id)
        };

        // Persist plain subjects for temporary task checkpoints.
        let commit_msg = subject;

        // If the repo has no commits yet, skip silently.
        let Some(head) = try_head_hash(&self.repo_root)? else {
            return Ok(());
        };

        // Load or initialise session state.
        let mut state = match self.backend.load_session(&ctx.session_id)? {
            Some(s) if !s.base_commit.is_empty() => s,
            _ => {
                let agent_type = if ctx.agent_type.trim().is_empty() {
                    AGENT_TYPE_CLAUDE_CODE.to_string()
                } else {
                    ctx.agent_type.clone()
                };
                let s = SessionState {
                    session_id: ctx.session_id.clone(),
                    base_commit: head.clone(),
                    transcript_path: ctx.transcript_path.clone(),
                    started_at: now_rfc3339(),
                    agent_type,
                    cli_version: CLI_VERSION.to_string(),
                    turn_id: generate_checkpoint_id(),
                    ..Default::default()
                };
                self.backend.save_session(&s)?;
                s
            }
        };

        let author_name = if ctx.author_name.trim().is_empty() {
            "Bitloops".to_string()
        } else {
            ctx.author_name.clone()
        };
        let author_email = if ctx.author_email.trim().is_empty() {
            "bitloops@localhost".to_string()
        } else {
            ctx.author_email.clone()
        };

        let task_result = write_temporary_task(
            &self.repo_root,
            WriteTemporaryTaskOptions {
                session_id: ctx.session_id.clone(),
                base_commit: state.base_commit.clone(),
                step_number: state.pending.step_count,
                tool_use_id: ctx.tool_use_id.clone(),
                agent_id: ctx.agent_id.clone(),
                modified_files: ctx.modified_files.clone(),
                new_files: ctx.new_files.clone(),
                deleted_files: ctx.deleted_files.clone(),
                session_metadata: if ctx.session_metadata.is_some() {
                    ctx.session_metadata.clone()
                } else if !ctx.transcript_path.trim().is_empty() {
                    let _ = write_session_metadata(
                        &self.repo_root,
                        &ctx.session_id,
                        &ctx.transcript_path,
                    );
                    read_session_metadata_bundle(&self.repo_root, &ctx.session_id)
                } else {
                    None
                },
                task_metadata: ctx.task_metadata.clone(),
                metadata_entries: vec![],
                is_incremental: ctx.is_incremental,
                incremental_sequence: ctx.incremental_sequence,
                incremental_type: ctx.incremental_type.clone(),
                incremental_data: ctx.incremental_data.clone(),
                commit_message: commit_msg,
                author_name,
                author_email,
            },
        )?;
        if task_result.commit_hash.is_empty() {
            anyhow::bail!("task checkpoint commit hash is empty");
        }

        // Update session state — accumulate pending files but don't bump checkpoint count
        // (task checkpoints are subordinate to regular turn checkpoints).
        let all_files: Vec<String> = ctx
            .modified_files
            .iter()
            .chain(ctx.new_files.iter())
            .chain(ctx.deleted_files.iter())
            .cloned()
            .collect();
        merge_files_touched(&mut state.pending.files_touched, &all_files);
        self.backend.save_session(&state)?;

        Ok(())
    }

    fn handle_turn_end(&self, state: &mut SessionState) -> Result<()> {
        self.finalize_all_turn_checkpoints(state);
        Ok(())
    }

    /// Called by the `prepare-commit-msg` git hook.
    /// This is intentionally a no-op.
    fn prepare_commit_msg(&self, _commit_msg_file: &Path, _source: Option<&str>) -> Result<()> {
        Ok(())
    }

    /// Called by the `commit-msg` git hook.
    /// This is intentionally a no-op.
    fn commit_msg(&self, _commit_msg_file: &Path) -> Result<()> {
        Ok(())
    }

    /// Condenses session data onto `bitloops/checkpoints/v1` after a commit lands.
    ///
    /// Called by the `post-commit` git hook.
    ///
    fn post_commit(&self) -> Result<()> {
        let is_rebase_in_progress = is_git_sequence_operation(&self.repo_root);
        let Some(head) = try_head_hash(&self.repo_root)? else {
            return Ok(());
        };
        let committed_files = files_changed_in_commit(&self.repo_root, &head).unwrap_or_default();
        let committed_files_set: std::collections::HashSet<String> =
            committed_files.iter().cloned().collect();
        if let Err(err) = run_devql_post_commit_refresh(&self.repo_root, &head, &committed_files) {
            eprintln!(
                "[bitloops] Warning: DevQL post-commit artefact refresh failed for commit {}: {err:#}",
                head
            );
        }
        if commit_has_checkpoint_mapping(&self.repo_root, &head)? {
            if let Some(checkpoint_id) = read_commit_checkpoint_mappings(&self.repo_root)?
                .get(&head)
                .cloned()
            {
                let projection_result = run_devql_post_commit_checkpoint_projection_refresh(
                    &self.repo_root,
                    &head,
                    &checkpoint_id,
                );
                if let Err(err) = projection_result {
                    eprintln!(
                        "[bitloops] Warning: DevQL checkpoint projection refresh failed for commit {} and checkpoint {}: {err:#}",
                        head, checkpoint_id
                    );
                }
            }
            return Ok(());
        }

        let interaction_spool = open_interaction_spool(&self.repo_root).ok();
        let interaction_spool_ref = interaction_spool
            .as_ref()
            .map(|spool| spool as &dyn InteractionSpool);
        let spool_pending_work = interaction_spool_ref.is_some_and(spool_has_pending_work);
        let interaction_repository = match resolve_interaction_repository_for_post_commit(
            &self.repo_root,
        ) {
            Ok(repository) => repository,
            Err(err) => {
                let context = format_post_commit_derivation_context(
                    &head,
                    None,
                    None,
                    &[],
                    Some(spool_pending_work),
                );
                if spool_pending_work {
                    eprintln!(
                        "[bitloops] Warning: failed to resolve interaction event repository for post_commit ({context}): {err:#}"
                    );
                    return Err(err).context(format!(
                        "resolving interaction event repository ({context})"
                    ));
                }
                eprintln!(
                    "[bitloops] Warning: failed to resolve interaction event repository for post_commit ({context}): {err:#}"
                );
                update_active_session_base_commits(
                    self.backend.as_ref(),
                    &head,
                    &std::collections::HashSet::new(),
                );
                return Ok(());
            }
        };

        if let Some(checkpoint_id) = self.derive_post_commit_from_interaction_sources(
            &head,
            &committed_files_set,
            is_rebase_in_progress,
            &interaction_repository,
            interaction_spool_ref,
        )? {
            insert_commit_checkpoint_mapping(&self.repo_root, &head, &checkpoint_id)?;
            if let Err(err) = run_devql_post_commit_checkpoint_projection_refresh(
                &self.repo_root,
                &head,
                &checkpoint_id,
            ) {
                eprintln!(
                    "[bitloops] Warning: DevQL checkpoint projection refresh failed for commit {} and checkpoint {}: {err:#}",
                    head, checkpoint_id
                );
            }
        }

        Ok(())
    }

    /// Called by the `pre-push` git hook.
    fn pre_push(&self, remote: &str, stdin_lines: &[String]) -> Result<()> {
        if let Err(err) = run_devql_pre_push_sync(&self.repo_root, remote, stdin_lines) {
            eprintln!(
                "[bitloops] Warning: DevQL pre-push replication failed for remote `{}`: {err:#}",
                remote
            );
        }
        Ok(())
    }

    /// Called by the `post-merge` git hook.
    fn post_merge(&self, is_squash: bool) -> Result<()> {
        if let Err(err) = run_devql_post_merge_refresh(&self.repo_root, is_squash) {
            eprintln!(
                "[bitloops] Warning: DevQL post-merge artefact refresh failed (is_squash={}): {err:#}",
                is_squash
            );
        }

        Ok(())
    }

    /// Called by the `post-checkout` git hook.
    fn post_checkout(
        &self,
        previous_head: &str,
        new_head: &str,
        is_branch_checkout: bool,
    ) -> Result<()> {
        if !is_branch_checkout {
            return Ok(());
        }

        if let Err(err) = run_devql_post_checkout_seed(
            &self.repo_root,
            previous_head,
            new_head,
            is_branch_checkout,
        ) {
            eprintln!(
                "[bitloops] Warning: DevQL post-checkout branch seeding failed (prev={}, new={}): {err:#}",
                previous_head, new_head
            );
        }

        Ok(())
    }

    /// Called by the `reference-transaction` git hook.
    fn reference_transaction(&self, state: &str, stdin_lines: &[String]) -> Result<()> {
        let deletions = collect_reference_transaction_branch_deletions(state, stdin_lines);
        if deletions.local_branches.is_empty() && deletions.remote_branches.is_empty() {
            return Ok(());
        }

        if let Err(err) = run_devql_reference_transaction_cleanup(&self.repo_root, &deletions) {
            eprintln!(
                "[bitloops] Warning: DevQL reference-transaction cleanup failed (state={}): {err:#}",
                state
            );
        }

        Ok(())
    }
}

impl ManualCommitStrategy {
    pub(crate) fn derive_post_commit_from_interaction_sources(
        &self,
        head: &str,
        committed_files_set: &std::collections::HashSet<String>,
        is_rebase_in_progress: bool,
        interaction_repository: &dyn InteractionEventRepository,
        interaction_spool: Option<&dyn InteractionSpool>,
    ) -> Result<Option<String>> {
        flush_interaction_spool_or_fail(head, interaction_spool, interaction_repository)?;

        let uncheckpointed_turns = interaction_repository
            .list_uncheckpointed_turns()
            .with_context(|| {
                let context = format_post_commit_derivation_context(head, None, None, &[], None);
                format!(
                    "listing uncheckpointed interaction turns from event repository ({context})"
                )
            })?;
        let mut turns_by_session: std::collections::BTreeMap<
            String,
            Vec<crate::host::interactions::InteractionTurn>,
        > = std::collections::BTreeMap::new();
        for turn in uncheckpointed_turns {
            turns_by_session
                .entry(turn.session_id.clone())
                .or_default()
                .push(turn);
        }
        let pending_turn_sessions = turns_by_session
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();

        let checkpoint_id = generate_checkpoint_id();
        let checkpoint_assigned_at = now_rfc3339();
        let mut condensed_any_session = false;
        let mut condensed_sessions = std::collections::HashSet::new();

        if !is_rebase_in_progress {
            for (session_id, mut session_turns) in turns_by_session {
                session_turns.sort_by(|left, right| {
                    left.turn_number
                        .cmp(&right.turn_number)
                        .then_with(|| left.started_at.cmp(&right.started_at))
                        .then_with(|| left.turn_id.cmp(&right.turn_id))
                });
                let contributing_turns = session_turns
                    .iter()
                    .filter(|turn| turn_overlaps_committed_files(turn, committed_files_set))
                    .cloned()
                    .collect::<Vec<_>>();
                if contributing_turns.is_empty() {
                    continue;
                }
                let session_turn_ids: Vec<String> = contributing_turns
                    .iter()
                    .map(|turn| turn.turn_id.clone())
                    .collect();
                let session_context = format_post_commit_derivation_context(
                    head,
                    Some(&checkpoint_id),
                    Some(&session_id),
                    &session_turn_ids,
                    interaction_spool.map(spool_has_pending_work),
                );

                let interaction_session = interaction_repository
                    .load_session(&session_id)
                    .with_context(|| {
                        format!(
                            "loading interaction session `{session_id}` from event repository ({session_context})"
                        )
                    })?
                    .ok_or_else(|| {
                        eprintln!(
                            "[bitloops] Warning: missing interaction session for overlapping turns ({session_context})"
                        );
                        anyhow::anyhow!(
                            "missing interaction session for overlapping interaction turns ({session_context})"
                        )
                    })?;
                let latest_session_tree_hash =
                    latest_temporary_checkpoint_tree_hash(&self.repo_root, &session_id);

                self.condense_interaction_session(
                    &interaction_session,
                    &contributing_turns,
                    &checkpoint_id,
                    head,
                    committed_files_set,
                )?;

                for turn in contributing_turns {
                    let remaining_files = files_with_remaining_agent_changes_from_tree(
                        &self.repo_root,
                        latest_session_tree_hash.as_deref(),
                        head,
                        &turn.files_modified,
                        committed_files_set,
                    );
                    let mut updated_turn = turn.clone();
                    updated_turn.updated_at = checkpoint_assigned_at.clone();

                    if remaining_files.is_empty() {
                        updated_turn.checkpoint_id = Some(checkpoint_id.clone());
                    } else {
                        updated_turn.files_modified = remaining_files;
                        updated_turn.checkpoint_id = None;
                    }

                    interaction_repository
                        .upsert_turn(&updated_turn)
                        .with_context(|| {
                            format!(
                                "updating checkpoint progress for interaction turn `{}` in session `{session_id}` ({session_context})",
                                updated_turn.turn_id
                            )
                        })?;
                    if let Some(spool) = interaction_spool {
                        spool.record_turn(&updated_turn).with_context(|| {
                            format!(
                                "mirroring checkpoint progress for interaction turn `{}` into the local spool ({session_context})",
                                updated_turn.turn_id
                            )
                        })?;
                    }
                }

                condensed_any_session = true;
                condensed_sessions.insert(session_id);
            }
        }

        let active_base_skip_sessions = if is_rebase_in_progress {
            pending_turn_sessions.clone()
        } else {
            condensed_sessions.clone()
        };
        update_active_session_base_commits(self.backend.as_ref(), head, &active_base_skip_sessions);

        if condensed_any_session
            && let Some(spool) = interaction_spool
            && let Err(err) = spool.flush(interaction_repository)
        {
            let context = format_post_commit_derivation_context(
                head,
                Some(&checkpoint_id),
                None,
                &[],
                Some(spool_has_pending_work(spool)),
            );
            eprintln!(
                "[bitloops] Warning: failed to flush checkpoint assignments to the interaction event store ({context}): {err:#}"
            );
        }

        Ok(condensed_any_session.then_some(checkpoint_id))
    }
}

fn resolve_interaction_repository_for_post_commit(
    repo_root: &Path,
) -> Result<impl InteractionEventRepository + use<>> {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving store backend config for interaction event repository")?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for interaction event repository")?
        .repo_id;
    create_interaction_repository(&backends.events, repo_root, repo_id)
}

fn spool_has_pending_work(spool: &dyn InteractionSpool) -> bool {
    spool.has_pending_mutations().unwrap_or(false)
        || spool
            .list_uncheckpointed_turns()
            .map(|turns| !turns.is_empty())
            .unwrap_or(false)
}

fn flush_interaction_spool_or_fail(
    head: &str,
    spool: Option<&dyn InteractionSpool>,
    repository: &dyn InteractionEventRepository,
) -> Result<()> {
    let Some(spool) = spool else {
        return Ok(());
    };
    let pending_work = spool_has_pending_work(spool);
    if let Err(err) = spool.flush(repository) {
        let context =
            format_post_commit_derivation_context(head, None, None, &[], Some(pending_work));
        if pending_work {
            eprintln!(
                "[bitloops] Warning: failed to flush interaction spool before post_commit derivation ({context}): {err:#}"
            );
            return Err(err).context(format!(
                "flushing interaction spool before post_commit derivation ({context})"
            ));
        }
        eprintln!(
            "[bitloops] Warning: failed to flush interaction spool before post_commit derivation ({context}): {err:#}"
        );
    }
    Ok(())
}

fn update_active_session_base_commits(
    backend: &dyn crate::host::checkpoints::session::SessionBackend,
    head: &str,
    skip_session_ids: &std::collections::HashSet<String>,
) {
    for mut state in backend.list_sessions().unwrap_or_default() {
        if !state.phase.is_active() || state.base_commit == head {
            continue;
        }
        if skip_session_ids.contains(&state.session_id) {
            continue;
        }
        state.base_commit = head.to_string();
        let _ = backend.save_session(&state);
    }
}

fn open_interaction_spool(repo_root: &Path) -> Result<SqliteInteractionSpool> {
    crate::host::runtime_store::RepoSqliteRuntimeStore::open(repo_root)
        .context("opening repo runtime store for interaction spool")?
        .interaction_spool()
}
