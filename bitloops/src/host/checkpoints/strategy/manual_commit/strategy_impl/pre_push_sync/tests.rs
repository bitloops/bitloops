use super::*;
use serde_json::json;

#[test]
fn parse_pre_push_update_line_accepts_branch_refs() {
    let line = "refs/heads/main abc123 refs/heads/main def456";
    let parsed = parsing::parse_pre_push_update_line(line).expect("parse branch update");
    assert_eq!(parsed.local_ref, "refs/heads/main");
    assert_eq!(parsed.remote_ref, "refs/heads/main");
    assert_eq!(parsed.local_branch.as_deref(), Some("main"));
    assert_eq!(parsed.remote_branch, "main");
}

#[test]
fn parse_pre_push_update_line_rejects_non_branch_remote_ref() {
    let line = "refs/heads/main abc123 refs/tags/v1 def456";
    assert!(
        parsing::parse_pre_push_update_line(line).is_none(),
        "tag pushes should be ignored by pre-push replication"
    );
}

#[test]
fn build_artefacts_replication_sql_targets_expected_columns() {
    let rows = vec![json!({
        "artefact_id": "a1",
        "symbol_id": "s1",
        "blob_sha": "b1",
        "path": "src/lib.rs",
        "language": "rust",
        "canonical_kind": "function",
        "language_kind": "function_item",
        "symbol_fqn": "src/lib.rs::run",
        "parent_artefact_id": null,
        "start_line": 1,
        "end_line": 3,
        "start_byte": 0,
        "end_byte": 10,
        "signature": "fn run()",
        "modifiers": "[]",
        "docstring": "test",
        "content_hash": "hash-1"
    })];

    let sql = history_replication::build_artefacts_replication_sql("repo-1", &rows).join("\n");
    assert!(sql.contains("INSERT INTO artefacts"));
    assert!(sql.contains("content_hash"));
    assert!(
        !sql.contains("created_at, created_at"),
        "artefacts replication SQL must not duplicate created_at columns"
    );
}
