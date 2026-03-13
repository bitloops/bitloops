use std::path::Path;

use rusqlite::{Connection, params};

pub struct ProductionArtefactSeed {
    pub artefact_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
    pub language_kind: Option<String>,
    pub symbol_fqn: Option<String>,
    pub parent_artefact_id: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub signature: Option<String>,
}

pub fn initialize_schema(db_path: &Path) {
    let conn = Connection::open(db_path).expect("failed to open sqlite db");
    conn.execute_batch(testlens::db::schema::SCHEMA_SQL)
        .expect("failed to create schema");
}

pub fn seed_production_artefacts(db_path: &Path, artefacts: &[ProductionArtefactSeed]) {
    let mut conn = Connection::open(db_path).expect("failed to open sqlite db");
    let tx = conn
        .transaction()
        .expect("failed to start seed transaction");

    for artefact in artefacts {
        tx.execute(
            r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, commit_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash
) VALUES (
  ?1, NULL, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, ?12, NULL
)
"#,
            params![
                artefact.artefact_id,
                artefact.repo_id,
                artefact.commit_sha,
                artefact.path,
                artefact.language,
                artefact.canonical_kind,
                artefact.language_kind,
                artefact.symbol_fqn,
                artefact.parent_artefact_id,
                artefact.start_line,
                artefact.end_line,
                artefact.signature,
            ],
        )
        .expect("failed to insert production artefact seed row");
    }

    tx.commit().expect("failed to commit seed transaction");
}

pub fn seed_source_file_for_commits(
    db_path: &Path,
    repo_id: &str,
    commits: &[&str],
    path: &str,
    language: &str,
) {
    let mut rows = Vec::new();
    for commit in commits {
        rows.push(ProductionArtefactSeed {
            artefact_id: format!("seed:{commit}:{path}"),
            repo_id: repo_id.to_string(),
            commit_sha: (*commit).to_string(),
            path: path.to_string(),
            language: language.to_string(),
            canonical_kind: "file".to_string(),
            language_kind: Some("source_file".to_string()),
            symbol_fqn: Some(path.to_string()),
            parent_artefact_id: None,
            start_line: 1,
            end_line: 1000,
            signature: None,
        });
    }
    seed_production_artefacts(db_path, &rows);
}
