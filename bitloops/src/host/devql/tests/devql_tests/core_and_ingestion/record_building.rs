use super::*;
use crate::host::language_adapter::{LanguageKind, TsJsKind};

#[test]
fn build_file_current_record_preserves_file_metadata() {
    let cfg = test_cfg();
    let file = test_file_row(&cfg, "src/main.rs", "blob-1", 42, 420);
    let record = build_file_current_record(
        "src/main.rs",
        "blob-1",
        &file,
        Some("Top-level docs".to_string()),
    );

    assert_eq!(record.symbol_id, file.symbol_id);
    assert_eq!(record.artefact_id, file.artefact_id);
    assert_eq!(record.canonical_kind.as_deref(), Some("file"));
    assert_eq!(record.language_kind, "file");
    assert_eq!(record.symbol_fqn, "src/main.rs");
    assert_eq!(record.end_line, 42);
    assert_eq!(record.end_byte, 420);
    assert_eq!(record.docstring.as_deref(), Some("Top-level docs"));
    assert_eq!(record.content_hash, "blob-1");
}

#[test]
fn build_symbol_records_chain_file_and_nested_parent_links() {
    let cfg = test_cfg();
    let path = "src/ui.ts";
    let blob_sha = "blob-ui";
    let file = test_file_row(&cfg, path, blob_sha, 30, 300);
    let content = "export class Widget {\n  render(): void {}\n}\n";
    let items = vec![
        LanguageArtefact {
            canonical_kind: Some("class".to_string()),
            language_kind: LanguageKind::ts_js(TsJsKind::ClassDeclaration),
            name: "Widget".to_string(),
            symbol_fqn: format!("{path}::Widget"),
            parent_symbol_fqn: None,
            start_line: 1,
            end_line: 20,
            start_byte: 0,
            end_byte: 200,
            signature: "export class Widget {}".to_string(),
            modifiers: vec!["export".to_string()],
            docstring: Some("Widget docs".to_string()),
        },
        LanguageArtefact {
            canonical_kind: Some("method".to_string()),
            language_kind: LanguageKind::ts_js(TsJsKind::MethodDefinition),
            name: "render".to_string(),
            symbol_fqn: format!("{path}::Widget::render"),
            parent_symbol_fqn: Some(format!("{path}::Widget")),
            start_line: 5,
            end_line: 10,
            start_byte: 40,
            end_byte: 120,
            signature: "render(): void {}".to_string(),
            modifiers: vec![],
            docstring: None,
        },
    ];

    let records = build_symbol_records(&cfg, path, blob_sha, &file, &items, content);
    assert_eq!(records.len(), 2);

    let class_record = &records[0];
    assert_eq!(class_record.parent_symbol_id, Some(file.symbol_id.clone()));
    assert_eq!(
        class_record.parent_artefact_id,
        Some(file.artefact_id.clone())
    );
    assert_eq!(class_record.docstring.as_deref(), Some("Widget docs"));

    let method_record = &records[1];
    assert_eq!(
        method_record.parent_symbol_id,
        Some(class_record.symbol_id.clone())
    );
    assert_eq!(
        method_record.parent_artefact_id,
        Some(class_record.artefact_id.clone())
    );
    assert_eq!(
        method_record.signature.as_deref(),
        Some("render(): void {}")
    );
}

#[test]
fn build_symbol_records_keep_content_hash_stable_while_revision_artefact_id_changes_per_blob() {
    let cfg = test_cfg();
    let path = "src/ui.ts";
    let file_a = test_file_row(&cfg, path, "blob-a", 10, 100);
    let file_b = test_file_row(&cfg, path, "blob-b", 10, 100);
    let content = "export function render() {\n  return 1;\n}\n";
    let items = vec![LanguageArtefact {
        canonical_kind: Some("function".to_string()),
        language_kind: LanguageKind::ts_js(TsJsKind::FunctionDeclaration),
        name: "render".to_string(),
        symbol_fqn: format!("{path}::render"),
        parent_symbol_fqn: None,
        start_line: 1,
        end_line: 3,
        start_byte: 0,
        end_byte: content.len() as i32,
        signature: "export function render() {".to_string(),
        modifiers: vec!["export".to_string()],
        docstring: None,
    }];

    let first = build_symbol_records(&cfg, path, "blob-a", &file_a, &items, content);
    let second = build_symbol_records(&cfg, path, "blob-b", &file_b, &items, content);

    assert_eq!(first[0].content_hash, second[0].content_hash);
    assert_eq!(first[0].symbol_id, second[0].symbol_id);
    assert_ne!(first[0].artefact_id, second[0].artefact_id);
}

#[test]
fn build_historical_edge_records_keep_resolved_and_unresolved_targets() {
    let cfg = test_cfg();
    let path = "src/main.ts";
    let blob_sha = "blob-2";
    let from = test_symbol_record(&cfg, path, blob_sha, "from-symbol", "source", 1, 2);
    let to = test_symbol_record(&cfg, path, blob_sha, "to-symbol", "target", 4, 5);
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_historical_edge_records(
        &cfg,
        blob_sha,
        "typescript",
        vec![
            test_call_edge(&from.symbol_fqn, &to.symbol_fqn, 7),
            test_unresolved_call_edge(&from.symbol_fqn, "remote::symbol", 9),
            test_call_edge("missing::from", &to.symbol_fqn, 11),
        ],
        &current_by_fqn,
    );

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert!(records[0].to_symbol_ref.is_none());
    assert!(records[1].to_symbol_id.is_none());
    assert!(records[1].to_artefact_id.is_none());
    assert_eq!(records[1].to_symbol_ref.as_deref(), Some("remote::symbol"));
}

#[test]
fn build_current_edge_records_resolve_local_and_external_targets() {
    let cfg = test_cfg();
    let path = "src/main.ts";
    let blob_sha = "blob-3";
    let from = test_symbol_record(&cfg, path, blob_sha, "from-symbol", "source", 1, 2);
    let to = test_symbol_record(&cfg, path, blob_sha, "to-symbol", "target", 4, 5);
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let external_targets = [(
        "pkg::remote".to_string(),
        (
            "external-symbol".to_string(),
            "external-artefact".to_string(),
        ),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_current_edge_records(
        &cfg,
        path,
        "typescript",
        vec![
            test_call_edge(&from.symbol_fqn, &to.symbol_fqn, 7),
            test_unresolved_call_edge(&from.symbol_fqn, "pkg::remote", 8),
        ],
        &current_by_fqn,
        &external_targets,
    );

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert_eq!(records[1].to_symbol_id.as_deref(), Some("external-symbol"));
    assert_eq!(
        records[1].to_artefact_id.as_deref(),
        Some("external-artefact")
    );
    assert_eq!(records[1].to_symbol_ref.as_deref(), Some("pkg::remote"));
}

#[test]
fn build_historical_edge_records_resolve_explicit_local_rust_symbol_refs() {
    let cfg = test_cfg();
    let helper_path = "crates/ruff_linter/src/rules/pyflakes/fixes.rs";
    let caller_path = "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs";
    let blob_sha = "blob-rust";
    let from = test_symbol_record(
        &cfg,
        caller_path,
        blob_sha,
        "caller-symbol",
        "string_dot_format_extra_positional_arguments",
        1,
        4,
    );
    let to = test_symbol_record(
        &cfg,
        helper_path,
        blob_sha,
        "helper-symbol",
        "remove_unused_positional_arguments_from_format_call",
        1,
        1,
    );
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_historical_edge_records(
        &cfg,
        blob_sha,
        "rust",
        vec![DependencyEdge {
            edge_kind: EdgeKind::Calls,
            from_symbol_fqn: from.symbol_fqn.clone(),
            to_target_symbol_fqn: None,
            to_symbol_ref: Some(
                "super::super::fixes::remove_unused_positional_arguments_from_format_call"
                    .to_string(),
            ),
            start_line: Some(3),
            end_line: Some(3),
            metadata: EdgeMetadata::call(CallForm::Function, Resolution::Import),
        }],
        &current_by_fqn,
    );

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert_eq!(
        records[0].to_symbol_ref.as_deref(),
        Some(to.symbol_fqn.as_str())
    );
}

#[test]
fn incoming_revision_is_newer_prefers_revision_kind_then_timestamp_then_sha() {
    let state =
        |_commit_sha: &str, revision_kind: &str, revision_id: &str, updated_at_unix: i64| {
            CurrentFileRevisionRecord {
                revision_kind: TemporalRevisionKind::from_str(revision_kind)
                    .expect("test revision kind should be valid"),
                revision_id: revision_id.to_string(),
                blob_sha: "blob".to_string(),
                updated_at_unix,
            }
        };
    assert!(incoming_revision_is_newer(
        None,
        TemporalRevisionKind::Commit,
        "bbb",
        10
    ));
    let existing_1 = state("aaa", "commit", "aaa", 9);
    assert!(incoming_revision_is_newer(
        Some(&existing_1),
        TemporalRevisionKind::Commit,
        "bbb",
        10
    ));
    let existing_2 = state("zzz", "commit", "zzz", 11);
    assert!(!incoming_revision_is_newer(
        Some(&existing_2),
        TemporalRevisionKind::Commit,
        "bbb",
        10
    ));
    let existing_3 = state("aaa", "commit", "aaa", 10);
    assert!(incoming_revision_is_newer(
        Some(&existing_3),
        TemporalRevisionKind::Commit,
        "bbb",
        10
    ));
    let existing_4 = state("ccc", "commit", "ccc", 10);
    assert!(!incoming_revision_is_newer(
        Some(&existing_4),
        TemporalRevisionKind::Commit,
        "bbb",
        10
    ));
    let existing_5 = state("temp:9", "temporary", "temp:9", 10);
    assert!(incoming_revision_is_newer(
        Some(&existing_5),
        TemporalRevisionKind::Temporary,
        "temp:10",
        10
    ));
    let existing_6 = state("temp:10", "temporary", "temp:10", 10);
    assert!(!incoming_revision_is_newer(
        Some(&existing_6),
        TemporalRevisionKind::Temporary,
        "temp:9",
        10
    ));
    let existing_7 = state("commit-a", "commit", "commit-a", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_7),
        TemporalRevisionKind::Temporary,
        "temp:200",
        200
    ));
    let existing_7b = state("commit-a", "commit", "commit-a", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_7b),
        TemporalRevisionKind::Temporary,
        "temp:201",
        100
    ));
    let existing_8 = state("commit-a", "temporary", "temp:88", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_8),
        TemporalRevisionKind::Commit,
        "commit-b",
        100
    ));
}
