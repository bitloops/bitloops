use super::*;

pub fn append_transcript_section(
    output: &mut String,
    verbose: bool,
    full: bool,
    full_transcript: &[u8],
    scoped_transcript: &[u8],
    scoped_fallback: &str,
    agent_type: AgentType,
) {
    if full {
        output.push('\n');
        output.push_str("Transcript (full session):\n");
        output.push_str(&format_transcript_bytes(full_transcript, "", agent_type));
    } else if verbose {
        output.push('\n');
        output.push_str("Transcript (checkpoint scope):\n");
        output.push_str(&format_transcript_bytes(
            scoped_transcript,
            scoped_fallback,
            agent_type,
        ));
    }
}

// CLI-856 / CLI-857 / CLI-850: git traversal + branch discovery stubs
#[cfg(test)]
pub fn get_associated_commits(
    commits: &[CommitNode],
    checkpoint_id: &str,
    search_all: bool,
) -> Result<Vec<AssociatedCommit>> {
    if checkpoint_id.is_empty() {
        return Ok(Vec::new());
    }

    let mut collected: Vec<(i64, AssociatedCommit)> = Vec::new();
    let commit_map: HashMap<&str, &CommitNode> =
        commits.iter().map(|c| (c.sha.as_str(), c)).collect();

    if search_all || commits.iter().all(|commit| commit.parents.is_empty()) {
        for commit in commits {
            if commit_checkpoint_matches(commit, checkpoint_id) {
                collected.push((commit.timestamp, to_associated_commit(commit)));
            }
        }
    } else if let Some(head) = commits.iter().max_by_key(|commit| commit.timestamp) {
        let mut current = head;
        let mut visited: HashSet<&str> = HashSet::new();

        loop {
            if !visited.insert(current.sha.as_str()) {
                break;
            }
            if commit_checkpoint_matches(current, checkpoint_id) {
                collected.push((current.timestamp, to_associated_commit(current)));
            }

            let Some(parent) = current.parents.first() else {
                break;
            };
            let Some(parent_commit) = commit_map.get(parent.as_str()) else {
                break;
            };
            current = parent_commit;
        }
    }

    collected.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(collected.into_iter().map(|(_, commit)| commit).collect())
}

pub fn get_associated_commits_from_db(
    repo_root: &std::path::Path,
    commits: &[CommitNode],
    checkpoint_id: &str,
    search_all: bool,
) -> Result<Vec<AssociatedCommit>> {
    if checkpoint_id.is_empty() {
        return Ok(Vec::new());
    }

    let checkpoint_map = read_commit_checkpoint_mappings(repo_root)?;
    let mut collected: Vec<(i64, AssociatedCommit)> = Vec::new();
    let commit_map: HashMap<&str, &CommitNode> =
        commits.iter().map(|c| (c.sha.as_str(), c)).collect();

    let commit_matches = |commit: &CommitNode| {
        checkpoint_map
            .get(commit.sha.as_str())
            .is_some_and(|mapped| mapped == checkpoint_id)
    };

    if search_all || commits.iter().all(|commit| commit.parents.is_empty()) {
        for commit in commits {
            if commit_matches(commit) {
                collected.push((commit.timestamp, to_associated_commit(commit)));
            }
        }
    } else if let Some(head) = commits.iter().max_by_key(|commit| commit.timestamp) {
        let mut current = head;
        let mut visited: HashSet<&str> = HashSet::new();

        loop {
            if !visited.insert(current.sha.as_str()) {
                break;
            }
            if commit_matches(current) {
                collected.push((current.timestamp, to_associated_commit(current)));
            }

            let Some(parent) = current.parents.first() else {
                break;
            };
            let Some(parent_commit) = commit_map.get(parent.as_str()) else {
                break;
            };
            current = parent_commit;
        }
    }

    collected.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(collected.into_iter().map(|(_, commit)| commit).collect())
}

fn compute_reachable_from_main(
    repo_root: &std::path::Path,
    is_on_default_branch: bool,
) -> HashSet<String> {
    if is_on_default_branch {
        return HashSet::new();
    }

    let out = run_git(repo_root, &["log", "--first-parent", "--format=%H", "main"])
        .or_else(|_| {
            run_git(
                repo_root,
                &["log", "--first-parent", "--format=%H", "master"],
            )
        })
        .unwrap_or_default();

    out.lines()
        .take(1000)
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn walk_first_parent_commits(
    head_sha: &str,
    commit_map: &HashMap<String, CommitNode>,
    limit: usize,
) -> Result<Vec<CommitNode>> {
    let Some(mut current) = commit_map.get(head_sha).cloned() else {
        bail!("failed to get commit {head_sha}")
    };

    let mut output = Vec::new();
    let mut visited = HashSet::new();
    let mut count = 0usize;

    loop {
        if limit > 0 && count >= limit {
            break;
        }
        if !visited.insert(current.sha.clone()) {
            break;
        }
        output.push(current.clone());
        count += 1;

        let Some(parent_sha) = current.parents.first() else {
            break;
        };
        let Some(parent) = commit_map.get(parent_sha) else {
            break;
        };
        current = parent.clone();
    }

    Ok(output)
}

pub fn has_code_changes(files_changed: &[String], is_first_commit: bool) -> bool {
    if is_first_commit {
        return true;
    }
    files_changed
        .iter()
        .any(|path| !paths::is_infrastructure_path(path))
}

pub fn get_current_worktree_hash(worktree_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(worktree_id.as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)[..6].to_string()
}

/// Real implementation: derives the worktree hash from the repo's actual worktree ID.
pub fn get_current_worktree_hash_real(repo_root: &std::path::Path) -> String {
    let wt_id = paths::get_worktree_id(repo_root).unwrap_or_default();
    get_current_worktree_hash(&wt_id)
}

pub fn is_ancestor_of(
    commit_map: &HashMap<String, CommitNode>,
    ancestor_sha: &str,
    descendant_sha: &str,
) -> bool {
    if ancestor_sha.is_empty() || descendant_sha.is_empty() {
        return false;
    }
    if ancestor_sha == descendant_sha {
        return true;
    }

    let mut stack = VecDeque::from([descendant_sha.to_string()]);
    let mut visited = HashSet::new();

    while let Some(current) = stack.pop_front() {
        if !visited.insert(current.clone()) {
            continue;
        }
        let Some(node) = commit_map.get(&current) else {
            continue;
        };
        for parent in &node.parents {
            if parent == ancestor_sha {
                return true;
            }
            stack.push_back(parent.clone());
        }
    }

    false
}

fn resolve_branch_display_name(repo_root: &std::path::Path) -> String {
    if run_git(repo_root, &["rev-parse", "--verify", "HEAD"]).is_err() {
        return "HEAD (no commits yet)".to_string();
    }

    if let Ok(branch) = run_git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        let branch = branch.trim();
        if !branch.is_empty() {
            return branch.to_string();
        }
    }

    // Detached HEAD: show short sha.
    let sha = run_git(repo_root, &["rev-parse", "HEAD"]).unwrap_or_default();
    let short = &sha[..sha.len().min(7)];
    format!("HEAD ({short})")
}

// CLI-859: integration flow + pager/output
pub(crate) fn run_explain_branch_with_filter_in(
    repo_root: &std::path::Path,
    session_filter: &str,
    no_pager: bool,
) -> Result<String> {
    let branch = resolve_branch_display_name(repo_root);
    let points = get_branch_checkpoints_real(repo_root, 100).unwrap_or_default();
    if !session_filter.is_empty() {
        let strategy = new_manual_commit_strategy();
        let source: &dyn SessionSource = &strategy;
        let additional_sessions = source.get_additional_sessions().unwrap_or_default();
        if let Some((session, details)) =
            build_runtime_session_info(&points, session_filter, &additional_sessions)
        {
            let source_ref = format!("branch:{branch}");
            let content = format_session_info(&session, &source_ref, &details);
            return Ok(output_explain_content(&content, no_pager));
        }
    }

    let content = format_branch_checkpoints(&branch, &points, session_filter);
    Ok(output_explain_content(&content, no_pager))
}

pub fn run_explain_branch_with_filter(session_filter: &str, no_pager: bool) -> Result<String> {
    let repo_root = paths::repo_root()?;
    run_explain_branch_with_filter_in(&repo_root, session_filter, no_pager)
}

pub fn run_explain_branch_default(no_pager: bool) -> Result<String> {
    run_explain_branch_with_filter("", no_pager)
}

fn short_display_id(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    if input.chars().count() <= 7 {
        return input.to_string();
    }
    input.chars().take(7).collect()
}

fn build_runtime_session_info(
    points: &[RewindPoint],
    session_filter: &str,
    additional_sessions: &[SessionInfo],
) -> Option<(SessionInfo, Vec<CheckpointDetail>)> {
    if session_filter.is_empty() {
        return None;
    }

    let mut filtered: Vec<RewindPoint> = points
        .iter()
        .filter(|point| {
            point.session_id == session_filter || point.session_id.starts_with(session_filter)
        })
        .cloned()
        .collect();

    filtered.sort_by(|a, b| b.date.cmp(&a.date));

    let session_match = additional_sessions
        .iter()
        .find(|session| session.id == session_filter || session.id.starts_with(session_filter))
        .cloned();

    if filtered.is_empty() {
        return session_match.map(|session| (session, Vec::new()));
    }

    let session_id = filtered
        .iter()
        .find_map(|point| {
            if point.session_id.is_empty() {
                None
            } else {
                Some(point.session_id.clone())
            }
        })
        .unwrap_or_else(|| session_filter.to_string());

    let mut session = session_match.unwrap_or_else(|| SessionInfo {
        id: session_id.clone(),
        strategy: "manual-commit".to_string(),
        ..SessionInfo::default()
    });
    if session.id.is_empty() {
        session.id = session_id.clone();
    }
    if session.strategy.is_empty() {
        session.strategy = "manual-commit".to_string();
    }
    if session.start_time.is_empty() {
        session.start_time = filtered
            .last()
            .map(|point| point.date.clone())
            .unwrap_or_default();
    }

    session.checkpoints = filtered
        .iter()
        .map(|point| SessionCheckpoint {
            checkpoint_id: if point.checkpoint_id.is_empty() {
                point.id.clone()
            } else {
                point.checkpoint_id.clone()
            },
            message: point.message.clone(),
            timestamp: point.date.clone(),
        })
        .collect();

    let total = filtered.len();
    let details: Vec<CheckpointDetail> = filtered
        .iter()
        .enumerate()
        .map(|(idx, point)| {
            let display_id = if point.checkpoint_id.is_empty() {
                &point.id
            } else {
                &point.checkpoint_id
            };

            let interactions = if point.session_prompt.is_empty() {
                Vec::new()
            } else {
                vec![Interaction {
                    prompt: point.session_prompt.clone(),
                    ..Interaction::default()
                }]
            };

            CheckpointDetail {
                index: total.saturating_sub(idx),
                short_id: short_display_id(display_id),
                timestamp: point.date.clone(),
                is_task_checkpoint: point.is_task_checkpoint,
                message: point.message.clone(),
                interactions,
                files: Vec::new(),
            }
        })
        .collect();

    Some((session, details))
}

/// Real implementation: reads committed checkpoints from git commit graph + DB checkpoint mappings.
pub fn get_branch_checkpoints_real(
    repo_root: &std::path::Path,
    limit: usize,
) -> Result<Vec<RewindPoint>> {
    let branch = run_git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let is_default = is_default_branch(&branch);

    let all_committed = list_committed(repo_root)?;
    let committed_map: HashMap<_, _> = all_committed
        .iter()
        .map(|c| (c.checkpoint_id.clone(), c))
        .collect();
    let commit_checkpoint_map = read_commit_checkpoint_mappings(repo_root)?;

    // Unlimited walk on default branch, capped on feature branches.
    let graph_limit = if is_default { 0 } else { COMMIT_SCAN_LIMIT };
    let commits = build_commit_graph_from_git(repo_root, graph_limit)?;
    let reachable_from_main = compute_reachable_from_main(repo_root, is_default);

    let walk_commits: Vec<CommitNode> = if is_default {
        commits.clone()
    } else if commits.is_empty() {
        Vec::new()
    } else {
        let commit_map: HashMap<String, CommitNode> = commits
            .iter()
            .map(|commit| (commit.sha.clone(), commit.clone()))
            .collect();
        let head_sha = commits[0].sha.clone();
        let mut walked: Vec<CommitNode> = Vec::new();
        for commit in walk_first_parent_commits(&head_sha, &commit_map, COMMIT_SCAN_LIMIT)? {
            if reachable_from_main.contains(&commit.sha) {
                break;
            }
            walked.push(commit);
        }
        walked
    };

    let mut points: Vec<RewindPoint> = Vec::new();
    for commit in &walk_commits {
        let Some(cp_id) = commit_checkpoint_map.get(&commit.sha) else {
            continue;
        };
        if !committed_map.contains_key(cp_id.as_str()) {
            continue;
        }
        let sv = committed_map[cp_id.as_str()];

        // Read first prompt from session content (best-effort).
        let session_prompt_content = if !sv.session_id.is_empty() {
            read_session_content_by_id(repo_root, cp_id, &sv.session_id)
        } else {
            read_latest_session_content(repo_root, cp_id)
        };
        let session_prompt = session_prompt_content
            .ok()
            .and_then(|cv| {
                cv.prompts
                    .lines()
                    .find(|l| !l.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_default();

        let date = chrono_format(commit.timestamp);
        points.push(RewindPoint {
            id: commit.sha.clone(),
            checkpoint_id: cp_id.clone(),
            session_id: sv.session_id.clone(),
            message: commit.message.clone(),
            date,
            session_prompt,
            is_logs_only: true,
            is_task_checkpoint: sv.is_task,
            tool_use_id: sv.tool_use_id.clone(),
        });
    }

    points.sort_by(|a, b| b.date.cmp(&a.date));
    if limit > 0 && points.len() > limit {
        points.truncate(limit);
    }

    Ok(points)
}

#[allow(dead_code)]
/// Try to gather temporary (shadow-branch) checkpoints from real git branches.
fn get_reachable_temporary_checkpoints_shell(
    repo_root: &std::path::Path,
    commits: &[CommitNode],
    is_default: bool,
) -> Vec<RewindPoint> {
    let current_wt_hash = get_current_worktree_hash_real(repo_root);

    // List bitloops/* shadow branches.
    let branches_out = run_git(repo_root, &["branch", "--list", "bitloops/*"]).unwrap_or_default();

    let shadow_branches: Vec<String> = branches_out
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim().to_string())
        .filter(|b| !b.is_empty() && b != paths::METADATA_BRANCH_NAME)
        .collect();

    let commit_map: HashMap<&str, &CommitNode> =
        commits.iter().map(|c| (c.sha.as_str(), c)).collect();

    let head_sha = commits.first().map(|c| c.sha.as_str()).unwrap_or("");

    let mut points = Vec::new();

    for branch in &shadow_branches {
        // Parse: bitloops/<base7>-<wt6>
        let stem = branch.strip_prefix("bitloops/").unwrap_or(branch.as_str());
        // stem should be like: abc1234-wt1234
        let parts: Vec<&str> = stem.rsplitn(2, '-').collect();
        if parts.len() != 2 {
            continue;
        }
        let wt_hash = parts[0];
        let base_commit_short = parts[1];

        // Filter by worktree if we know our hash.
        if !current_wt_hash.is_empty() && wt_hash != current_wt_hash {
            continue;
        }

        // Check reachability of the base commit from HEAD.
        if !head_sha.is_empty() && !is_default {
            let base_full = commit_map
                .keys()
                .find(|sha| sha.starts_with(base_commit_short))
                .copied()
                .unwrap_or(base_commit_short);
            if !is_ancestor_of(
                &commit_map
                    .iter()
                    .map(|(k, v)| (k.to_string(), (*v).clone()))
                    .collect(),
                base_full,
                head_sha,
            ) {
                continue;
            }
        }

        // List commits on this shadow branch.
        let log_out = run_git(
            repo_root,
            &[
                "log",
                "--format=%H|%an|%ct|%s",
                "--name-only",
                branch.as_str(),
            ],
        )
        .unwrap_or_default();

        // Parse the log: blocks separated by empty lines.
        // Format: header line, then file names, then empty line.
        let mut block_lines = log_out.lines().peekable();
        let mut is_first = true;
        while let Some(header) = block_lines.next() {
            let header = header.trim();
            if header.is_empty() {
                continue;
            }
            let hparts: Vec<&str> = header.splitn(4, '|').collect();
            if hparts.len() < 4 {
                continue;
            }
            let commit_sha = hparts[0].trim().to_string();
            let author = hparts[1].trim().to_string();
            let timestamp: i64 = hparts[2].trim().parse().unwrap_or(0);
            let message = hparts[3].trim().to_string();

            // `git log --name-only` emits a blank line after each commit header.
            // Skip those separators before collecting file names.
            while let Some(&next_line) = block_lines.peek() {
                if next_line.trim().is_empty() {
                    let _ = block_lines.next();
                    continue;
                }
                break;
            }

            // Collect file names until blank line.
            let mut files: Vec<String> = Vec::new();
            while let Some(&next_line) = block_lines.peek() {
                if next_line.trim().is_empty() {
                    let _ = block_lines.next();
                    break;
                }
                // Next commit header: leave it for the outer loop.
                let next_parts: Vec<&str> = next_line.splitn(4, '|').collect();
                if next_parts.len() == 4
                    && next_parts[0].len() == 40
                    && next_parts[0].chars().all(|c| c.is_ascii_hexdigit())
                {
                    break;
                }
                files.push(block_lines.next().unwrap_or("").trim().to_string());
            }

            if !has_code_changes(&files, is_first) {
                is_first = false;
                continue;
            }
            is_first = false;

            let date = chrono_format(timestamp);
            let _ = author; // suppress unused warning
            points.push(RewindPoint {
                id: commit_sha,
                checkpoint_id: String::new(),
                session_id: String::new(),
                message,
                date,
                session_prompt: String::new(),
                is_logs_only: false,
                is_task_checkpoint: false,
                tool_use_id: String::new(),
            });
        }
    }

    points
}
