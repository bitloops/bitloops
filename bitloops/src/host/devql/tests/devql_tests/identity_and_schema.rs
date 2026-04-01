use super::*;

#[test]
fn symbol_id_is_stable_when_impl_block_moves_lines() {
    let original = LanguageArtefact {
        canonical_kind: None,
        language_kind: "impl_item".to_string(),
        name: "impl@12".to_string(),
        symbol_fqn: "src/lib.rs::impl@12".to_string(),
        parent_symbol_fqn: None,
        start_line: 12,
        end_line: 18,
        start_byte: 0,
        end_byte: 0,
        signature: "impl Repo for PgRepo {".to_string(),
        modifiers: vec![],
        docstring: None,
    };
    let moved = LanguageArtefact {
        name: "impl@30".to_string(),
        symbol_fqn: "src/lib.rs::impl@30".to_string(),
        start_line: 30,
        end_line: 36,
        ..original.clone()
    };

    assert_eq!(
        structural_symbol_id_for_artefact(&original, None),
        structural_symbol_id_for_artefact(&moved, None)
    );
}

#[test]
fn revision_artefact_id_changes_per_blob_for_same_symbol() {
    let symbol_id = deterministic_uuid("stable-symbol");

    assert_eq!(
        revision_artefact_id("repo-1", "blob-a", &symbol_id),
        revision_artefact_id("repo-1", "blob-a", &symbol_id)
    );
    assert_ne!(
        revision_artefact_id("repo-1", "blob-a", &symbol_id),
        revision_artefact_id("repo-1", "blob-b", &symbol_id)
    );
}

#[test]
fn reingestion_is_idempotent_for_unchanged_js_symbols() {
    let content = r#"export function greet(name: string) {
  return name.trim();
}
"#;
    let first = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let second = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let first_fn = first
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/sample.ts::greet")
        .expect("expected greet artefact in first ingest");
    let second_fn = second
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/sample.ts::greet")
        .expect("expected greet artefact in second ingest");

    let first_symbol_id = symbol_id_for_artefact(first_fn);
    let second_symbol_id = symbol_id_for_artefact(second_fn);
    assert_eq!(first_symbol_id, second_symbol_id);
    assert_eq!(
        revision_artefact_id("repo-1", "blob-a", &first_symbol_id),
        revision_artefact_id("repo-1", "blob-a", &second_symbol_id)
    );
}

#[test]
fn reingestion_preserves_symbol_continuity_across_rust_line_moves() {
    let original = r#"struct Repo;
trait Service {
    fn run(&self);
}
impl Service for Repo {
    fn run(&self) {}
}
"#;
    let moved = r#"

struct Repo;

trait Service {
    fn run(&self);
}

impl Service for Repo {
    fn run(&self) {}
}
"#;

    let original_artefacts = extract_rust_artefacts(original, "src/lib.rs").unwrap();
    let moved_artefacts = extract_rust_artefacts(moved, "src/lib.rs").unwrap();

    let original_impl = original_artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "impl_item")
        .expect("expected impl artefact in original ingest");
    let moved_impl = moved_artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "impl_item")
        .expect("expected impl artefact in moved ingest");
    let original_impl_symbol_id = structural_symbol_id_for_artefact(original_impl, None);
    let moved_impl_symbol_id = structural_symbol_id_for_artefact(moved_impl, None);
    assert_eq!(original_impl_symbol_id, moved_impl_symbol_id);

    let original_method = original_artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected run method in original ingest");
    let moved_method = moved_artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected run method in moved ingest");

    let original_method_symbol_id =
        structural_symbol_id_for_artefact(original_method, Some(&original_impl_symbol_id));
    let moved_method_symbol_id =
        structural_symbol_id_for_artefact(moved_method, Some(&moved_impl_symbol_id));

    assert_eq!(original_method_symbol_id, moved_method_symbol_id);
    assert_ne!(
        revision_artefact_id("repo-1", "blob-a", &original_method_symbol_id),
        revision_artefact_id("repo-1", "blob-b", &moved_method_symbol_id)
    );
}

#[test]
fn postgres_schema_sql_includes_artefact_edges_hardening() {
    let sql = postgres_schema_sql();
    assert!(sql.contains("symbol_id TEXT"));
    assert!(sql.contains("modifiers JSONB NOT NULL DEFAULT '[]'::jsonb"));
    assert!(sql.contains("docstring TEXT"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_symbol_idx"));
    assert!(sql.contains("content_id TEXT NOT NULL"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts_current"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges_current"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS checkpoint_file_snapshots"));
    assert!(sql.contains("event_time TIMESTAMPTZ NOT NULL"));
    assert!(sql.contains("PRIMARY KEY (repo_id, checkpoint_id, path, blob_sha)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_lookup_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_agent_time_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_event_time_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_checkpoint_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_commit_idx"));
    assert!(sql.contains("PRIMARY KEY (repo_id, path, symbol_id)"));
    assert!(!sql.contains("CREATE TABLE IF NOT EXISTS sync_state"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges"));
    assert!(sql.contains("CONSTRAINT artefact_edges_target_chk"));
    assert!(sql.contains("CONSTRAINT artefact_edges_line_range_chk"));
    assert!(sql.contains("metadata JSONB DEFAULT '{}'::jsonb"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx"));
    assert!(sql.contains("CONSTRAINT artefact_edges_current_target_chk"));
    assert!(sql.contains("CONSTRAINT artefact_edges_current_line_range_chk"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx"));
}

#[test]
fn sqlite_schema_sql_includes_sync_state_table() {
    let sql = sqlite_schema_sql();
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS checkpoint_file_snapshots"));
    assert!(sql.contains("event_time TEXT NOT NULL"));
    assert!(sql.contains("PRIMARY KEY (repo_id, checkpoint_id, path, blob_sha)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_lookup_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_agent_time_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_event_time_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_checkpoint_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_commit_idx"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS sync_state"));
    assert!(sql.contains("PRIMARY KEY (repo_id, state_key)"));
}

#[test]
fn artefact_edges_hardening_sql_includes_constraints_and_indexes() {
    let sql = artefact_edges_hardening_sql();
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_target_chk"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_line_range_chk"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx"));
}

#[test]
fn current_state_hardening_sql_includes_current_state_constraints_and_indexes() {
    let sql = current_state_hardening_sql();
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS branch TEXT"));
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS commit_sha TEXT"));
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS modifiers JSONB"));
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS docstring TEXT"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_branch_fqn_idx"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_current_target_chk"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_current_line_range_chk"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq"));
}

#[test]
fn artefacts_upgrade_sql_adds_modifiers_and_docstring() {
    let sql = artefacts_upgrade_sql();
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS modifiers JSONB"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS docstring TEXT"));
    assert!(sql.contains("SET modifiers = '[]'::jsonb"));
    assert!(sql.contains("ALTER COLUMN modifiers SET NOT NULL"));
}

#[test]
fn incoming_revision_is_newer_rejects_older_commits_and_uses_commit_sha_as_tiebreaker() {
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
        "commit-b",
        200
    ));
    let existing_1 = state("commit-a", "commit", "commit-a", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_1),
        TemporalRevisionKind::Commit,
        "commit-b",
        200
    ));
    let existing_2 = state("commit-a", "commit", "commit-a", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_2),
        TemporalRevisionKind::Commit,
        "commit-b",
        100
    ));
    let existing_3 = state("commit-b", "commit", "commit-b", 200);
    assert!(!incoming_revision_is_newer(
        Some(&existing_3),
        TemporalRevisionKind::Commit,
        "commit-a",
        100
    ));
    let existing_4 = state("commit-z", "commit", "commit-z", 200);
    assert!(!incoming_revision_is_newer(
        Some(&existing_4),
        TemporalRevisionKind::Commit,
        "commit-a",
        200
    ));
}

#[test]
fn devql_ingest_rejects_removed_init_flag() {
    let err = match crate::cli::Cli::try_parse_from(["bitloops", "devql", "ingest", "--init=false"])
    {
        Ok(_) => panic!("devql ingest --init flag should be rejected"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("--init"));
}

#[test]
fn resolve_repo_id_for_query_is_strict_for_unknown_repo_names() {
    let cfg = test_cfg();

    let local = resolve_repo_id_for_query(&cfg, Some("temp2"));
    let unknown = resolve_repo_id_for_query(&cfg, Some("test2"));

    assert_eq!(local, cfg.repo.repo_id);
    assert_ne!(unknown, cfg.repo.repo_id);
}

#[test]
fn postgres_sslmode_validation_allows_default_dsn_without_sslmode() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops";
    let pg_cfg: tokio_postgres::Config = dsn.parse().expect("valid dsn");
    assert!(matches!(pg_cfg.get_ssl_mode(), SslMode::Prefer));
    validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode()).expect("prefer is allowed");
}

#[test]
fn postgres_sslmode_validation_rejects_require() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops?sslmode=require";
    let pg_cfg: tokio_postgres::Config = dsn.parse().expect("valid dsn");
    let err = validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode()).unwrap_err();
    assert!(
        err.to_string()
            .contains("Postgres DSN requires TLS (sslmode=Require)")
    );
}

#[test]
fn postgres_sslmode_validation_rejects_verify_full_dsn() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops?sslmode=verify-full";
    let err = validate_postgres_sslmode_for_notls(dsn, SslMode::Prefer).unwrap_err();
    assert!(err.to_string().contains("sslmode=verify-ca/verify-full"));
}
