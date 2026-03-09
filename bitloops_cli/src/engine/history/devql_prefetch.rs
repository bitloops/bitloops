use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::task;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryTarget {
    pub path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefetchResult {
    pub session_id: String,
    pub turn_id: String,
    pub query: String,
    pub targets: Vec<HistoryTarget>,
    pub rows: Value,
}

/// Best-effort pre-hook DevQL history prefetch.
/// Returns `Ok(None)` on normal no-op cases (no target anchors in prompt).
pub fn prefetch_for_prompt(
    repo_root: &Path,
    session_id: &str,
    turn_id: &str,
    prompt: &str,
) -> Result<Option<PrefetchResult>> {
    let Some(primary_target) = extract_line_anchors_from_prompt(prompt).into_iter().next() else {
        return Ok(None);
    };

    let repo_name = infer_repo_name(repo_root);
    let query = build_devql_history_query(&repo_name, &primary_target);
    let rows = run_devql_query(repo_root, &query)?;

    Ok(Some(PrefetchResult {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        query,
        targets: vec![primary_target],
        rows,
    }))
}

fn extract_line_anchors_from_prompt(prompt: &str) -> Vec<HistoryTarget> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+):(?P<start>\d+)(?:-(?P<end>\d+))?")
            .expect("valid anchor regex")
    });

    let mut targets = Vec::new();
    for caps in re.captures_iter(prompt) {
        let path = caps
            .name("path")
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        if path.is_empty() {
            continue;
        }

        let start_line = caps
            .name("start")
            .and_then(|m| m.as_str().parse::<i32>().ok());
        let end_line = caps
            .name("end")
            .and_then(|m| m.as_str().parse::<i32>().ok())
            .or(start_line);

        targets.push(HistoryTarget {
            path,
            start_line,
            end_line,
        });
    }

    targets
}

fn build_devql_history_query(repo_name: &str, target: &HistoryTarget) -> String {
    let escaped_repo = repo_name.replace('"', "\\\"");
    let escaped_path = target.path.replace('"', "\\\"");

    let artefacts_stage = match (target.start_line, target.end_line) {
        (Some(start), Some(end)) => format!("artefacts(lines:{start}..{end})"),
        _ => "artefacts()".to_string(),
    };

    format!(
        "repo(\"{escaped_repo}\")->file(\"{escaped_path}\")->{}->chatHistory()->limit(5)",
        artefacts_stage
    )
}

fn infer_repo_name(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-repo".to_string())
}

fn run_devql_query(repo_root: &Path, query: &str) -> Result<Value> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return task::block_in_place(|| {
            handle.block_on(crate::commands::devql::execute_query_json_for_repo_root(
                repo_root, query,
            ))
        });
    }

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for pre-hook DevQL query")?;
    runtime.block_on(crate::commands::devql::execute_query_json_for_repo_root(
        repo_root, query,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_line_anchors_parses_single_line_anchor() {
        let targets = extract_line_anchors_from_prompt("fix src/main.rs:42");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "src/main.rs");
        assert_eq!(targets[0].start_line, Some(42));
        assert_eq!(targets[0].end_line, Some(42));
    }

    #[test]
    fn build_history_query_includes_line_range_when_present() {
        let query = build_devql_history_query(
            "bitloops-cli",
            &HistoryTarget {
                path: "src/main.rs".to_string(),
                start_line: Some(10),
                end_line: Some(25),
            },
        );
        assert_eq!(
            query,
            "repo(\"bitloops-cli\")->file(\"src/main.rs\")->artefacts(lines:10..25)->chatHistory()->limit(5)"
        );
    }
}
