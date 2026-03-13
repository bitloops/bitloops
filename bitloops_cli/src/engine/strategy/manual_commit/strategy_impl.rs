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
            checkpoint_number: state.step_count as i32 + 1,
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
        let default_metadata_dir = paths::session_metadata_dir_from_session_id(&ctx.session_id);
        let mut metadata_dir = if ctx.metadata_dir.trim().is_empty() {
            default_metadata_dir.clone()
        } else {
            ctx.metadata_dir.clone()
        };
        let mut metadata_dir_abs = if ctx.metadata_dir_abs.trim().is_empty() {
            self.repo_root
                .join(&metadata_dir)
                .to_string_lossy()
                .to_string()
        } else {
            ctx.metadata_dir_abs.clone()
        };
        if metadata_dir.trim().is_empty() {
            metadata_dir = default_metadata_dir.clone();
        }
        if metadata_dir_abs.trim().is_empty() {
            metadata_dir_abs = self
                .repo_root
                .join(&metadata_dir)
                .to_string_lossy()
                .to_string();
        }
        // Keep compatibility with direct strategy tests that don't precreate metadata.
        if metadata_dir == default_metadata_dir && !transcript_path.trim().is_empty() {
            let _ = write_session_metadata(&self.repo_root, &ctx.session_id, &transcript_path);
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

        // Commit message with Bitloops metadata trailers.
        let subject = if ctx.commit_message.is_empty() {
            "Bitloops checkpoint".to_string()
        } else {
            ctx.commit_message.clone()
        };
        let commit_msg =
            crate::engine::trailers::format_shadow_commit(&subject, &metadata_dir, &ctx.session_id);

        let result = write_temporary(
            &self.repo_root,
            WriteTemporaryOptions {
                session_id: ctx.session_id.clone(),
                base_commit: state.base_commit.clone(),
                step_number: state.step_count + 1,
                modified_files: modified.clone(),
                new_files: new_files.clone(),
                deleted_files: deleted.clone(),
                metadata_dir,
                metadata_dir_abs,
                commit_message: commit_msg,
                author_name,
                author_email,
                is_first_checkpoint: state.step_count == 0,
            },
        )?;
        if !result.skipped && result.commit_hash.is_empty() {
            anyhow::bail!("temporary checkpoint commit hash is empty");
        }

        // Dedup: skip if tree is identical to the latest temporary checkpoint tree.
        // Still persist token_usage when provided so turn-end can record usage without a new commit.
        if result.skipped {
            if let Some(usage) = &ctx.token_usage {
                state.token_usage = Some(accumulate_token_usage(state.token_usage.take(), usage));
                self.backend.save_session(&state)?;
            }
            return Ok(());
        }

        // Update session state.
        state.base_commit = head;
        state.step_count += 1;
        state.cli_version = CLI_VERSION.to_string();
        state.pending_prompt_attribution = None;
        state.prompt_attributions.push(prompt_attr);
        // Record transcript identifier at the first step.
        if state.step_count == 1 && state.transcript_identifier_at_start.is_empty() {
            state.transcript_identifier_at_start = ctx.step_transcript_identifier.clone();
        }
        if let Some(usage) = &ctx.token_usage {
            state.token_usage = Some(accumulate_token_usage(state.token_usage.take(), usage));
        }
        let all_files: Vec<String> = modified
            .iter()
            .chain(new_files.iter())
            .chain(deleted.iter())
            .cloned()
            .collect();
        merge_files_touched(&mut state.files_touched, &all_files);
        self.backend.save_session(&state)?;

        Ok(())
    }

    /// Persists a task checkpoint as a temporary checkpoint tree.
    ///
    fn save_task_step(&self, ctx: &TaskStepContext) -> Result<()> {
        use super::messages::{format_incremental_subject, format_subagent_end_message};

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

        // Build the full commit message with task-specific trailers.
        let session_metadata_dir = paths::session_metadata_dir_from_session_id(&ctx.session_id);
        let task_metadata_dir = format!("{}/tasks/{}", session_metadata_dir, ctx.tool_use_id);
        let commit_msg = crate::engine::trailers::format_shadow_task_commit(
            &subject,
            &task_metadata_dir,
            &ctx.session_id,
        );

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
                step_number: state.step_count,
                tool_use_id: ctx.tool_use_id.clone(),
                agent_id: ctx.agent_id.clone(),
                modified_files: ctx.modified_files.clone(),
                new_files: ctx.new_files.clone(),
                deleted_files: ctx.deleted_files.clone(),
                transcript_path: ctx.transcript_path.clone(),
                subagent_transcript_path: ctx.subagent_transcript_path.clone(),
                checkpoint_uuid: ctx.checkpoint_uuid.clone(),
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

        // Update session state — accumulate files_touched but don't bump step_count
        // (task checkpoints are subordinate to regular turn checkpoints).
        let all_files: Vec<String> = ctx
            .modified_files
            .iter()
            .chain(ctx.new_files.iter())
            .chain(ctx.deleted_files.iter())
            .cloned()
            .collect();
        merge_files_touched(&mut state.files_touched, &all_files);
        self.backend.save_session(&state)?;

        Ok(())
    }

    fn handle_turn_end(&self, state: &mut SessionState) -> Result<()> {
        self.finalize_all_turn_checkpoints(state);
        Ok(())
    }

    /// Appends a `Bitloops-Checkpoint: <id>` trailer to the commit message file.
    ///
    /// Called by the `prepare-commit-msg` git hook.
    ///
    fn prepare_commit_msg(&self, commit_msg_file: &Path, source: Option<&str>) -> Result<()> {
        let source = source.unwrap_or("");

        // Skip during git sequence operations (rebase, cherry-pick, revert).
        if is_git_sequence_operation(&self.repo_root) {
            return Ok(());
        }

        // Skip for auto-generated messages.
        if source == "merge" || source == "squash" {
            return Ok(());
        }

        // For amend: preserve or restore trailer.
        if source == "commit" {
            return self.handle_amend_commit_msg(commit_msg_file);
        }

        let sessions = self.backend.list_sessions().unwrap_or_default();
        if sessions.is_empty() {
            return Ok(());
        }

        let staged_files = get_staged_files(&self.repo_root);

        // Only attach a fresh trailer when there is pending session work
        // and the staged content overlaps with files touched by the session.
        let has_pending_session_content = sessions.iter().any(|s| {
            let has_pending = s.phase.is_active()
                || !s.files_touched.is_empty()
                || (s.phase != SessionPhase::Ended && s.step_count > 0);
            if !has_pending {
                return false;
            }
            if s.files_touched.is_empty() {
                return true;
            }
            if staged_files.is_empty() {
                return true;
            }
            let shadow_branch = expected_shadow_branch_short_name(&s.base_commit, &s.worktree_id);
            staged_files_overlap_with_content(
                &self.repo_root,
                &shadow_branch,
                &staged_files,
                &s.files_touched,
            )
        });
        if !has_pending_session_content {
            return Ok(());
        }

        // Read current commit message.
        let content = fs::read_to_string(commit_msg_file).unwrap_or_default();

        // If trailer already present, keep it.
        if parse_checkpoint_id(&content).is_some() {
            return Ok(());
        }

        // Generate a new checkpoint ID and append the trailer.
        let id = generate_checkpoint_id();
        let new_content = add_checkpoint_trailer(&content, &id);
        fs::write(commit_msg_file, new_content)
            .with_context(|| format!("writing commit msg file: {}", commit_msg_file.display()))?;

        Ok(())
    }

    /// Strips the checkpoint trailer when the commit message has no user content.
    ///
    /// Called by the `commit-msg` git hook.  Returning an error causes git to abort.
    ///
    fn commit_msg(&self, commit_msg_file: &Path) -> Result<()> {
        let content = match fs::read_to_string(commit_msg_file) {
            Ok(c) => c,
            Err(_) => return Ok(()), // be silent on failure
        };

        // Only act when our trailer is present.
        if parse_checkpoint_id(&content).is_none() {
            return Ok(());
        }

        // Strip the trailer if there's no real user content.
        if !has_user_content(&content) {
            let stripped = strip_checkpoint_trailer(&content);
            let _ = fs::write(commit_msg_file, stripped);
        }

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
        // Extract checkpoint ID from HEAD commit.
        let checkpoint_id = match get_checkpoint_id_from_head(&self.repo_root)? {
            Some(id) => id,
            None => {
                // No trailer — just update base_commit for active sessions.
                self.update_base_commit_for_active_sessions()?;
                return Ok(());
            }
        };

        let committed_files = files_changed_in_commit(&self.repo_root, &head).unwrap_or_default();
        let sessions = self.backend.list_sessions().unwrap_or_default();
        let mut shadow_branches_to_delete: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut preserved_shadow_branches: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();

        for mut state in sessions {
            let shadow_branch_before =
                expected_shadow_branch_short_name(&state.base_commit, &state.worktree_id);
            let mut condensed = false;
            let has_pending = state.phase.is_active()
                || state.phase == SessionPhase::Ended
                || !state.files_touched.is_empty()
                || state.step_count > 0;
            if !has_pending {
                continue;
            }

            let transition_actions =
                self.apply_git_commit_transition(&mut state, is_rebase_in_progress);
            let should_condense = transition_actions
                .iter()
                .any(|action| matches!(action, Action::Condense | Action::CondenseIfFilesTouched));
            let should_update_active_base = transition_actions
                .iter()
                .any(|action| matches!(action, Action::DiscardIfNoFiles));

            if should_condense {
                if !state.files_touched.is_empty() {
                    let committed_touched: Vec<String> = state
                        .files_touched
                        .iter()
                        .filter(|f| committed_files.contains(f.as_str()))
                        .cloned()
                        .collect();
                    if committed_touched.is_empty() {
                        if state.phase.is_active() && state.base_commit != head {
                            state.base_commit = head.clone();
                        }
                        let _ = self.backend.save_session(&state);
                        continue;
                    }
                    if !files_overlap_with_content(
                        &self.repo_root,
                        &shadow_branch_before,
                        &head,
                        &committed_touched,
                    ) {
                        if state.phase.is_active() && state.base_commit != head {
                            state.base_commit = head.clone();
                        }
                        let _ = self.backend.save_session(&state);
                        continue;
                    }
                }

                if let Err(e) = self.condense_session(&mut state, &checkpoint_id, &head) {
                    eprintln!(
                        "[bitloops] Warning: condensation failed for session {}: {e}",
                        state.session_id
                    );
                    let _ = self.backend.save_session(&state);
                    continue;
                }
                condensed = true;

                // ACTIVE sessions track all turn checkpoint IDs for stop-time finalization.
                if state.phase.is_active() {
                    state.turn_checkpoint_ids.push(checkpoint_id.clone());
                }
            } else if should_update_active_base
                && state.phase.is_active()
                && state.base_commit != head
            {
                state.base_commit = head.clone();
            }

            if condensed && !shadow_branch_before.is_empty() {
                // Condensed branches are eligible for cleanup; keep any
                // branch that still carries uncommitted files.
                if state.files_touched.is_empty() {
                    shadow_branches_to_delete.insert(shadow_branch_before.clone());
                } else {
                    preserved_shadow_branches.insert(shadow_branch_before.clone());
                }
            }
            if state.phase.is_active() && !condensed && !shadow_branch_before.is_empty() {
                preserved_shadow_branches.insert(shadow_branch_before.clone());
            }

            let _ = self.backend.save_session(&state);
        }

        for branch in shadow_branches_to_delete {
            if preserved_shadow_branches.contains(&branch) {
                continue;
            }
            let _ = run_git(&self.repo_root, &["branch", "-D", &branch]);
        }

        Ok(())
    }

    /// Pushes `bitloops/checkpoints/v1` alongside the user's push.
    ///
    /// Called by the `pre-push` git hook.
    ///
    fn pre_push(&self, remote: &str) -> Result<()> {
        // Only push if the checkpoints branch exists.
        if run_git(&self.repo_root, &["rev-parse", paths::METADATA_BRANCH_NAME]).is_ok() {
            // Non-fatal: push failure must not block the user's push.
            // Use --no-verify to avoid recursive pre-push hook execution.
            let _ = push_checkpoints_branch_no_verify(&self.repo_root, remote);
        }
        Ok(())
    }
}
