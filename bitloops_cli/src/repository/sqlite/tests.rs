use rusqlite::{Connection, params};
use tempfile::TempDir;

use super::SqliteTestHarnessRepository;
use crate::db::init_database;
use crate::models::{
    CommitRecord, CurrentFileStateRecord, CurrentProductionArtefactRecord, FileStateRecord,
    ProductionArtefactRecord, ProductionIngestionBatch, RepositoryRecord,
};
use crate::repository::{TestHarnessQueryRepository, TestHarnessRepository};

struct SampleBatch<'a> {
    repo_id: &'a str,
    commit_sha: &'a str,
    path: &'a str,
    blob_sha: &'a str,
    artefact_id: &'a str,
    symbol_id: &'a str,
    canonical_kind: &'a str,
    symbol_fqn: Option<&'a str>,
}

#[test]
fn load_repo_id_for_commit_supports_workspace_crate_paths() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("workspace-layout.db");
    init_database(&db_path, false, "seed").expect("failed to initialize db");

    let mut repository = SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
    repository
        .replace_production_artefacts(&sample_batch(SampleBatch {
            repo_id: "ruff-workspace",
            commit_sha: "commit-workspace",
            path: "crates/ruff/src/lib.rs",
            blob_sha: "blob-a",
            artefact_id: "file:workspace",
            symbol_id: "sym:file:workspace",
            canonical_kind: "file",
            symbol_fqn: Some("crates/ruff/src/lib.rs"),
        }))
        .expect("replace production artefacts");

    let repo_id = repository
        .load_repo_id_for_commit("commit-workspace")
        .expect("load repo id");
    assert_eq!(repo_id, "ruff-workspace");
}

#[test]
fn load_production_artefacts_includes_workspace_crate_functions() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("workspace-functions.db");
    init_database(&db_path, false, "seed").expect("failed to initialize db");

    let mut repository = SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
    repository
        .replace_production_artefacts(&sample_batch(SampleBatch {
            repo_id: "ruff-workspace",
            commit_sha: "commit-workspace",
            path: "crates/ruff/src/version.rs",
            blob_sha: "blob-b",
            artefact_id: "function:workspace",
            symbol_id: "sym:function:workspace",
            canonical_kind: "function",
            symbol_fqn: Some("crates/ruff/src/version.rs::version"),
        }))
        .expect("replace production artefacts");

    let artefacts = repository
        .load_production_artefacts("commit-workspace")
        .expect("load production artefacts");
    assert_eq!(artefacts.len(), 1);
    assert_eq!(artefacts[0].artefact_id, "function:workspace");
}

#[test]
fn list_and_find_artefacts_fall_back_to_language_kind_when_canonical_kind_is_null() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("workspace-null-kind.db");
    init_database(&db_path, false, "seed").expect("failed to initialize db");

    let mut repository = SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
    repository
        .replace_production_artefacts(&sample_batch(SampleBatch {
            repo_id: "ruff-workspace",
            commit_sha: "commit-workspace",
            path: "src/lib.rs",
            blob_sha: "blob-c",
            artefact_id: "file:workspace",
            symbol_id: "sym:file:workspace",
            canonical_kind: "file",
            symbol_fqn: Some("src/lib.rs"),
        }))
        .expect("replace production artefacts");

    let conn = Connection::open(&db_path).expect("open sqlite connection");
    conn.execute(
        r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind,
  symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature,
  modifiers, docstring, content_hash
) VALUES (
  ?1, ?2, ?3, ?4, ?5, 'rust', NULL, 'Struct', ?6, ?7, 10, 20, 100, 240, NULL, '[]', NULL, 'hash-struct'
)
"#,
        params![
            "struct:workspace",
            "sym:struct:workspace",
            "ruff-workspace",
            "blob-c",
            "src/lib.rs",
            "src/lib.rs::Struct",
            "file:workspace",
        ],
    )
    .expect("insert struct artefact");

    let listed = repository
        .list_artefacts("commit-workspace", None)
        .expect("list artefacts");
    assert!(listed.iter().any(|artefact| {
        artefact.symbol_fqn.as_deref() == Some("src/lib.rs::Struct") && artefact.kind == "struct"
    }));

    let structs = repository
        .list_artefacts("commit-workspace", Some("struct"))
        .expect("list struct artefacts");
    assert_eq!(structs.len(), 1);
    assert_eq!(structs[0].kind, "struct");

    let queried = repository
        .find_artefact("commit-workspace", "Struct")
        .expect("find struct artefact");
    assert_eq!(queried.artefact_id, "struct:workspace");
    assert_eq!(queried.canonical_kind, "struct");
}

fn sample_batch(sample: SampleBatch<'_>) -> ProductionIngestionBatch {
    ProductionIngestionBatch {
        repository: RepositoryRecord {
            repo_id: sample.repo_id.to_string(),
            provider: "local".to_string(),
            organization: "local".to_string(),
            name: "repo".to_string(),
            default_branch: Some("main".to_string()),
        },
        commit: CommitRecord {
            commit_sha: sample.commit_sha.to_string(),
            repo_id: sample.repo_id.to_string(),
            author_name: None,
            author_email: None,
            commit_message: None,
            committed_at: Some("2026-03-18T00:00:00Z".to_string()),
        },
        file_states: vec![FileStateRecord {
            repo_id: sample.repo_id.to_string(),
            commit_sha: sample.commit_sha.to_string(),
            path: sample.path.to_string(),
            blob_sha: sample.blob_sha.to_string(),
        }],
        current_file_states: vec![CurrentFileStateRecord {
            repo_id: sample.repo_id.to_string(),
            path: sample.path.to_string(),
            commit_sha: sample.commit_sha.to_string(),
            blob_sha: sample.blob_sha.to_string(),
            committed_at: "2026-03-18T00:00:00Z".to_string(),
        }],
        artefacts: vec![ProductionArtefactRecord {
            artefact_id: sample.artefact_id.to_string(),
            symbol_id: sample.symbol_id.to_string(),
            repo_id: sample.repo_id.to_string(),
            blob_sha: sample.blob_sha.to_string(),
            path: sample.path.to_string(),
            language: "rust".to_string(),
            canonical_kind: sample.canonical_kind.to_string(),
            language_kind: Some("function_item".to_string()),
            symbol_fqn: sample.symbol_fqn.map(str::to_string),
            parent_artefact_id: None,
            start_line: 1,
            end_line: 5,
            start_byte: 0,
            end_byte: 32,
            signature: Some("pub fn version() -> String".to_string()),
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: Some("hash".to_string()),
        }],
        current_artefacts: vec![CurrentProductionArtefactRecord {
            repo_id: sample.repo_id.to_string(),
            symbol_id: sample.symbol_id.to_string(),
            artefact_id: sample.artefact_id.to_string(),
            commit_sha: sample.commit_sha.to_string(),
            blob_sha: sample.blob_sha.to_string(),
            path: sample.path.to_string(),
            language: "rust".to_string(),
            canonical_kind: sample.canonical_kind.to_string(),
            language_kind: Some("function_item".to_string()),
            symbol_fqn: sample.symbol_fqn.map(str::to_string),
            parent_symbol_id: None,
            parent_artefact_id: None,
            start_line: 1,
            end_line: 5,
            start_byte: 0,
            end_byte: 32,
            signature: Some("pub fn version() -> String".to_string()),
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: Some("hash".to_string()),
        }],
        edges: Vec::new(),
        current_edges: Vec::new(),
    }
}
