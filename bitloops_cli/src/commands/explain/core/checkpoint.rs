/// Builds a commit graph by running `git log` and parsing each line.
/// When `limit` is 0 the walk is unlimited.
fn build_commit_graph_from_git(
    repo_root: &std::path::Path,
    limit: usize,
) -> Result<Vec<CommitNode>> {
    let format = format!(
        "--format=%H|%P|%an|%ct|%s|%(trailers:key={CHECKPOINT_TRAILER_KEY},valueonly=true,separator=%x00)"
    );
    let mut args: Vec<String> = vec!["log".to_string(), format];
    if limit > 0 {
        args.push("--max-count".to_string());
        args.push(limit.to_string());
    }
    args.push("HEAD".to_string());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run_git(repo_root, &arg_refs)?;

    let mut nodes = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(6, '|').collect();
        if parts.len() < 5 {
            continue;
        }
        let sha = parts[0].trim().to_string();
        let parents: Vec<String> = parts[1]
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let author = parts[2].trim().to_string();
        let timestamp: i64 = parts[3].trim().parse().unwrap_or(0);
        let message = parts[4].trim().to_string();
        let trailer_raw = if parts.len() > 5 { parts[5] } else { "" };

        let mut trailers: HashMap<String, String> = HashMap::new();
        // The %(trailers:...) format may emit empty lines or newlines for missing trailers.
        let cp_id = trailer_raw
            .split('\x00')
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("")
            .to_string();
        if !cp_id.is_empty() {
            trailers.insert(CHECKPOINT_TRAILER_KEY.to_string(), cp_id);
        }

        nodes.push(CommitNode {
            sha,
            parents,
            author,
            timestamp,
            message,
            trailers,
            files_changed: Vec::new(),
        });
    }

    Ok(nodes)
}

/// Helper to convert `SessionContentView.metadata` (a JSON Value) into `CheckpointMetadata`.
fn metadata_from_json(meta: &serde_json::Value, checkpoint_id: &str) -> CheckpointMetadata {
    let session_id = meta
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let created_at = meta
        .get("created_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let files_touched: Vec<String> = meta
        .get("files_touched")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let checkpoints_count = meta
        .get("checkpoints_count")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let checkpoint_transcript_start = meta
        .get("checkpoint_transcript_start")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    // token usage comes from nested object
    let (has_token_usage, token_input, token_output) = if let Some(tu) = meta.get("token_usage") {
        let input = tu.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        let output = tu.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        (token_usage_json_has_values(tu), input, output)
    } else {
        (false, 0u64, 0u64)
    };

    let summary = meta.get("summary").and_then(parse_summary_details);

    // Parse agent type from session-level metadata.
    let agent_type = meta
        .get("agent")
        .and_then(Value::as_str)
        .map(agent_type_from_str)
        .unwrap_or_default();

    CheckpointMetadata {
        checkpoint_id: checkpoint_id.to_string(),
        session_id,
        created_at,
        files_touched,
        checkpoints_count,
        checkpoint_transcript_start,
        has_token_usage,
        token_input,
        token_output,
        summary,
        agent_type,
    }
}

fn token_usage_json_has_values(token_usage: &serde_json::Value) -> bool {
    let keys = [
        "input_tokens",
        "output_tokens",
        "cache_creation_tokens",
        "cache_read_tokens",
        "api_call_count",
    ];
    if keys
        .iter()
        .any(|key| token_usage.get(key).and_then(Value::as_u64).unwrap_or(0) > 0)
    {
        return true;
    }

    token_usage
        .get("subagent_tokens")
        .map(token_usage_json_has_values)
        .unwrap_or(false)
}

fn token_usage_metadata_has_values(
    token_usage: &crate::engine::strategy::manual_commit::TokenUsageMetadata,
) -> bool {
    if token_usage.input_tokens > 0
        || token_usage.output_tokens > 0
        || token_usage.cache_creation_tokens > 0
        || token_usage.cache_read_tokens > 0
        || token_usage.api_call_count > 0
    {
        return true;
    }

    token_usage
        .subagent_tokens
        .as_deref()
        .map(token_usage_metadata_has_values)
        .unwrap_or(false)
}

/// Converts the `"agent"` string stored in committed metadata to an `AgentType`.
/// Uses canonical agent keys ("claude-code", "gemini-cli", "opencode", "cursor").
fn agent_type_from_str(s: &str) -> AgentType {
    use crate::engine::agent::{AGENT_TYPE_CURSOR, AGENT_TYPE_GEMINI, AGENT_TYPE_OPEN_CODE};
    match s {
        s if s == AGENT_TYPE_CURSOR => AgentType::Cursor,
        s if s == AGENT_TYPE_GEMINI => AgentType::Gemini,
        s if s == AGENT_TYPE_OPEN_CODE => AgentType::OpenCode,
        _ => AgentType::ClaudeCode,
    }
}

/// Parses a JSON summary value into `SummaryDetails`.
fn parse_summary_details(v: &serde_json::Value) -> Option<SummaryDetails> {
    // If null or not an object, return None.
    if !v.is_object() {
        return None;
    }
    let intent = v
        .get("intent")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let outcome = v
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let repo_learnings: Vec<String> = v
        .get("learnings")
        .and_then(|l| l.get("repo"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let workflow_learnings: Vec<String> = v
        .get("learnings")
        .and_then(|l| l.get("workflow"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let code_learnings: Vec<CodeLearning> = v
        .get("learnings")
        .and_then(|l| l.get("code"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_object)
                .map(|obj| CodeLearning {
                    path: obj
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    line: obj.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                    end_line: obj.get("end_line").and_then(Value::as_u64).unwrap_or(0) as usize,
                    finding: obj
                        .get("finding")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let friction: Vec<String> = v
        .get("friction")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let open_items: Vec<String> = v
        .get("open_items")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some(SummaryDetails {
        intent,
        outcome,
        repo_learnings,
        code_learnings,
        workflow_learnings,
        friction,
        open_items,
    })
}

// CLI-852 / CLI-853 / CLI-854: checkpoint explain flow
pub fn run_explain_checkpoint(
    checkpoint_id_prefix: &str,
    opts: &ExplainExecutionOptions,
) -> Result<String> {
    let repo_root = paths::repo_root()?;
    run_explain_checkpoint_in(&repo_root, checkpoint_id_prefix, opts)
}

pub(crate) fn run_explain_checkpoint_in(
    repo_root: &std::path::Path,
    checkpoint_id_prefix: &str,
    opts: &ExplainExecutionOptions,
) -> Result<String> {
    if checkpoint_id_prefix.is_empty() {
        bail!("checkpoint not found")
    }

    let all = list_committed(repo_root)?;

    let committed_matches: Vec<_> = all
        .iter()
        .filter(|c| c.checkpoint_id.starts_with(checkpoint_id_prefix))
        .collect();

    let full_checkpoint_id = match committed_matches.as_slice() {
        [] => {
            if opts.generate {
                bail!(
                    "cannot generate summary for temporary checkpoint {} (only committed checkpoints supported)",
                    checkpoint_id_prefix
                );
            }
            return match explain_temporary_checkpoint_real(repo_root, checkpoint_id_prefix, opts) {
                Ok(Some(output)) => Ok(output),
                Ok(None) => Err(anyhow!("checkpoint not found: {checkpoint_id_prefix}")),
                Err(err) if err.to_string().contains("ambiguous checkpoint prefix") => Err(err),
                Err(_) => Err(anyhow!("checkpoint not found: {checkpoint_id_prefix}")),
            };
        }
        [one] => one.checkpoint_id.clone(),
        many => {
            let examples = many
                .iter()
                .take(5)
                .map(|c| c.checkpoint_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "ambiguous checkpoint prefix {:?} matches {} checkpoints: {}",
                checkpoint_id_prefix,
                many.len(),
                examples
            )
        }
    };

    let summary_view = read_committed(repo_root, &full_checkpoint_id)?
        .ok_or_else(|| anyhow!("checkpoint not found: {full_checkpoint_id}"))?;
    let content_view = read_latest_session_content(repo_root, &full_checkpoint_id)?;

    // Build explain.rs types from the store types.
    let meta = metadata_from_json(&content_view.metadata, &full_checkpoint_id);
    let (has_token_usage, token_input, token_output) =
        if let Some(ref tu) = summary_view.token_usage {
            (
                token_usage_metadata_has_values(tu),
                tu.input_tokens,
                tu.output_tokens,
            )
        } else {
            (meta.has_token_usage, meta.token_input, meta.token_output)
        };
    let summary = CheckpointSummary {
        checkpoint_id: full_checkpoint_id.clone(),
        checkpoints_count: summary_view.checkpoints_count as usize,
        files_touched: summary_view.files_touched.clone(),
        has_token_usage,
        token_input,
        token_output,
    };
    let mut content = SessionContent {
        metadata: meta,
        prompts: content_view.prompts.clone(),
        transcript: content_view.transcript.as_bytes().to_vec(),
    };

    if opts.generate {
        generate_checkpoint_summary(repo_root, &full_checkpoint_id, &content, opts.force)?;
        // Reload content after generation (best-effort).
        if let Ok(refreshed) = read_latest_session_content(repo_root, &full_checkpoint_id) {
            content.metadata = metadata_from_json(&refreshed.metadata, &full_checkpoint_id);
            content.prompts = refreshed.prompts;
            content.transcript = refreshed.transcript.into_bytes();
        }
    }

    if opts.raw_transcript {
        if content.transcript.is_empty() {
            bail!("checkpoint {full_checkpoint_id} has no transcript");
        }
        use std::io::Write;
        std::io::stdout().write_all(&content.transcript)?;
        return Ok(String::new());
    }

    let store_author = get_checkpoint_author(repo_root, &full_checkpoint_id).unwrap_or_default();
    let author = Author {
        name: store_author.name.clone(),
        email: store_author.email.clone(),
    };

    // Unlimited walk when searchAll=true, capped otherwise.
    let graph_limit = if opts.search_all {
        0
    } else {
        COMMIT_SCAN_LIMIT
    };
    let commits = build_commit_graph_from_git(repo_root, graph_limit).unwrap_or_default();
    let associated = get_associated_commits(&commits, &full_checkpoint_id, opts.search_all)?;

    let output = format_checkpoint_output(
        &summary,
        &content,
        &full_checkpoint_id,
        Some(&associated),
        &author,
        opts.verbose,
        opts.full,
    );
    Ok(output)
}

/// Format a Unix timestamp as "YYYY-MM-DD HH:MM:SS" UTC.
fn format_unix_datetime_utc(unix: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    let s = unix % 60;
    let m = (unix / 60) % 60;
    let h = (unix / 3600) % 24;
    let days = unix / 86400;
    // Euclidean affine Gregorian calendar algorithm (UTC).
    // Reference: https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// Read transcript bytes from a git commit tree, trying primary filename then legacy.
fn read_transcript_from_tree(
    repo_root: &std::path::Path,
    commit_sha: &str,
    metadata_dir: &str,
) -> Vec<u8> {
    let primary = format!(
        "{}:{}/{}",
        commit_sha,
        metadata_dir,
        paths::TRANSCRIPT_FILE_NAME
    );
    if let Ok(out) = run_git(repo_root, &["show", &primary])
        && !out.is_empty()
    {
        return out.into_bytes();
    }
    let legacy = format!(
        "{}:{}/{}",
        commit_sha,
        metadata_dir,
        paths::TRANSCRIPT_FILE_NAME_LEGACY
    );
    run_git(repo_root, &["show", &legacy])
        .unwrap_or_default()
        .into_bytes()
}

/// Try to explain a temporary (shadow-branch) checkpoint by SHA prefix.
/// Searches ALL shadow branches.
/// Returns Ok(Some(output)) if found, Ok(None) if not found, Err for ambiguous.
fn explain_temporary_checkpoint_real(
    repo_root: &std::path::Path,
    sha_prefix: &str,
    opts: &ExplainExecutionOptions,
) -> Result<Option<String>> {
    if sha_prefix.is_empty() {
        return Ok(None);
    }

    // List ALL bitloops/* shadow branches — no worktree or reachability filter.
    let branches_out = run_git(repo_root, &["branch", "--list", "bitloops/*"]).unwrap_or_default();
    let shadow_branches: Vec<String> = branches_out
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim().to_string())
        .filter(|b| !b.is_empty() && b != paths::METADATA_BRANCH_NAME)
        .collect();

    struct TempMatch {
        commit_sha: String,
        timestamp: i64,
        session_id: String,
    }

    let mut matches: Vec<TempMatch> = Vec::new();

    for branch in &shadow_branches {
        let log_out =
            run_git(repo_root, &["log", "--format=%H|%ct", branch.as_str()]).unwrap_or_default();

        for line in log_out.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, '|');
            let sha = match parts.next() {
                Some(s) => s.trim(),
                None => continue,
            };
            let timestamp: i64 = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);

            if sha.starts_with(sha_prefix) {
                let commit_msg =
                    run_git(repo_root, &["show", "-s", "--format=%B", sha]).unwrap_or_default();
                let (session_id, _) = parse_session(&commit_msg);
                matches.push(TempMatch {
                    commit_sha: sha.to_string(),
                    timestamp,
                    session_id,
                });
            }
        }
    }

    if matches.is_empty() {
        return Ok(None);
    }

    if matches.len() > 1 {
        let mut msg = format!(
            "ambiguous checkpoint prefix {:?} matches {} temporary checkpoints:\n",
            sha_prefix,
            matches.len()
        );
        for m in matches.iter().take(5) {
            let short_id = &m.commit_sha[..m.commit_sha.len().min(7)];
            let dt = format_unix_datetime_utc(m.timestamp);
            let _ = writeln!(msg, "  {}  {}  session {}", short_id, dt, m.session_id);
        }
        bail!("{}", msg.trim_end());
    }

    let m = &matches[0];
    let commit_sha = &m.commit_sha;

    // Read commit message to extract metadata_dir and session_id.
    let commit_msg = run_git(
        repo_root,
        &["show", "-s", "--format=%B", commit_sha.as_str()],
    )?;
    let (metadata_dir, md_found) = parse_metadata(&commit_msg);
    if !md_found || metadata_dir.is_empty() {
        return Ok(None);
    }
    let (session_id, _) = parse_session(&commit_msg);

    // Read metadata.json to determine agent type.
    let metadata_path = format!(
        "{}:{}/{}",
        commit_sha,
        metadata_dir,
        paths::METADATA_FILE_NAME
    );
    let metadata_json = run_git(repo_root, &["show", &metadata_path]).unwrap_or_default();
    let agent_type = if metadata_json.is_empty() {
        AgentType::ClaudeCode
    } else if let Ok(val) = serde_json::from_str::<Value>(&metadata_json) {
        val.get("agent")
            .and_then(Value::as_str)
            .map(agent_type_from_str)
            .unwrap_or_default()
    } else {
        AgentType::ClaudeCode
    };

    // Handle raw transcript output.
    if opts.raw_transcript {
        let transcript_bytes = read_transcript_from_tree(repo_root, commit_sha, &metadata_dir);
        if transcript_bytes.is_empty() {
            bail!(
                "checkpoint {} has no transcript",
                &commit_sha[..commit_sha.len().min(7)]
            );
        }
        use std::io::Write;
        std::io::stdout().write_all(&transcript_bytes)?;
        return Ok(Some(String::new()));
    }

    // Read prompt from shadow commit tree.
    let prompt_path = format!(
        "{}:{}/{}",
        commit_sha,
        metadata_dir,
        paths::PROMPT_FILE_NAME
    );
    let session_prompt = run_git(repo_root, &["show", &prompt_path]).unwrap_or_default();

    // Build output matching formatCheckpointOutput style but for temporary checkpoints.
    let short_id = &commit_sha[..commit_sha.len().min(7)];
    let dt = format_unix_datetime_utc(m.timestamp);
    let mut out = String::new();
    let _ = writeln!(out, "Checkpoint: {} [temporary]", short_id);
    let _ = writeln!(out, "Session: {}", session_id);
    let _ = writeln!(out, "Created: {}", dt);
    out.push('\n');

    let intent = session_prompt
        .lines()
        .find(|l| !l.is_empty())
        .map(|l| truncate_description(l, MAX_INTENT_DISPLAY_LENGTH))
        .unwrap_or_else(|| "(not available)".to_string());
    let _ = writeln!(out, "Intent: {}", intent);
    out.push_str("Outcome: (not generated)\n");

    if opts.full || opts.verbose {
        let full_transcript = read_transcript_from_tree(repo_root, commit_sha, &metadata_dir);
        let scoped_transcript = if opts.verbose && !full_transcript.is_empty() {
            // Get parent commit to compute checkpoint scope.
            let parent_out =
                run_git(repo_root, &["rev-parse", &format!("{}^", commit_sha)]).unwrap_or_default();
            let parent_sha = parent_out.trim();
            if !parent_sha.is_empty() {
                let parent_transcript =
                    read_transcript_from_tree(repo_root, parent_sha, &metadata_dir);
                if !parent_transcript.is_empty() {
                    let offset = transcript_offset(&parent_transcript, agent_type);
                    scope_transcript_for_checkpoint(&full_transcript, offset, agent_type)
                } else {
                    full_transcript.clone()
                }
            } else {
                full_transcript.clone()
            }
        } else {
            Vec::new()
        };
        append_transcript_section(
            &mut out,
            opts.verbose,
            opts.full,
            &full_transcript,
            &scoped_transcript,
            &session_prompt,
            agent_type,
        );
    }

    Ok(Some(out))
}

/// Generates an AI summary for a checkpoint and persists it via `update_summary`.
pub fn generate_checkpoint_summary(
    repo_root: &std::path::Path,
    checkpoint_id: &str,
    content: &SessionContent,
    force: bool,
) -> Result<()> {
    if checkpoint_id.is_empty() {
        bail!("checkpoint id is required")
    }
    if content.transcript.is_empty() {
        bail!("checkpoint {checkpoint_id} has no transcript to summarize")
    }
    if content.metadata.summary.is_some() && !force {
        bail!("checkpoint {checkpoint_id} already has a summary (use --force to regenerate)")
    }

    // Convert explain::AgentType to summarize::AgentType.
    let summarize_agent = match content.metadata.agent_type {
        AgentType::Cursor => crate::engine::summarize::AgentType::Cursor,
        AgentType::Gemini => crate::engine::summarize::AgentType::Gemini,
        AgentType::OpenCode => crate::engine::summarize::AgentType::OpenCode,
        AgentType::ClaudeCode => crate::engine::summarize::AgentType::ClaudeCode,
    };

    // Scope the transcript to only this checkpoint's portion.
    let scoped = crate::engine::summarize::scope_transcript_for_checkpoint(
        &content.transcript,
        content.metadata.checkpoint_transcript_start,
        summarize_agent,
    );
    if scoped.is_empty() {
        bail!("checkpoint {checkpoint_id} has no transcript content for this checkpoint (scoped)")
    }

    let summary = crate::engine::summarize::generate_from_transcript(
        &scoped,
        &content.metadata.files_touched,
        summarize_agent,
        None,
    )?;

    let summary_json = serde_json::to_value(&summary)
        .map_err(|e| anyhow!("serializing generated summary: {e}"))?;

    crate::engine::strategy::manual_commit::update_summary(repo_root, checkpoint_id, summary_json)
}

// CLI-855 / CLI-858: formatting + transcript stubs
pub fn format_session_info(
    session: &SessionInfo,
    source_ref: &str,
    checkpoints: &[CheckpointDetail],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Session: {}", session.id);
    let _ = writeln!(out, "Strategy: {}", session.strategy);
    if !session.start_time.is_empty() {
        let _ = writeln!(out, "Started: {}", session.start_time);
    }
    if !source_ref.is_empty() {
        let _ = writeln!(out, "Source Ref: {}", source_ref);
    }
    let _ = writeln!(out, "Checkpoints: {}", checkpoints.len());

    for cp in checkpoints {
        out.push('\n');
        let task_marker = if cp.is_task_checkpoint { " [Task]" } else { "" };
        let _ = writeln!(
            out,
            "─── Checkpoint {} [{}] {}{} ───",
            cp.index, cp.short_id, cp.timestamp, task_marker
        );
        out.push('\n');

        if cp.interactions.is_empty() {
            if !cp.message.is_empty() {
                let _ = writeln!(out, "{}", cp.message);
                out.push('\n');
            }
            if !cp.files.is_empty() {
                let _ = writeln!(out, "Files Modified ({}):", cp.files.len());
                for file in &cp.files {
                    let _ = writeln!(out, "  - {}", file);
                }
            }
            continue;
        }

        for (idx, inter) in cp.interactions.iter().enumerate() {
            if cp.interactions.len() > 1 {
                let _ = writeln!(out, "### Interaction {}", idx + 1);
                out.push('\n');
            }
            if !inter.prompt.is_empty() {
                out.push_str("## Prompt\n\n");
                let _ = writeln!(out, "{}", inter.prompt);
                out.push('\n');
            }
            if !inter.responses.is_empty() {
                out.push_str("## Responses\n\n");
                let _ = writeln!(out, "{}", inter.responses.join("\n\n"));
                out.push('\n');
            }
            if !inter.files.is_empty() {
                let _ = writeln!(out, "Files Modified ({}):", inter.files.len());
                for file in &inter.files {
                    let _ = writeln!(out, "  - {}", file);
                }
                out.push('\n');
            }
        }
    }

    out
}

