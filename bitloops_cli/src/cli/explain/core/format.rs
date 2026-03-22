use super::super::*;

pub fn format_checkpoint_output(
    summary: &CheckpointSummary,
    content: &SessionContent,
    checkpoint_id: &str,
    associated_commits: Option<&[AssociatedCommit]>,
    author: &Author,
    verbose: bool,
    full: bool,
) -> String {
    let mut out = String::new();
    let meta = &content.metadata;
    let agent_type = meta.agent_type;
    let scoped_transcript = scope_transcript_for_checkpoint(
        &content.transcript,
        meta.checkpoint_transcript_start,
        agent_type,
    );
    let scoped_prompts = extract_prompts_from_transcript(&scoped_transcript, agent_type);

    let _ = writeln!(out, "Checkpoint: {}", checkpoint_id);
    let _ = writeln!(out, "Session: {}", meta.session_id);
    let _ = writeln!(out, "Created: {}", meta.created_at);

    if !author.name.is_empty() {
        let _ = writeln!(out, "Author: {} <{}>", author.name, author.email);
    }

    let token_input = if meta.token_input == 0 {
        summary.token_input
    } else {
        meta.token_input
    };
    let token_output = if meta.token_output == 0 {
        summary.token_output
    } else {
        meta.token_output
    };
    let total_tokens = token_input + token_output;
    let should_show_tokens = summary.has_token_usage || meta.has_token_usage;
    if should_show_tokens {
        let _ = writeln!(out, "Tokens: {}", total_tokens);
    }

    match associated_commits {
        Some(commits) if !commits.is_empty() => {
            out.push('\n');
            let _ = writeln!(out, "Commits: ({})", commits.len());
            for commit in commits {
                let _ = writeln!(
                    out,
                    "  {} {} {}",
                    commit.short_sha, commit.date, commit.message
                );
            }
        }
        Some(_) => out.push_str("\nCommits: No commits found on this branch\n"),
        None => {}
    }

    out.push('\n');

    if let Some(summary_details) = &meta.summary {
        let _ = writeln!(out, "Intent: {}", summary_details.intent);
        let _ = writeln!(out, "Outcome: {}", summary_details.outcome);
    } else {
        let intent = scoped_prompts
            .first()
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                content
                    .prompts
                    .lines()
                    .find(|line| !line.is_empty())
                    .map(ToString::to_string)
            })
            .map(|text| truncate_description(&text, MAX_INTENT_DISPLAY_LENGTH))
            .unwrap_or_else(|| "(not generated)".to_string());

        let _ = writeln!(out, "Intent: {}", intent);
        out.push_str("Outcome: (not generated)\n");
    }

    if verbose || full {
        if let Some(summary_details) = &meta.summary {
            out.push_str(&format_summary_details(summary_details));
        }

        out.push('\n');
        if !meta.files_touched.is_empty() {
            let _ = writeln!(out, "Files: ({})", meta.files_touched.len());
            for file in &meta.files_touched {
                let _ = writeln!(out, "  - {}", file);
            }
        } else {
            out.push_str("Files: (none)\n");
        }
    }

    append_transcript_section(
        &mut out,
        verbose,
        full,
        &content.transcript,
        &scoped_transcript,
        &content.prompts,
        agent_type,
    );

    out
}

pub fn format_summary_details(summary: &SummaryDetails) -> String {
    let mut out = String::new();
    let has_learnings = !summary.repo_learnings.is_empty()
        || !summary.code_learnings.is_empty()
        || !summary.workflow_learnings.is_empty();

    if has_learnings {
        out.push_str("\nLearnings:\n");

        if !summary.repo_learnings.is_empty() {
            out.push_str("  Repository:\n");
            for item in &summary.repo_learnings {
                let _ = writeln!(out, "    - {}", item);
            }
        }

        if !summary.code_learnings.is_empty() {
            out.push_str("  Code:\n");
            for item in &summary.code_learnings {
                if item.line > 0 && item.end_line > 0 && item.end_line != item.line {
                    let _ = writeln!(
                        out,
                        "    - {}:{}-{}: {}",
                        item.path, item.line, item.end_line, item.finding
                    );
                } else if item.line > 0 {
                    let _ = writeln!(out, "    - {}:{}: {}", item.path, item.line, item.finding);
                } else {
                    let _ = writeln!(out, "    - {}: {}", item.path, item.finding);
                }
            }
        }

        if !summary.workflow_learnings.is_empty() {
            out.push_str("  Workflow:\n");
            for item in &summary.workflow_learnings {
                let _ = writeln!(out, "    - {}", item);
            }
        }
    }

    if !summary.friction.is_empty() {
        out.push_str("\nFriction:\n");
        for item in &summary.friction {
            let _ = writeln!(out, "  - {}", item);
        }
    }

    if !summary.open_items.is_empty() {
        out.push_str("\nOpen Items:\n");
        for item in &summary.open_items {
            let _ = writeln!(out, "  - {}", item);
        }
    }

    out
}

pub fn format_branch_checkpoints(
    branch_name: &str,
    points: &[RewindPoint],
    session_filter: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Branch: {}", branch_name);

    let filtered: Vec<RewindPoint> = if session_filter.is_empty() {
        points.to_vec()
    } else {
        points
            .iter()
            .filter(|p| p.session_id == session_filter || p.session_id.starts_with(session_filter))
            .cloned()
            .collect()
    };

    if filtered.is_empty() {
        out.push_str("Checkpoints: 0\n");
        if !session_filter.is_empty() {
            let _ = writeln!(out, "Filtered by session: {}", session_filter);
        }
        out.push_str("\nNo checkpoints found on this branch.\n");
        out.push_str(
            "Checkpoints will appear here after you save changes during an agent session.\n",
        );
        return out;
    }

    let groups = group_by_checkpoint_id(&filtered);
    let _ = writeln!(out, "Checkpoints: {}", groups.len());
    if !session_filter.is_empty() {
        let _ = writeln!(out, "Filtered by session: {}", session_filter);
    }
    out.push('\n');

    for group in &groups {
        out.push_str(&format_checkpoint_group(group));
        out.push('\n');
    }

    out
}

pub fn group_by_checkpoint_id(points: &[RewindPoint]) -> Vec<CheckpointGroup> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut grouped: HashMap<String, Vec<RewindPoint>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for point in points {
        let mut key = point.checkpoint_id.clone();
        if key.is_empty() {
            key = if point.session_id.is_empty() {
                "temporary".to_string()
            } else {
                point.session_id.clone()
            };
        }

        if !grouped.contains_key(&key) {
            order.push(key.clone());
        }
        grouped.entry(key).or_default().push(point.clone());
    }

    for group_points in grouped.values_mut() {
        group_points.sort_by(|a, b| b.date.cmp(&a.date));
    }

    let mut groups: Vec<CheckpointGroup> = order
        .into_iter()
        .map(|key| CheckpointGroup {
            checkpoint_id: key.clone(),
            points: grouped.remove(&key).unwrap_or_default(),
        })
        .collect();

    groups.sort_by(|a, b| {
        let a_date = a
            .points
            .first()
            .map(|p| p.date.as_str())
            .unwrap_or_default();
        let b_date = b
            .points
            .first()
            .map(|p| p.date.as_str())
            .unwrap_or_default();
        b_date.cmp(a_date)
    });

    groups
}

pub fn format_checkpoint_group(group: &CheckpointGroup) -> String {
    let mut out = String::new();

    let mut display_id = group.checkpoint_id.clone();
    if display_id.chars().count() > CHECKPOINT_ID_DISPLAY_LENGTH {
        display_id = display_id
            .chars()
            .take(CHECKPOINT_ID_DISPLAY_LENGTH)
            .collect();
    }

    let is_task = group.points.iter().any(|p| p.is_task_checkpoint);
    let is_temporary = group.points.iter().any(|p| !p.is_logs_only);
    let prompt = group
        .points
        .iter()
        .map(|p| p.session_prompt.as_str())
        .find(|prompt| !prompt.is_empty())
        .unwrap_or("(no prompt)");

    let prompt = if prompt == "(no prompt)" {
        "(no prompt)".to_string()
    } else {
        format!(
            "\"{}\"",
            truncate_description(prompt, MAX_PROMPT_DISPLAY_LENGTH)
        )
    };

    let mut indicators = Vec::new();
    if is_task {
        indicators.push("[Task]");
    }
    if is_temporary && display_id != "temporary" {
        indicators.push("[temporary]");
    }
    let indicator_text = if indicators.is_empty() {
        String::new()
    } else {
        format!(" {}", indicators.join(" "))
    };

    let _ = writeln!(out, "[{}]{} {}", display_id, indicator_text, prompt);

    for point in &group.points {
        let mut short_sha = point.id.clone();
        if short_sha.chars().count() > 7 {
            short_sha = short_sha.chars().take(7).collect();
        }
        let message = truncate_description(&point.message, MAX_MESSAGE_DISPLAY_LENGTH);
        let _ = writeln!(out, "  {} ({}) {}", point.date, short_sha, message);
    }

    out
}

pub fn scope_transcript_for_checkpoint(
    full_transcript: &[u8],
    start_offset: usize,
    agent_type: AgentType,
) -> Vec<u8> {
    match agent_type {
        // Gemini stores transcripts as a single JSON blob; offset is a message index.
        AgentType::Gemini => crate::adapters::agents::gemini::transcript::slice_from_message(
            full_transcript,
            start_offset,
        )
        .unwrap_or_else(|| full_transcript.to_vec()),
        // Claude Code and OpenCode use JSONL; offset is a line number.
        AgentType::Codex | AgentType::ClaudeCode | AgentType::Cursor | AgentType::OpenCode => {
            crate::host::checkpoints::transcript::parse::slice_from_line(
                full_transcript,
                start_offset,
            )
        }
    }
}

pub fn extract_prompts_from_transcript(
    transcript_bytes: &[u8],
    _agent_type: AgentType,
) -> Vec<String> {
    if transcript_bytes.is_empty() {
        return Vec::new();
    }

    transcript_bytes
        .split(|b| *b == b'\n')
        .filter_map(|line| {
            if line.is_empty() {
                return None;
            }
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                return None;
            };
            let entry_type = value
                .get("type")
                .or_else(|| value.get("role"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if entry_type != "user" {
                return None;
            }
            extract_user_prompt(&value)
        })
        .collect()
}

pub fn format_transcript_bytes(
    transcript_bytes: &[u8],
    fallback: &str,
    _agent_type: AgentType,
) -> String {
    if transcript_bytes.is_empty() {
        if fallback.is_empty() {
            return "  (none)\n".to_string();
        }
        return format!("{fallback}\n");
    }

    let mut out = String::new();
    let mut parsed_any = false;

    for line in transcript_bytes.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let entry_type = value
            .get("type")
            .or_else(|| value.get("role"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        match entry_type {
            "user" => {
                if let Some(prompt) = extract_user_prompt(&value)
                    && !prompt.is_empty()
                {
                    parsed_any = true;
                    let _ = writeln!(out, "[User] {}", prompt);
                }
            }
            "assistant" => {
                if let Some(response) = extract_assistant_response(&value)
                    && !response.is_empty()
                {
                    parsed_any = true;
                    let _ = writeln!(out, "[Assistant] {}", response);
                }
            }
            _ => {}
        }
    }

    if parsed_any {
        return out;
    }

    if !fallback.is_empty() {
        return format!("{fallback}\n");
    }

    "  (failed to parse transcript)\n".to_string()
}
