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
    let Some(primary_target) = extract_history_target_from_prompt(repo_root, prompt) else {
        return Ok(None);
    };

    let query = build_devql_history_query(&primary_target);
    let rows = run_devql_query(repo_root, &query)?;

    Ok(Some(PrefetchResult {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        query,
        targets: vec![primary_target],
        rows,
    }))
}

fn extract_history_target_from_prompt(repo_root: &Path, prompt: &str) -> Option<HistoryTarget> {
    let targets = extract_candidate_history_targets_from_prompt(prompt);
    let primary_target = get_history_target(repo_root, targets)?;
    Some(primary_target)
}

fn extract_candidate_history_targets_from_prompt(prompt: &str) -> Vec<HistoryTarget> {
    static COLON_LINE_RE: OnceLock<Regex> = OnceLock::new();
    static HASH_LINE_RE: OnceLock<Regex> = OnceLock::new();
    static LINES_PARENS_RE: OnceLock<Regex> = OnceLock::new();
    static FILE_ONLY_RE: OnceLock<Regex> = OnceLock::new();

    let colon_line_re = COLON_LINE_RE.get_or_init(|| {
        Regex::new(r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+):(?P<start>\d+)(?:-(?P<end>\d+))?")
            .expect("valid colon line anchor regex")
    });
    let hash_line_re = HASH_LINE_RE.get_or_init(|| {
        Regex::new(
            r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+)#L(?P<start>\d+)(?:-L?(?P<end>\d+))?",
        )
        .expect("valid hash line anchor regex")
    });
    let lines_parens_re = LINES_PARENS_RE.get_or_init(|| {
        Regex::new(
            r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+)\s*\(lines?\s*(?P<start>\d+)(?:\s*-\s*(?P<end>\d+))?\)",
        )
        .expect("valid lines() anchor regex")
    });
    let file_only_re = FILE_ONLY_RE.get_or_init(|| {
        Regex::new(r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+)").expect("valid file-only regex")
    });

    let mut targets = Vec::<HistoryTarget>::new();
    for caps in colon_line_re.captures_iter(prompt) {
        insert_target(
            &mut targets,
            caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
            caps.name("start")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
            caps.name("end")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
        );
    }
    for caps in hash_line_re.captures_iter(prompt) {
        insert_target(
            &mut targets,
            caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
            caps.name("start")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
            caps.name("end")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
        );
    }
    for caps in lines_parens_re.captures_iter(prompt) {
        insert_target(
            &mut targets,
            caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
            caps.name("start")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
            caps.name("end")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
        );
    }

    if targets.is_empty() {
        for caps in file_only_re.captures_iter(prompt) {
            insert_target(
                &mut targets,
                caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
                None,
                None,
            );
        }
    }

    targets
}

fn insert_target(
    targets: &mut Vec<HistoryTarget>,
    path: &str,
    start_line: Option<i32>,
    end_line: Option<i32>,
) {
    let Some(path) = sanitize_target_path(path) else {
        return;
    };

    let final_end = end_line.or(start_line);
    if let Some(existing) = targets.iter_mut().find(|t| t.path == path) {
        if existing.start_line.is_none() && start_line.is_some() {
            existing.start_line = start_line;
            existing.end_line = final_end;
        }
        return;
    }

    targets.push(HistoryTarget {
        path,
        start_line,
        end_line: final_end,
    });
}

fn sanitize_target_path(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = path.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | ',' | ';' | ')' | ']'));
    if path.is_empty() || path.starts_with("http://") || path.starts_with("https://") {
        return None;
    }
    Some(path.to_string())
}

fn get_history_target(repo_root: &Path, targets: Vec<HistoryTarget>) -> Option<HistoryTarget> {
    targets.into_iter().find_map(|target| {
        normalize_target_for_repo(repo_root, &target.path).and_then(|repo_relative_path| {
            if repo_root.join(&repo_relative_path).is_file() {
                let mut normalized = target;
                normalized.path = repo_relative_path;
                Some(normalized)
            } else {
                None
            }
        })
    })
}

fn normalize_target_for_repo(repo_root: &Path, raw_path: &str) -> Option<String> {
    let repo_root = repo_root.canonicalize().ok()?;
    let input = Path::new(raw_path);
    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repo_root.join(input)
    };
    let candidate = candidate.canonicalize().ok()?;
    if !candidate.starts_with(&repo_root) {
        return None;
    }
    let rel = candidate.strip_prefix(&repo_root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn build_devql_history_query(target: &HistoryTarget) -> String {
    let escaped_path = target.path.replace('"', "\\\"");

    let artefacts_stage = match (target.start_line, target.end_line) {
        (Some(start), Some(end)) => format!("artefacts(lines:{start}..{end})"),
        _ => "artefacts()".to_string(),
    };

    format!(
        "file(\"{escaped_path}\")->{}->chatHistory()->limit(5)",
        artefacts_stage
    )
}

fn run_devql_query(repo_root: &Path, query: &str) -> Result<Value> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return task::block_in_place(|| {
            handle.block_on(crate::engine::devql::execute_query_json_for_repo_root(
                repo_root, query,
            ))
        });
    }

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for pre-hook DevQL query")?;
    runtime.block_on(crate::engine::devql::execute_query_json_for_repo_root(
        repo_root, query,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_targets_parses_single_line_anchor() {
        let targets = extract_candidate_history_targets_from_prompt("fix src/main.rs:42");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "src/main.rs");
        assert_eq!(targets[0].start_line, Some(42));
        assert_eq!(targets[0].end_line, Some(42));
    }

    #[test]
    fn extract_targets_parses_hash_line_anchor() {
        let targets = extract_candidate_history_targets_from_prompt("check src/main.rs#L10-L25");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "src/main.rs");
        assert_eq!(targets[0].start_line, Some(10));
        assert_eq!(targets[0].end_line, Some(25));
    }

    #[test]
    fn extract_targets_falls_back_to_file_level_history() {
        let targets =
            extract_candidate_history_targets_from_prompt("please check src/main.rs for context");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "src/main.rs");
        assert_eq!(targets[0].start_line, None);
        assert_eq!(targets[0].end_line, None);
    }

    #[test]
    fn extract_targets_returns_non_file_tokens() {
        let targets = extract_candidate_history_targets_from_prompt(
            "await app.register(swagger, { openapi: { info: { title: 'My API' } } });",
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, "app.register");
    }

    #[test]
    fn select_existing_target_ignores_nonexistent_tokens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let targets = extract_candidate_history_targets_from_prompt(
            "await app.register(swagger, { openapi: { info: { title: 'My API' } } });",
        );
        assert!(get_history_target(dir.path(), targets).is_none());
    }

    #[test]
    fn get_history_target_converts_absolute_path_to_repo_relative() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("src").join("main.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "fn main() {}").expect("write");

        let targets = vec![HistoryTarget {
            path: file.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(1),
        }];
        let selected = get_history_target(dir.path(), targets).expect("target selected");
        assert_eq!(selected.path, "src/main.rs");
    }

    #[test]
    fn build_history_query_includes_line_range_when_present() {
        let query = build_devql_history_query(&HistoryTarget {
            path: "src/main.rs".to_string(),
            start_line: Some(10),
            end_line: Some(25),
        });
        assert_eq!(
            query,
            "file(\"src/main.rs\")->artefacts(lines:10..25)->chatHistory()->limit(5)"
        );
    }
}
