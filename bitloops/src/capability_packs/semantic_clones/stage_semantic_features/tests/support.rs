use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::features::SemanticSummaryCandidate;
use crate::host::devql::{RelationalStorage, sqlite_exec_path_allow_create};
use tempfile::tempdir;

pub(super) struct StrictNoopSummaryProvider;

impl semantic::SemanticSummaryProvider for StrictNoopSummaryProvider {
    fn cache_key(&self) -> String {
        "strict-noop".to_string()
    }

    fn generate(
        &self,
        _input: &semantic::SemanticFeatureInput,
    ) -> Option<SemanticSummaryCandidate> {
        None
    }

    fn requires_model_output(&self) -> bool {
        true
    }
}

pub(super) struct TestSummaryProvider;

impl semantic::SemanticSummaryProvider for TestSummaryProvider {
    fn cache_key(&self) -> String {
        "provider=ollama:ministral-3:3b".to_string()
    }

    fn generate(
        &self,
        _input: &semantic::SemanticFeatureInput,
    ) -> Option<SemanticSummaryCandidate> {
        Some(SemanticSummaryCandidate {
            summary: "Summarises the symbol.".to_string(),
            confidence: None,
            source_model: Some("ollama:ministral-3:3b".to_string()),
        })
    }

    fn requires_model_output(&self) -> bool {
        true
    }
}

pub(super) async fn sqlite_relational_with_current_projection_schema() -> RelationalStorage {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("semantic-features.sqlite");
    sqlite_exec_path_allow_create(
        &db_path,
        &format!(
            "{}\nCREATE TABLE artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id)
);
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    analysis_mode TEXT NOT NULL,
    effective_content_id TEXT NOT NULL,
    PRIMARY KEY (repo_id, path)
);",
            super::super::storage::semantic_features_sqlite_schema_sql(),
        ),
    )
    .await
    .expect("create sqlite schema");
    std::mem::forget(temp);
    RelationalStorage::local_only(db_path)
}

pub(super) fn sample_semantic_input(
    artefact_id: &str,
    blob_sha: &str,
) -> semantic::SemanticFeatureInput {
    semantic::SemanticFeatureInput {
        artefact_id: artefact_id.to_string(),
        symbol_id: Some(format!("symbol-{artefact_id}")),
        repo_id: "repo-1".to_string(),
        blob_sha: blob_sha.to_string(),
        path: "src/lib.rs".to_string(),
        language: "rust".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: format!("src/lib.rs::{artefact_id}"),
        name: artefact_id.to_string(),
        signature: Some(format!("fn {artefact_id}()")),
        modifiers: vec!["pub".to_string()],
        body: "do_work()".to_string(),
        docstring: Some("Performs work.".to_string()),
        parent_kind: Some("file".to_string()),
        dependency_signals: vec!["calls:worker::do_work".to_string()],
        content_hash: Some(blob_sha.to_string()),
    }
}
