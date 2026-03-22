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
        let legacy_metadata_enabled = crate::host::session::legacy_local_backend_enabled();
        let default_metadata_dir = if legacy_metadata_enabled {
            paths::session_metadata_dir_from_session_id(&ctx.session_id)
        } else {
            String::new()
        };
        let mut metadata_dir = ctx.metadata_dir.trim().to_string();
        let mut metadata_dir_abs = ctx.metadata_dir_abs.trim().to_string();

        if legacy_metadata_enabled && metadata_dir.is_empty() {
            metadata_dir = default_metadata_dir.clone();
        }
        if legacy_metadata_enabled && metadata_dir_abs.is_empty() && !metadata_dir.is_empty() {
            metadata_dir_abs = self
                .repo_root
                .join(&metadata_dir)
                .to_string_lossy()
                .to_string();
        }
        if !legacy_metadata_enabled && (metadata_dir.is_empty() || metadata_dir_abs.is_empty()) {
            metadata_dir.clear();
            metadata_dir_abs.clear();
        }
        // Legacy compatibility: only materialise session metadata files when explicitly enabled.
        if legacy_metadata_enabled
            && metadata_dir == default_metadata_dir
            && !transcript_path.trim().is_empty()
        {
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
        if commit_has_checkpoint_mapping(&self.repo_root, &head)? {
            return Ok(());
        }

        let committed_files = files_changed_in_commit(&self.repo_root, &head).unwrap_or_default();
        let sessions = self.backend.list_sessions().unwrap_or_default();
        let checkpoint_id = generate_checkpoint_id();
        let mut condensed_any_session = false;

        for mut state in sessions {
            let has_pending = state.phase == SessionPhase::Ended
                || !state.files_touched.is_empty()
                || state.step_count > 0;
            if !has_pending {
                if state.phase.is_active() && state.base_commit != head {
                    state.base_commit = head.clone();
                    let _ = self.backend.save_session(&state);
                }
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
                }

                if let Err(e) = self.condense_session(&mut state, &checkpoint_id, &head) {
                    eprintln!(
                        "[bitloops] Warning: condensation failed for session {}: {e}",
                        state.session_id
                    );
                    let _ = self.backend.save_session(&state);
                    continue;
                }
                condensed_any_session = true;

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

            let _ = self.backend.save_session(&state);
        }

        if condensed_any_session {
            insert_commit_checkpoint_mapping(&self.repo_root, &head, &checkpoint_id)?;
        }

        Ok(())
    }

    /// Called by the `pre-push` git hook.
    /// This is intentionally a no-op.
    fn pre_push(&self, _remote: &str) -> Result<()> {
        Ok(())
    }
}

fn open_commit_checkpoint_mapping_db(
    repo_root: &Path,
) -> Result<(crate::storage::SqliteConnectionPool, String)> {
    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root)
        .context("resolving SQLite path for commit_checkpoints")?;
    let sqlite = crate::storage::SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening SQLite for commit_checkpoints")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for commit_checkpoints")?;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for commit_checkpoints")?
        .repo_id;
    Ok((sqlite, repo_id))
}

fn commit_has_checkpoint_mapping(repo_root: &Path, commit_sha: &str) -> Result<bool> {
    use rusqlite::OptionalExtension;

    let (sqlite, repo_id) = open_commit_checkpoint_mapping_db(repo_root)?;
    sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT 1
             FROM commit_checkpoints
             WHERE commit_sha = ?1 AND repo_id = ?2
             LIMIT 1",
            rusqlite::params![commit_sha, repo_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|hit| hit.is_some())
        .map_err(anyhow::Error::from)
    })
}

fn insert_commit_checkpoint_mapping(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let (sqlite, repo_id) = open_commit_checkpoint_mapping_db(repo_root)?;
    sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![commit_sha, checkpoint_id, repo_id],
        )
        .context("inserting commit_checkpoints row")?;
        Ok(())
    })
}
