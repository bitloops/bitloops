use super::*;

// ── Private helpers ───────────────────────────────────────────────────────────

impl ManualCommitStrategy {
    pub(crate) fn initialize_or_refresh_session(
        &self,
        session_id: &str,
        agent_type: &str,
        transcript_path: &str,
        user_prompt: &str,
    ) -> Result<()> {
        let Some(head) = try_head_hash(&self.repo_root)? else {
            anyhow::bail!("failed to initialize session: HEAD is missing");
        };

        let mut state = match self.backend.load_session(session_id)? {
            Some(existing) if !existing.base_commit.is_empty() => existing,
            _ => SessionState {
                session_id: session_id.to_string(),
                cli_version: CLI_VERSION.to_string(),
                base_commit: head.clone(),
                attribution_base_commit: head.clone(),
                worktree_path: self.repo_root.to_string_lossy().to_string(),
                worktree_id: paths::get_worktree_id(&self.repo_root)?,
                started_at: now_rfc3339(),
                phase: SessionPhase::Active,
                turn_id: generate_checkpoint_id(),
                step_count: 0,
                untracked_files_at_start: collect_untracked_files_at_start(&self.repo_root),
                agent_type: canonicalize_agent_type(agent_type),
                transcript_path: transcript_path.to_string(),
                first_prompt: truncate_prompt_for_storage(user_prompt),
                ..Default::default()
            },
        };

        state.phase = SessionPhase::Active;
        state.last_interaction_time = Some(now_rfc3339());
        state.turn_id = generate_checkpoint_id();

        if !agent_type.trim().is_empty()
            && (state.agent_type.trim().is_empty()
                || state.agent_type
                    == canonicalize_agent_type(crate::adapters::agents::AGENT_TYPE_UNKNOWN))
        {
            state.agent_type = canonicalize_agent_type(agent_type);
        }
        if state.agent_type.trim().is_empty() {
            state.agent_type = AGENT_TYPE_CLAUDE_CODE.to_string();
        }
        if state.first_prompt.trim().is_empty() && !user_prompt.trim().is_empty() {
            state.first_prompt = truncate_prompt_for_storage(user_prompt);
        }
        if !transcript_path.trim().is_empty() && state.transcript_path != transcript_path {
            state.transcript_path = transcript_path.to_string();
        }

        state.last_checkpoint_id.clear();
        state.turn_checkpoint_ids.clear();

        state.pending_prompt_attribution = Some(self.calculate_prompt_attribution_at_start(&state));

        self.backend.save_session(&state)
    }

    fn calculate_prompt_attribution_at_start(
        &self,
        state: &SessionState,
    ) -> SessionPromptAttribution {
        let checkpoint_number = state.step_count as i32 + 1;
        let mut changed_files: BTreeMap<String, String> = BTreeMap::new();
        if let Ok((modified, new_files, deleted)) = working_tree_changes(&self.repo_root) {
            for file in modified.into_iter().chain(new_files).chain(deleted) {
                let path = self.repo_root.join(&file);
                let content = match fs::read(&path) {
                    Ok(bytes) => {
                        if bytes.contains(&0) {
                            String::new()
                        } else {
                            String::from_utf8_lossy(&bytes).to_string()
                        }
                    }
                    Err(_) => String::new(),
                };
                changed_files.insert(file, content);
            }
        }

        let base_tree = if state.base_commit.trim().is_empty() {
            None
        } else {
            load_tree_snapshot(&self.repo_root, &state.base_commit)
        };
        let last_checkpoint_tree =
            latest_temporary_checkpoint_tree_hash(&self.repo_root, &state.session_id)
                .and_then(|tree_hash| load_tree_snapshot(&self.repo_root, &tree_hash));

        let attr = calculate_prompt_attribution(
            base_tree.as_ref(),
            last_checkpoint_tree.as_ref(),
            &changed_files,
            checkpoint_number,
        );
        SessionPromptAttribution {
            checkpoint_number: attr.checkpoint_number,
            user_lines_added: attr.user_lines_added,
            user_lines_removed: attr.user_lines_removed,
            agent_lines_added: attr.agent_lines_added,
            agent_lines_removed: attr.agent_lines_removed,
            user_added_per_file: attr.user_added_per_file,
        }
    }

    pub(crate) fn finalize_all_turn_checkpoints(&self, state: &mut SessionState) {
        if state.turn_checkpoint_ids.is_empty() {
            return;
        }
        if state.transcript_path.trim().is_empty() {
            state.turn_checkpoint_ids.clear();
            return;
        }

        let Ok(full_transcript) = fs::read(&state.transcript_path) else {
            state.turn_checkpoint_ids.clear();
            return;
        };
        if full_transcript.is_empty() {
            state.turn_checkpoint_ids.clear();
            return;
        }

        let transcript_text = String::from_utf8_lossy(&full_transcript).to_string();
        let prompts = extract_user_prompts_from_jsonl(&transcript_text);
        let context = generate_context_from_prompts(&prompts).into_bytes();

        for checkpoint_id in state.turn_checkpoint_ids.clone() {
            let _ = update_committed(
                &self.repo_root,
                UpdateCommittedOptions {
                    checkpoint_id,
                    session_id: state.session_id.clone(),
                    transcript: Some(full_transcript.clone()),
                    prompts: Some(prompts.clone()),
                    context: Some(context.clone()),
                    agent: state.agent_type.clone(),
                },
            );
        }
        state.turn_checkpoint_ids.clear();
    }

    /// Condenses session work into committed checkpoint rows/blobs.
    ///
    pub(crate) fn condense_session(
        &self,
        state: &mut SessionState,
        checkpoint_id: &str,
        new_head: &str,
    ) -> Result<()> {
        let committed_files =
            files_changed_in_commit(&self.repo_root, new_head).unwrap_or_default();
        let had_tracked_files = !state.files_touched.is_empty();
        let committed_touched: Vec<String> = if had_tracked_files {
            state
                .files_touched
                .iter()
                .filter(|f| committed_files.contains(f.as_str()))
                .cloned()
                .collect()
        } else {
            let mut fallback: Vec<String> = committed_files.iter().cloned().collect();
            fallback.sort();
            fallback
        };
        let latest_session_tree_hash =
            latest_temporary_checkpoint_tree_hash(&self.repo_root, &state.session_id);
        let initial_attribution = calculate_session_initial_attribution(
            &self.repo_root,
            state,
            latest_session_tree_hash.as_deref(),
            new_head,
            &committed_touched,
        );
        let transcript_content = read_transcript_from_disk(&self.repo_root, &state.session_id)
            .or_else(|| {
                if state.transcript_path.trim().is_empty() {
                    return None;
                }
                fs::read_to_string(&state.transcript_path)
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
        let total_transcript_lines = transcript_content.lines().count() as i64;
        let prompts = extract_user_prompts_from_jsonl(&transcript_content);
        let context = generate_context_from_prompts(&prompts).into_bytes();
        let token_usage = state
            .token_usage
            .as_ref()
            .map(token_usage_metadata_from_runtime)
            .or_else(|| {
                calculate_token_usage_from_transcript(
                    &transcript_content,
                    state.checkpoint_transcript_start,
                )
            });
        let (author_name, author_email) = get_commit_author(&self.repo_root, new_head)
            .unwrap_or(get_git_author_from_repo(&self.repo_root)?);

        // Auto-summarize if enabled in settings.
        // Non-blocking: failure logs a warning and continues without a summary.
        let summary: Option<serde_json::Value> = {
            let settings =
                crate::config::settings::load_settings(&self.repo_root).unwrap_or_default();
            if settings.is_summarize_enabled() && !transcript_content.is_empty() {
                let summarize_agent = match state.agent_type.as_str() {
                    s if s == AGENT_TYPE_GEMINI => {
                        crate::host::checkpoints::summarize::AgentType::Gemini
                    }
                    s if s == AGENT_TYPE_OPEN_CODE => {
                        crate::host::checkpoints::summarize::AgentType::OpenCode
                    }
                    s if s == AGENT_TYPE_CLAUDE_CODE || s == AGENT_TYPE_CODEX => {
                        crate::host::checkpoints::summarize::AgentType::ClaudeCode
                    }
                    _ => crate::host::checkpoints::summarize::AgentType::Unknown,
                };
                let scoped = crate::host::checkpoints::summarize::scope_transcript_for_checkpoint(
                    transcript_content.as_bytes(),
                    state.checkpoint_transcript_start.max(0) as usize,
                    summarize_agent,
                );
                if !scoped.is_empty() {
                    match crate::host::checkpoints::summarize::generate_from_transcript(
                        &scoped,
                        &state.files_touched,
                        summarize_agent,
                        None,
                    ) {
                        Ok(s) => serde_json::to_value(s).ok(),
                        Err(e) => {
                            eprintln!(
                                "[warn] summary generation failed session_id={} error={e}",
                                state.session_id
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        write_committed(
            &self.repo_root,
            WriteCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: state.session_id.clone(),
                strategy: "manual-commit".to_string(),
                agent: state.agent_type.clone(),
                transcript: transcript_content.clone().into_bytes(),
                prompts: Some(prompts),
                context: Some(context),
                checkpoints_count: state.step_count,
                files_touched: committed_touched,
                token_usage_input: None,
                token_usage_output: None,
                token_usage_api_call_count: None,
                turn_id: state.turn_id.clone(),
                transcript_identifier_at_start: state.transcript_identifier_at_start.clone(),
                checkpoint_transcript_start: state.checkpoint_transcript_start,
                token_usage,
                initial_attribution,
                author_name,
                author_email,
                summary,
                is_task: false,
                tool_use_id: String::new(),
                agent_id: String::new(),
                transcript_path: state.transcript_path.clone(),
                subagent_transcript_path: String::new(),
            },
        )?;

        let remaining_files = files_with_remaining_agent_changes_from_tree(
            &self.repo_root,
            latest_session_tree_hash.as_deref(),
            new_head,
            &state.files_touched,
            &committed_files,
        );

        // Update session state.
        state.base_commit = new_head.to_string();
        state.attribution_base_commit = new_head.to_string();
        state.step_count = 0;
        state.checkpoint_transcript_start = total_transcript_lines;
        state.files_touched = remaining_files;
        state.transcript_identifier_at_start.clear();
        state.last_checkpoint_id = checkpoint_id.to_string();
        state.prompt_attributions.clear();
        state.pending_prompt_attribution = None;
        self.backend.save_session(state)?;

        eprintln!(
            "[bitloops] Condensed session {}: checkpoint {checkpoint_id}",
            &state.session_id[..state.session_id.len().min(8)]
        );

        Ok(())
    }

    pub(crate) fn condense_interaction_session(
        &self,
        session: &crate::host::interactions::types::InteractionSession,
        turns: &[crate::host::interactions::types::InteractionTurn],
        checkpoint_id: &str,
        new_head: &str,
        committed_files: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>> {
        let session_files = aggregate_turn_files(turns);
        let committed_touched: Vec<String> = session_files
            .iter()
            .filter(|file| committed_files.contains(file.as_str()))
            .cloned()
            .collect();
        if committed_touched.is_empty() {
            anyhow::bail!(
                "interaction session {} has no overlap with committed files",
                session.session_id
            );
        }

        let latest_session_tree_hash =
            latest_temporary_checkpoint_tree_hash(&self.repo_root, &session.session_id);
        let mut local_state = self.backend.load_session(&session.session_id)?;
        let initial_attribution = local_state.as_ref().and_then(|state| {
            calculate_session_initial_attribution(
                &self.repo_root,
                state,
                latest_session_tree_hash.as_deref(),
                new_head,
                &committed_touched,
            )
        });

        let full_transcript_content =
            load_interaction_session_transcript(&self.repo_root, session).unwrap_or_default();
        let total_transcript_lines = full_transcript_content.lines().count() as i64;
        let transcript_content = scope_interaction_transcript_for_turns(
            &full_transcript_content,
            turns,
            &session.session_id,
        );

        let mut prompts = aggregate_turn_prompts(turns);
        if prompts.is_empty() {
            prompts = extract_user_prompts_from_jsonl(&transcript_content);
        }
        if prompts.is_empty() && !session.first_prompt.trim().is_empty() {
            prompts.push(session.first_prompt.clone());
        }
        let context = generate_context_from_prompts(&prompts).into_bytes();
        let token_usage = aggregate_turn_token_usage(turns)
            .or_else(|| calculate_token_usage_from_transcript(&transcript_content, 0));
        let (author_name, author_email) = get_commit_author(&self.repo_root, new_head)
            .unwrap_or(get_git_author_from_repo(&self.repo_root)?);

        let resolved_agent = session
            .agent_type
            .trim()
            .is_empty()
            .then(|| {
                turns.iter().find_map(|turn| {
                    let candidate = turn.agent_type.trim();
                    (!candidate.is_empty()).then(|| canonicalize_agent_type(candidate))
                })
            })
            .flatten()
            .unwrap_or_else(|| {
                if session.agent_type.trim().is_empty() {
                    AGENT_TYPE_CLAUDE_CODE.to_string()
                } else {
                    canonicalize_agent_type(&session.agent_type)
                }
            });

        let summary: Option<serde_json::Value> = {
            let settings =
                crate::config::settings::load_settings(&self.repo_root).unwrap_or_default();
            if settings.is_summarize_enabled() && !transcript_content.is_empty() {
                let summarize_agent = match resolved_agent.as_str() {
                    s if s == AGENT_TYPE_GEMINI => {
                        crate::host::checkpoints::summarize::AgentType::Gemini
                    }
                    s if s == AGENT_TYPE_OPEN_CODE => {
                        crate::host::checkpoints::summarize::AgentType::OpenCode
                    }
                    s if s == AGENT_TYPE_CLAUDE_CODE || s == AGENT_TYPE_CODEX => {
                        crate::host::checkpoints::summarize::AgentType::ClaudeCode
                    }
                    _ => crate::host::checkpoints::summarize::AgentType::Unknown,
                };
                let scoped = crate::host::checkpoints::summarize::scope_transcript_for_checkpoint(
                    transcript_content.as_bytes(),
                    0,
                    summarize_agent,
                );
                if !scoped.is_empty() {
                    match crate::host::checkpoints::summarize::generate_from_transcript(
                        &scoped,
                        &committed_touched,
                        summarize_agent,
                        None,
                    ) {
                        Ok(summary) => serde_json::to_value(summary).ok(),
                        Err(err) => {
                            eprintln!(
                                "[warn] summary generation failed session_id={} error={err}",
                                session.session_id
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        let turn_id = turns
            .last()
            .map(|turn| turn.turn_id.clone())
            .unwrap_or_default();
        write_committed(
            &self.repo_root,
            WriteCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: session.session_id.clone(),
                strategy: "manual-commit".to_string(),
                agent: resolved_agent.clone(),
                transcript: transcript_content.clone().into_bytes(),
                prompts: Some(prompts),
                context: Some(context),
                checkpoints_count: turns.len().min(u32::MAX as usize) as u32,
                files_touched: committed_touched.clone(),
                token_usage_input: None,
                token_usage_output: None,
                token_usage_api_call_count: None,
                turn_id: turn_id.clone(),
                transcript_identifier_at_start: String::new(),
                checkpoint_transcript_start: 0,
                token_usage,
                initial_attribution,
                author_name,
                author_email,
                summary,
                is_task: false,
                tool_use_id: String::new(),
                agent_id: String::new(),
                transcript_path: session.transcript_path.clone(),
                subagent_transcript_path: String::new(),
            },
        )?;

        let remaining_files = files_with_remaining_agent_changes_from_tree(
            &self.repo_root,
            latest_session_tree_hash.as_deref(),
            new_head,
            &session_files,
            committed_files,
        );

        if let Some(mut state) = local_state.take() {
            if state.started_at.trim().is_empty() && !session.started_at.trim().is_empty() {
                state.started_at = session.started_at.clone();
            }
            if state.ended_at.is_none() && session.ended_at.is_some() {
                state.ended_at = session.ended_at.clone();
            }
            if state.worktree_path.trim().is_empty() && !session.worktree_path.trim().is_empty() {
                state.worktree_path = session.worktree_path.clone();
            }
            if state.worktree_id.trim().is_empty() && !session.worktree_id.trim().is_empty() {
                state.worktree_id = session.worktree_id.clone();
            }
            if state.transcript_path.trim().is_empty() && !session.transcript_path.trim().is_empty()
            {
                state.transcript_path = session.transcript_path.clone();
            }
            if state.first_prompt.trim().is_empty() && !session.first_prompt.trim().is_empty() {
                state.first_prompt = session.first_prompt.clone();
            }
            if !resolved_agent.trim().is_empty() {
                state.agent_type = resolved_agent;
            }
            if !session.last_event_at.trim().is_empty() {
                state.last_interaction_time = Some(session.last_event_at.clone());
            }

            state.base_commit = new_head.to_string();
            state.attribution_base_commit = new_head.to_string();
            state.step_count = 0;
            state.turn_id = turn_id;
            state.checkpoint_transcript_start = aggregate_turn_transcript_bounds(turns)
                .map(|(_, end)| end as i64)
                .unwrap_or(total_transcript_lines);
            state.files_touched = remaining_files;
            state.transcript_identifier_at_start.clear();
            state.last_checkpoint_id = checkpoint_id.to_string();
            state.prompt_attributions.clear();
            state.pending_prompt_attribution = None;
            if state.phase.is_active() {
                state.turn_checkpoint_ids.push(checkpoint_id.to_string());
            }
            self.backend.save_session(&state)?;
        }

        eprintln!(
            "[bitloops] Condensed interaction session {}: checkpoint {checkpoint_id}",
            &session.session_id[..session.session_id.len().min(8)]
        );

        Ok(turns.iter().map(|turn| turn.turn_id.clone()).collect())
    }

    /// Updates `base_commit` to HEAD for all active sessions.
    #[allow(dead_code)]
    pub(crate) fn update_base_commit_for_active_sessions(&self) -> Result<()> {
        let Some(head) = try_head_hash(&self.repo_root)? else {
            return Ok(());
        };
        let sessions = self.backend.list_sessions().unwrap_or_default();
        for mut state in sessions {
            if !state.phase.is_active() {
                continue;
            }
            if state.base_commit != head {
                state.base_commit = head.clone();
                let _ = self.backend.save_session(&state);
            }
        }
        Ok(())
    }
}

fn load_interaction_session_transcript(
    repo_root: &Path,
    session: &crate::host::interactions::types::InteractionSession,
) -> Option<String> {
    read_transcript_from_disk(repo_root, &session.session_id).or_else(|| {
        if session.transcript_path.trim().is_empty() {
            return None;
        }
        fs::read_to_string(&session.transcript_path)
            .ok()
            .filter(|content| !content.is_empty())
    })
}

fn scope_interaction_transcript_for_turns(
    full_transcript: &str,
    turns: &[crate::host::interactions::types::InteractionTurn],
    session_id: &str,
) -> String {
    if full_transcript.is_empty() {
        return String::new();
    }

    let Some((start, end)) = aggregate_turn_transcript_bounds(turns) else {
        eprintln!(
            "[bitloops] Warning: falling back to full transcript during checkpoint condensation session_id={} reason=missing_turn_offsets",
            session_id
        );
        return full_transcript.to_string();
    };

    let lines: Vec<&str> = full_transcript.split_inclusive('\n').collect();
    if start >= end || end > lines.len() {
        eprintln!(
            "[bitloops] Warning: falling back to full transcript during checkpoint condensation session_id={} reason=invalid_turn_offsets start={} end={} line_count={}",
            session_id,
            start,
            end,
            lines.len()
        );
        return full_transcript.to_string();
    }

    lines[start..end].concat()
}
