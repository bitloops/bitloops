use super::*;

pub(crate) fn get_git_author_from_repo(repo_root: &Path) -> Result<(String, String)> {
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct CodeLearning {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) line: u32,
    #[serde(default)]
    pub(crate) end_line: u32,
    pub(crate) finding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct LearningsSummary {
    pub(crate) repo: Vec<String>,
    pub(crate) code: Vec<CodeLearning>,
    pub(crate) workflow: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub(crate) intent: String,
    pub(crate) outcome: String,
    pub(crate) learnings: LearningsSummary,
    pub(crate) friction: Vec<String>,
    pub(crate) open_items: Vec<String>,
}

pub(crate) fn redact_summary(summary: Option<&Summary>) -> Result<Option<Summary>> {
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

pub(crate) fn redact_string_slice(values: Option<&[String]>) -> Result<Option<Vec<String>>> {
    let Some(values) = values else {
        return Ok(None);
    };
    Ok(Some(
        values.iter().map(|value| redact_text(value)).collect(),
    ))
}

pub(crate) fn redact_code_learnings(
    values: Option<&[CodeLearning]>,
) -> Result<Option<Vec<CodeLearning>>> {
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

#[cfg(test)]
pub(crate) fn copy_metadata_dir(
    metadata_dir: &Path,
    base_path: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    add_directory_to_entries_with_abs_path(metadata_dir, base_path)
}

#[cfg(test)]
pub(crate) fn add_directory_to_entries_with_abs_path(
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

#[allow(dead_code)]
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

pub(crate) fn get_commit_author(repo_root: &Path, commit_ref: &str) -> Option<(String, String)> {
    let raw = run_git(repo_root, &["show", "-s", "--format=%an%n%ae", commit_ref]).ok()?;
    let mut lines = raw.lines();
    let name = lines.next().unwrap_or_default().trim().to_string();
    let email = lines.next().unwrap_or_default().trim().to_string();
    if name.is_empty() || email.is_empty() {
        return None;
    }
    Some((name, email))
}

#[allow(dead_code)]
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

pub(crate) fn current_branch_name(repo_root: &Path) -> String {
    run_git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default()
}

pub(crate) fn redact_json_value(value: &serde_json::Value) -> serde_json::Value {
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

pub(crate) fn redact_bytes(input: &[u8]) -> Vec<u8> {
    redact::bytes(input).into_owned()
}

pub(crate) fn redact_jsonl_bytes_with_fallback(input: &[u8]) -> Vec<u8> {
    match redact::jsonl_bytes(input) {
        Ok(redacted) => redacted.into_owned(),
        Err(_) => redact::bytes(input).into_owned(),
    }
}

pub(crate) fn redact_text(input: &str) -> String {
    redact::string(input)
}
