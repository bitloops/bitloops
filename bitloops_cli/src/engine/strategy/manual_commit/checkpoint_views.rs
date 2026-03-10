fn get_git_author_from_repo(repo_root: &Path) -> Result<(String, String)> {
    let local_name = run_git(repo_root, &["config", "--get", "user.name"]).ok();
    let local_email = run_git(repo_root, &["config", "--get", "user.email"]).ok();
    let global_name = run_git(repo_root, &["config", "--global", "--get", "user.name"]).ok();
    let global_email = run_git(repo_root, &["config", "--global", "--get", "user.email"]).ok();

    let name = local_name
        .or(global_name)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Unknown".to_string());
    let email = local_email
        .or(global_email)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "unknown@local".to_string());
    Ok((name, email))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckpointSessionRef {
    #[serde(default)]
    pub metadata: String,
    #[serde(default)]
    pub transcript: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckpointSummaryView {
    #[serde(default)]
    pub checkpoint_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cli_version: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default)]
    pub checkpoints_count: u32,
    #[serde(default)]
    pub files_touched: Vec<String>,
    #[serde(default)]
    pub sessions: Vec<CheckpointSessionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsageMetadata>,
    #[serde(default, skip)]
    pub session_count: usize,
}

/// List-row view of a committed checkpoint (session-derived fields included).
///
/// Returned only by `list_committed()`. For single-checkpoint root metadata,
/// use `read_committed()`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommittedInfo {
    #[serde(default)]
    pub checkpoint_id: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default)]
    pub checkpoints_count: u32,
    #[serde(default)]
    pub files_touched: Vec<String>,
    #[serde(default)]
    pub session_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsageMetadata>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub first_prompt_preview: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_task: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_use_id: String,
}

fn summary_session_count(summary: &CheckpointSummaryView) -> usize {
    summary.sessions.len()
}

fn to_committed_info(
    repo_root: &Path,
    read_ref: &str,
    summary: &CheckpointSummaryView,
) -> CommittedInfo {
    let mut info = CommittedInfo {
        checkpoint_id: summary.checkpoint_id.clone(),
        strategy: summary.strategy.clone(),
        branch: summary.branch.clone(),
        checkpoints_count: summary.checkpoints_count,
        files_touched: summary.files_touched.clone(),
        session_count: summary_session_count(summary),
        token_usage: summary.token_usage.clone(),
        ..Default::default()
    };

    if info.session_count == 0 {
        return info;
    }

    let (a, b) = checkpoint_dir_parts(&summary.checkpoint_id);
    let latest_session_index = info.session_count - 1;

    for idx in 0..info.session_count {
        let meta_path = format!("{a}/{b}/{idx}/{}", paths::METADATA_FILE_NAME);
        let Ok(raw) = git_show_file(repo_root, read_ref, &meta_path) else {
            continue;
        };

        if let Ok(meta) = serde_json::from_str::<CommittedMetadata>(&raw) {
            push_unique_agent(&mut info.agents, &meta.agent);
            if idx == latest_session_index {
                info.session_id = meta.session_id;
                info.agent = canonicalize_agent_type(&meta.agent);
                info.created_at = meta.created_at;
                info.is_task = meta.is_task;
                info.tool_use_id = meta.tool_use_id;
            }
            continue;
        }

        // Keep list/read behavior resilient to legacy metadata with partial fields.
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&raw) {
            push_unique_agent(
                &mut info.agents,
                meta.get("agent")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            );

            if idx == latest_session_index {
                info.session_id = meta
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.agent = canonicalize_agent_type(
                    meta.get("agent")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                );
                info.created_at = meta
                    .get("created_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.is_task = meta
                    .get("is_task")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                info.tool_use_id = meta
                    .get("tool_use_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
        }
    }

    if info.agent.is_empty()
        && let Some(last) = info.agents.last()
    {
        info.agent = last.clone();
    }

    let first_prompt_path = format!("{a}/{b}/0/{}", paths::PROMPT_FILE_NAME);
    if let Ok(raw_prompts) = git_show_file(repo_root, read_ref, &first_prompt_path) {
        info.first_prompt_preview = first_prompt_preview(&raw_prompts);
    }

    info
}

fn push_unique_agent(agents: &mut Vec<String>, agent: &str) {
    let normalized = canonicalize_agent_type(agent);
    if normalized.is_empty() || agents.iter().any(|existing| existing == &normalized) {
        return;
    }
    agents.push(normalized);
}

fn first_prompt_preview(prompts_blob: &str) -> String {
    let first_prompt = prompts_blob
        .split("\n\n---\n\n")
        .next()
        .unwrap_or_default()
        .trim();
    first_prompt.chars().take(160).collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointAuthor {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionContentView {
    pub metadata: serde_json::Value,
    pub transcript: String,
    pub prompts: String,
    pub context: String,
}

pub fn update_summary(
    repo_root: &Path,
    checkpoint_id: &str,
    summary: serde_json::Value,
) -> Result<()> {
    ensure_metadata_branch(repo_root)?;
    let summary_view = read_committed(repo_root, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary_view);
    if session_count == 0 {
        anyhow::bail!("checkpoint not found");
    }
    let latest_index = session_count - 1;

    let metadata_ref = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let meta_tree_path = format!("{a}/{b}/{latest_index}/{}", paths::METADATA_FILE_NAME);
    let raw = git_show_file(repo_root, &metadata_ref, &meta_tree_path)
        .with_context(|| format!("reading session metadata at {meta_tree_path}"))?;
    let mut value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing session metadata at {meta_tree_path}"))?;
    value["summary"] = redact_json_value(&summary);

    let staging_dir = repo_root
        .join(paths::BITLOOPS_TMP_DIR)
        .join(format!("summary-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging_dir)?;
    let staged_meta = staging_dir.join("metadata.json");
    fs::write(
        &staged_meta,
        serde_json::to_string_pretty(&value).context("serializing updated summary metadata")?,
    )?;

    let (author_name, author_email) = get_git_author_from_repo(repo_root)?;
    let commit_result = commit_files_to_metadata_branch(
        repo_root,
        &[(staged_meta.clone(), meta_tree_path)],
        &format!("Update summary for checkpoint {checkpoint_id}"),
        &author_name,
        &author_email,
    );
    let _ = fs::remove_dir_all(&staging_dir);
    commit_result
}

pub fn list_committed(repo_root: &Path) -> Result<Vec<CommittedInfo>> {
    let Some(read_ref) = metadata_read_ref(repo_root) else {
        return Ok(vec![]);
    };

    let buckets = run_git(repo_root, &["ls-tree", "--name-only", &read_ref]).unwrap_or_default();
    let mut out: Vec<CommittedInfo> = vec![];

    for bucket in buckets.lines() {
        if bucket.len() != 2 || !bucket.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let bucket_ref = format!("{read_ref}:{bucket}");
        let children =
            run_git(repo_root, &["ls-tree", "--name-only", &bucket_ref]).unwrap_or_default();
        for suffix in children.lines() {
            if !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let checkpoint_id = format!("{bucket}{suffix}");
            if checkpoint_id.len() != 12 {
                continue;
            }
            if let Some(summary) = read_committed_with_ref(repo_root, &read_ref, &checkpoint_id)? {
                out.push(to_committed_info(repo_root, &read_ref, &summary));
            }
        }
    }

    Ok(out)
}

pub fn get_checkpoint_author(repo_root: &Path, checkpoint_id: &str) -> Result<CheckpointAuthor> {
    let metadata_ref = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    if run_git(repo_root, &["rev-parse", &metadata_ref]).is_err() {
        return Ok(CheckpointAuthor::default());
    }

    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let metadata_path = format!("{a}/{b}/{}", paths::METADATA_FILE_NAME);
    let log = run_git(
        repo_root,
        &[
            "log",
            "--reverse",
            "--format=%an|%ae",
            &metadata_ref,
            "--",
            &metadata_path,
        ],
    )
    .unwrap_or_default();
    let first = log.lines().next().unwrap_or_default().trim();
    if first.is_empty() {
        return Ok(CheckpointAuthor::default());
    }
    let mut parts = first.split('|');
    Ok(CheckpointAuthor {
        name: parts.next().unwrap_or_default().trim().to_string(),
        email: parts.next().unwrap_or_default().trim().to_string(),
    })
}

pub fn read_committed(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<Option<CheckpointSummaryView>> {
    let Some(read_ref) = metadata_read_ref(repo_root) else {
        return Ok(None);
    };
    read_committed_with_ref(repo_root, &read_ref, checkpoint_id)
}

fn read_committed_with_ref(
    repo_root: &Path,
    read_ref: &str,
    checkpoint_id: &str,
) -> Result<Option<CheckpointSummaryView>> {
    if checkpoint_type_for_ref(read_ref) != CheckpointType::Committed {
        return Ok(None);
    }

    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let metadata_path = format!("{a}/{b}/{}", paths::METADATA_FILE_NAME);
    let raw = match git_show_file(repo_root, read_ref, &metadata_path) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    let mut summary: CheckpointSummaryView = serde_json::from_str(&raw)
        .with_context(|| format!("parsing checkpoint {checkpoint_id}"))?;
    summary.session_count = summary.sessions.len();
    Ok(Some(summary))
}

/// Returns one committed checkpoint in list shape (session-derived fields included).
pub fn read_committed_info(repo_root: &Path, checkpoint_id: &str) -> Result<Option<CommittedInfo>> {
    let Some(read_ref) = metadata_read_ref(repo_root) else {
        return Ok(None);
    };
    let Some(summary) = read_committed_with_ref(repo_root, &read_ref, checkpoint_id)? else {
        return Ok(None);
    };
    Ok(Some(to_committed_info(repo_root, &read_ref, &summary)))
}

pub fn read_session_content(
    repo_root: &Path,
    checkpoint_id: &str,
    session_index: usize,
) -> Result<SessionContentView> {
    let summary = read_committed(repo_root, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary);
    if session_index >= session_count {
        anyhow::bail!("session {session_index} not found");
    }

    let metadata_ref =
        metadata_read_ref(repo_root).ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let base = format!("{a}/{b}/{session_index}");
    let metadata_path = format!("{base}/{}", paths::METADATA_FILE_NAME);
    let transcript_path = format!("{base}/{}", paths::TRANSCRIPT_FILE_NAME);
    let prompt_path = format!("{base}/{}", paths::PROMPT_FILE_NAME);
    let context_path = format!("{base}/{}", paths::CONTEXT_FILE_NAME);

    let metadata_raw = git_show_file(repo_root, &metadata_ref, &metadata_path)
        .with_context(|| format!("session {session_index} not found"))?;
    let metadata = serde_json::from_str::<serde_json::Value>(&metadata_raw)
        .context("parsing session metadata")?;
    let transcript = git_show_file(repo_root, &metadata_ref, &transcript_path).unwrap_or_default();
    let prompts = git_show_file(repo_root, &metadata_ref, &prompt_path).unwrap_or_default();
    let context = git_show_file(repo_root, &metadata_ref, &context_path).unwrap_or_default();

    Ok(SessionContentView {
        metadata,
        transcript,
        prompts,
        context,
    })
}

pub fn read_latest_session_content(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<SessionContentView> {
    let summary = read_committed(repo_root, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary);
    if session_count == 0 {
        anyhow::bail!("checkpoint has no sessions");
    }
    read_session_content(repo_root, checkpoint_id, session_count - 1)
}

pub fn read_session_content_by_id(
    repo_root: &Path,
    checkpoint_id: &str,
    session_id: &str,
) -> Result<SessionContentView> {
    let summary = read_committed(repo_root, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary);
    for idx in 0..session_count {
        // Skip unreadable session slots while searching by session ID.
        let Ok(content) = read_session_content(repo_root, checkpoint_id, idx) else {
            continue;
        };
        if content
            .metadata
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            == Some(session_id)
        {
            return Ok(content);
        }
    }
    anyhow::bail!("session {session_id:?} not found in checkpoint {checkpoint_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct CodeLearning {
    path: String,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    end_line: u32,
    finding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct LearningsSummary {
    repo: Vec<String>,
    code: Vec<CodeLearning>,
    workflow: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct Summary {
    intent: String,
    outcome: String,
    learnings: LearningsSummary,
    friction: Vec<String>,
    open_items: Vec<String>,
}

fn redact_summary(summary: Option<&Summary>) -> Result<Option<Summary>> {
    let Some(summary) = summary else {
        return Ok(None);
    };
    Ok(Some(Summary {
        intent: redact_text(&summary.intent),
        outcome: redact_text(&summary.outcome),
        learnings: LearningsSummary {
            repo: redact_string_slice(Some(&summary.learnings.repo))?.unwrap_or_default(),
            code: redact_code_learnings(Some(&summary.learnings.code))?.unwrap_or_default(),
            workflow: redact_string_slice(Some(&summary.learnings.workflow))?.unwrap_or_default(),
        },
        friction: redact_string_slice(Some(&summary.friction))?.unwrap_or_default(),
        open_items: redact_string_slice(Some(&summary.open_items))?.unwrap_or_default(),
    }))
}

fn redact_string_slice(values: Option<&[String]>) -> Result<Option<Vec<String>>> {
    let Some(values) = values else {
        return Ok(None);
    };
    Ok(Some(
        values.iter().map(|value| redact_text(value)).collect(),
    ))
}

fn redact_code_learnings(values: Option<&[CodeLearning]>) -> Result<Option<Vec<CodeLearning>>> {
    let Some(values) = values else {
        return Ok(None);
    };
    Ok(Some(
        values
            .iter()
            .map(|value| CodeLearning {
                path: value.path.clone(),
                line: value.line,
                end_line: value.end_line,
                finding: redact_text(&value.finding),
            })
            .collect(),
    ))
}

fn copy_metadata_dir(
    metadata_dir: &Path,
    base_path: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    add_directory_to_entries_with_abs_path(metadata_dir, base_path)
}

fn add_directory_to_entries_with_abs_path(
    metadata_dir: &Path,
    base_path: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut out: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    if !metadata_dir.exists() {
        return Ok(out);
    }

    let mut stack: Vec<PathBuf> = vec![metadata_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let lmeta = fs::symlink_metadata(&path)?;
            if lmeta.file_type().is_symlink() {
                continue;
            }
            if lmeta.is_dir() {
                stack.push(path);
                continue;
            }

            let rel = path
                .strip_prefix(metadata_dir)
                .with_context(|| format!("path traversal detected: {}", path.display()))?;
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.starts_with("..") {
                anyhow::bail!("path traversal detected: {rel}");
            }
            let key = format!(
                "{}/{}",
                base_path.trim_end_matches('/'),
                rel.trim_start_matches('/')
            );
            let content = fs::read(&path)?;
            let redacted_bytes = if key.ends_with(".jsonl") {
                redact_jsonl_bytes_with_fallback(&content)
            } else {
                redact_bytes(&content)
            };
            let redacted = String::from_utf8_lossy(&redacted_bytes).to_string();
            out.insert(key, redacted);
        }
    }

    Ok(out)
}

const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn ensure_metadata_branch(repo_root: &Path) -> Result<()> {
    let metadata_ref = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    if run_git(repo_root, &["rev-parse", &metadata_ref]).is_ok() {
        return Ok(());
    }
    let (author_name, author_email) = get_git_author_from_repo(repo_root)?;
    let commit = run_git_env(
        repo_root,
        &[
            "commit-tree",
            EMPTY_TREE_HASH,
            "-m",
            "Initialize checkpoints branch",
        ],
        &[
            ("GIT_AUTHOR_NAME", &author_name),
            ("GIT_AUTHOR_EMAIL", &author_email),
            ("GIT_COMMITTER_NAME", &author_name),
            ("GIT_COMMITTER_EMAIL", &author_email),
        ],
    )?;
    run_git(repo_root, &["update-ref", &metadata_ref, commit.trim()])?;
    Ok(())
}

pub(crate) fn commit_files_to_metadata_branch(
    repo_root: &Path,
    files: &[(PathBuf, String)],
    commit_message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<()> {
    ensure_metadata_branch(repo_root)?;
    let metadata_ref = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    let parent_tree = run_git(
        repo_root,
        &["rev-parse", &format!("{metadata_ref}^{{tree}}")],
    )
    .ok()
    .filter(|s| !s.is_empty());
    let parent_commit = run_git(repo_root, &["rev-parse", &metadata_ref])
        .ok()
        .filter(|s| !s.is_empty());

    let tree = build_tree_with_explicit_paths(repo_root, parent_tree.as_deref(), files)?;

    let mut ct_args: Vec<String> = vec!["commit-tree".into(), tree];
    if let Some(parent) = parent_commit {
        ct_args.push("-p".into());
        ct_args.push(parent);
    }
    ct_args.push("-m".into());
    ct_args.push(commit_message.to_string());
    let ct_args_ref: Vec<&str> = ct_args.iter().map(String::as_str).collect();
    let commit = run_git_env(
        repo_root,
        &ct_args_ref,
        &[
            ("GIT_AUTHOR_NAME", author_name),
            ("GIT_AUTHOR_EMAIL", author_email),
            ("GIT_COMMITTER_NAME", author_name),
            ("GIT_COMMITTER_EMAIL", author_email),
        ],
    )?;
    run_git(repo_root, &["update-ref", &metadata_ref, commit.trim()])?;
    Ok(())
}

pub(crate) fn git_show_file(repo_root: &Path, reference: &str, tree_path: &str) -> Result<String> {
    run_git(repo_root, &["show", &format!("{reference}:{tree_path}")])
}

pub(crate) fn git_show_file_bytes(
    repo_root: &Path,
    reference: &str,
    tree_path: &str,
) -> Result<Vec<u8>> {
    let output = new_git_command()
        .args(["show", &format!("{reference}:{tree_path}")])
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running git show {reference}:{tree_path}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git show {reference}:{tree_path} failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(output.stdout)
}

fn get_commit_author(repo_root: &Path, commit_ref: &str) -> Option<(String, String)> {
    let raw = run_git(repo_root, &["show", "-s", "--format=%an%n%ae", commit_ref]).ok()?;
    let mut lines = raw.lines();
    let name = lines.next().unwrap_or_default().trim().to_string();
    let email = lines.next().unwrap_or_default().trim().to_string();
    if name.is_empty() || email.is_empty() {
        return None;
    }
    Some((name, email))
}

pub(crate) fn metadata_read_ref(repo_root: &Path) -> Option<String> {
    let local = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    if run_git(repo_root, &["rev-parse", &local]).is_ok() {
        return Some(local);
    }
    let remote = format!("refs/remotes/origin/{}", paths::METADATA_BRANCH_NAME);
    if run_git(repo_root, &["rev-parse", &remote]).is_ok() {
        return Some(remote);
    }
    None
}

fn current_branch_name(repo_root: &Path) -> String {
    run_git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default()
}

fn redact_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact_text(s)),
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redact_json_value).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), redact_json_value(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn redact_bytes(input: &[u8]) -> Vec<u8> {
    redact::bytes(input).into_owned()
}

fn redact_jsonl_bytes_with_fallback(input: &[u8]) -> Vec<u8> {
    match redact::jsonl_bytes(input) {
        Ok(redacted) => redacted.into_owned(),
        Err(_) => redact::bytes(input).into_owned(),
    }
}

fn redact_text(input: &str) -> String {
    redact::string(input)
}
