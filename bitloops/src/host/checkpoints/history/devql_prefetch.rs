use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::task;

use crate::host::hooks::augmentation::prompt_target::{
    PromptTarget, extract_primary_prompt_target,
};

const DEVQL_PG_DSN_REQUIRED_ERROR_PREFIX: &str = "Postgres DSN is required for Postgres operations";
const DEVQL_PG_DSN_REQUIRED_NEW_PREFIX: &str =
    crate::host::devql::DEVQL_POSTGRES_DSN_REQUIRED_PREFIX;

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
    let rows = match run_devql_query(repo_root, &query) {
        Ok(rows) => rows,
        Err(err) if is_prefetch_backend_not_available_error(&err) => return Ok(None),
        Err(err) => return Err(err),
    };

    Ok(Some(PrefetchResult {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        query,
        targets: vec![primary_target],
        rows,
    }))
}

fn is_prefetch_backend_not_available_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        let message = cause.to_string();
        message.contains(DEVQL_PG_DSN_REQUIRED_ERROR_PREFIX)
            || message.contains(DEVQL_PG_DSN_REQUIRED_NEW_PREFIX)
    })
}

fn extract_history_target_from_prompt(repo_root: &Path, prompt: &str) -> Option<HistoryTarget> {
    extract_primary_prompt_target(repo_root, prompt).map(HistoryTarget::from)
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
            handle.block_on(crate::host::devql::execute_query_json_for_repo_root(
                repo_root, query,
            ))
        });
    }

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for pre-hook DevQL query")?;
    runtime.block_on(crate::host::devql::execute_query_json_for_repo_root(
        repo_root, query,
    ))
}

impl From<PromptTarget> for HistoryTarget {
    fn from(value: PromptTarget) -> Self {
        Self {
            path: value.path,
            start_line: value.start_line,
            end_line: value.end_line,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn extract_history_target_parses_single_line_anchor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("src").join("main.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "fn main() {}").expect("write");

        let target =
            extract_history_target_from_prompt(dir.path(), "fix src/main.rs:42").expect("target");
        assert_eq!(target.path, "src/main.rs");
        assert_eq!(target.start_line, Some(42));
        assert_eq!(target.end_line, Some(42));
    }

    #[test]
    fn extract_history_target_parses_hash_line_anchor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("src").join("main.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "fn main() {}").expect("write");

        let target = extract_history_target_from_prompt(dir.path(), "check src/main.rs#L10-L25")
            .expect("target");
        assert_eq!(target.path, "src/main.rs");
        assert_eq!(target.start_line, Some(10));
        assert_eq!(target.end_line, Some(25));
    }

    #[test]
    fn extract_history_target_falls_back_to_file_level_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("src").join("main.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "fn main() {}").expect("write");

        let target =
            extract_history_target_from_prompt(dir.path(), "please check src/main.rs for context")
                .expect("target");
        assert_eq!(target.path, "src/main.rs");
        assert_eq!(target.start_line, None);
        assert_eq!(target.end_line, None);
    }

    #[test]
    fn extract_history_target_ignores_nonexistent_tokens() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(
            extract_history_target_from_prompt(
                dir.path(),
                "await app.register(swagger, { openapi: { info: { title: 'My API' } } });",
            )
            .is_none()
        );
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

    #[test]
    fn missing_pg_dsn_error_is_detected() {
        let err = anyhow!(
            "Postgres DSN is required for Postgres operations (example: postgres://u:p@localhost:5432/db)"
        );
        assert!(is_prefetch_backend_not_available_error(&err));
    }

    #[test]
    fn non_pg_dsn_errors_are_not_detected() {
        let err = anyhow!("connection refused");
        assert!(!is_prefetch_backend_not_available_error(&err));
    }
}
