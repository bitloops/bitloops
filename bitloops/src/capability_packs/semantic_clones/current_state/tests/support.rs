use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use tempfile::tempdir;

use crate::host::capability_host::gateways::{
    CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway,
    CapabilityWorkplaneJob, DefaultHostServicesGateway, EmptyLanguageServicesGateway,
    HostServicesGateway, RelationalGateway,
};
use crate::host::capability_host::{
    ChangedArtefact, ChangedFile, CurrentStateConsumerContext, CurrentStateConsumerRequest,
    CurrentStateConsumerResult, ReconcileMode, RemovedArtefact, RemovedFile,
};
use crate::models::ProductionArtefact;

use super::super::super::types::SEMANTIC_CLONES_CAPABILITY_ID;

#[derive(Clone, Default)]
pub(super) struct CapturingWorkplaneGateway {
    jobs: Arc<Mutex<Vec<CapabilityWorkplaneJob>>>,
    status: BTreeMap<String, CapabilityMailboxStatus>,
}

impl CapturingWorkplaneGateway {
    pub(super) fn with_status(status: BTreeMap<String, CapabilityMailboxStatus>) -> Self {
        Self {
            jobs: Arc::new(Mutex::new(Vec::new())),
            status,
        }
    }

    pub(super) fn jobs(&self) -> Vec<CapabilityWorkplaneJob> {
        self.jobs.lock().expect("lock captured jobs").clone()
    }
}

impl CapabilityWorkplaneGateway for CapturingWorkplaneGateway {
    fn enqueue_jobs(
        &self,
        jobs: Vec<CapabilityWorkplaneJob>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        let inserted_jobs = jobs.len() as u64;
        self.jobs.lock().expect("lock captured jobs").extend(jobs);
        Ok(CapabilityWorkplaneEnqueueResult {
            inserted_jobs,
            updated_jobs: 0,
        })
    }

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
        Ok(self.status.clone())
    }
}

pub(super) struct NoopRelationalGateway;

impl RelationalGateway for NoopRelationalGateway {
    fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
        bail!("resolve_checkpoint_id is not used in semantic_clones current-state tests")
    }

    fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
        bail!("artefact_exists is not used in semantic_clones current-state tests")
    }

    fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
        bail!("load_repo_id_for_commit is not used in semantic_clones current-state tests")
    }

    fn load_current_production_artefacts(&self, _repo_id: &str) -> Result<Vec<ProductionArtefact>> {
        bail!(
            "load_current_production_artefacts is not used in semantic_clones current-state tests"
        )
    }

    fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        bail!("load_production_artefacts is not used in semantic_clones current-state tests")
    }

    fn load_artefacts_for_file_lines(
        &self,
        _commit_sha: &str,
        _file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        bail!("load_artefacts_for_file_lines is not used in semantic_clones current-state tests")
    }
}

pub(super) struct TestContext {
    pub(super) _tempdir: tempfile::TempDir,
    pub(super) storage: Arc<crate::host::devql::RelationalStorage>,
    pub(super) workplane: CapturingWorkplaneGateway,
    pub(super) request: CurrentStateConsumerRequest,
    pub(super) context: CurrentStateConsumerContext,
    pub(super) sqlite_path: PathBuf,
}

pub(super) async fn test_context(
    config_root: Value,
    workplane: CapturingWorkplaneGateway,
    request: CurrentStateConsumerRequest,
) -> Result<TestContext> {
    let tempdir = tempdir().expect("temp dir");
    let sqlite_path = tempdir.path().join("semantic-current-state.sqlite");
    let storage = Arc::new(crate::host::devql::RelationalStorage::local_only(
        sqlite_path.clone(),
    ));
    crate::host::devql::sqlite_exec_path_allow_create(
        &sqlite_path,
        crate::host::devql::devql_schema_sql_sqlite(),
    )
    .await?;
    crate::host::devql::sqlite_exec_path_allow_create(
        &sqlite_path,
        crate::host::devql::sync::schema::sync_schema_sql(),
    )
    .await?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_features_schema(&sqlite_path)
        .await?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_embeddings_schema(&sqlite_path)
        .await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
        storage.as_ref(),
        &request.repo_id,
    )
    .await?;

    let context = CurrentStateConsumerContext {
        config_root,
        storage: Arc::clone(&storage),
        relational: Arc::new(NoopRelationalGateway),
        language_services: Arc::new(EmptyLanguageServicesGateway),
        git_history: Arc::new(crate::host::capability_host::gateways::EmptyGitHistoryGateway),
        host_services: Arc::new(DefaultHostServicesGateway::new(request.repo_id.clone()))
            as Arc<dyn HostServicesGateway>,
        workplane: Arc::new(workplane.clone()),
        test_harness: None,
        init_session_id: None,
    };

    Ok(TestContext {
        _tempdir: tempdir,
        storage,
        workplane,
        request,
        context,
        sqlite_path,
    })
}

pub(super) fn request(
    repo_root: &Path,
    repo_id: &str,
    reconcile_mode: ReconcileMode,
    file_upserts: Vec<ChangedFile>,
    file_removals: Vec<RemovedFile>,
    artefact_upserts: Vec<ChangedArtefact>,
    artefact_removals: Vec<RemovedArtefact>,
) -> CurrentStateConsumerRequest {
    CurrentStateConsumerRequest {
        run_id: None,
        repo_id: repo_id.to_string(),
        repo_root: repo_root.to_path_buf(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 1,
        reconcile_mode,
        file_upserts,
        file_removals,
        affected_paths: Vec::new(),
        artefact_upserts,
        artefact_removals,
    }
}

pub(super) fn config_root(
    summary_generation: Option<&str>,
    code_embeddings: Option<&str>,
    summary_embeddings: Option<&str>,
) -> Value {
    let mut inference = Map::new();
    if let Some(summary_generation) = summary_generation {
        inference.insert(
            "summary_generation".to_string(),
            Value::String(summary_generation.to_string()),
        );
    }
    if let Some(code_embeddings) = code_embeddings {
        inference.insert(
            "code_embeddings".to_string(),
            Value::String(code_embeddings.to_string()),
        );
    }
    if let Some(summary_embeddings) = summary_embeddings {
        inference.insert(
            "summary_embeddings".to_string(),
            Value::String(summary_embeddings.to_string()),
        );
    }

    json!({
        SEMANTIC_CLONES_CAPABILITY_ID: {
            "summary_mode": if summary_generation.is_some() { "auto" } else { "off" },
            "embedding_mode": if code_embeddings.is_some() || summary_embeddings.is_some() {
                "semantic_aware_once"
            } else {
                "off"
            },
            "ann_neighbors": 5,
            "summary_workers": 1,
            "embedding_workers": 1,
            "clone_rebuild_workers": 1,
            "enrichment_workers": 1,
            "inference": Value::Object(inference),
        }
    })
}

pub(super) async fn seed_current_rows(
    storage: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    sqlite_path: &Path,
    path: &str,
    suffix: &str,
) {
    crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
        storage, repo_id,
    )
    .await
    .expect("ensure current clone edge schema");
    let conn = Connection::open(sqlite_path).expect("open sqlite");
    let symbol_id = format!("symbol-{suffix}");
    let artefact_id = format!("artefact-{suffix}");
    let content_id = format!("content-{suffix}");
    let semantic_hash = format!("semantic-hash-{suffix}");

    conn.execute(
        "INSERT INTO symbol_semantics_current (
            artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
            template_summary, summary, confidence
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            artefact_id,
            repo_id,
            path,
            content_id,
            symbol_id,
            semantic_hash,
            format!("summary {suffix}"),
            format!("summary {suffix}"),
            0.9_f64,
        ],
    )
    .expect("insert current semantics");
    conn.execute(
        "INSERT INTO symbol_features_current (
            artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
            normalized_name, normalized_signature, modifiers, identifier_tokens,
            normalized_body_tokens, parent_kind, context_tokens
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '[]', ?9, ?10, ?11, ?12)",
        rusqlite::params![
            artefact_id,
            repo_id,
            path,
            content_id,
            symbol_id,
            semantic_hash,
            format!("fn_{suffix}"),
            format!("fn {suffix}()"),
            r#"["fn"]"#,
            r#"["return"]"#,
            "module",
            r#"["ctx"]"#,
        ],
    )
    .expect("insert current features");
    for representation_kind in ["code", "summary"] {
        conn.execute(
            "INSERT INTO symbol_embeddings_current (
                artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                artefact_id,
                repo_id,
                path,
                content_id,
                symbol_id,
                representation_kind,
                format!("setup-{representation_kind}"),
                "local",
                format!("model-{representation_kind}"),
                3_i64,
                format!("embed-{representation_kind}-{suffix}"),
                "[0.1,0.2,0.3]",
            ],
        )
        .expect("insert current embedding");
    }
    conn.execute(
        "INSERT INTO symbol_clone_edges_current (
            repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
            target_artefact_id, relation_kind, score, semantic_score, lexical_score,
            structural_score, clone_input_hash, explanation_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            repo_id,
            format!("source-{suffix}"),
            format!("source-artefact-{suffix}"),
            symbol_id,
            artefact_id,
            "similar_implementation",
            0.8_f64,
            0.8_f64,
            0.5_f64,
            0.4_f64,
            format!("clone-{suffix}"),
            "{}",
        ],
    )
    .expect("insert current clone edge");
}

pub(super) async fn seed_current_artefact_ids(sqlite_path: &Path, repo_id: &str, count: usize) {
    let conn = Connection::open(sqlite_path).expect("open sqlite for current artefacts");
    seed_repository_row(&conn, repo_id);
    for index in 0..count {
        let path = format!("src/file-{index}.ts");
        let content_id = format!("content-{index}");
        seed_current_file_state_row(&conn, repo_id, &path, "code", "typescript", &content_id);
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript',
                'function', 'function', ?6, 1, 3, 0, 24, '[]', datetime('now')
            )",
            rusqlite::params![
                repo_id,
                path,
                content_id,
                format!("symbol-{index}"),
                format!("artefact-{index:03}"),
                format!("src/file-{index}.ts::fn_{index}"),
            ],
        )
        .expect("insert current artefact");
    }
}

pub(super) fn seed_current_file_state(
    sqlite_path: &Path,
    repo_id: &str,
    path: &str,
    analysis_mode: &str,
    language: &str,
) {
    let conn = Connection::open(sqlite_path).expect("open sqlite for current file state");
    seed_repository_row(&conn, repo_id);
    seed_current_file_state_row(
        &conn,
        repo_id,
        path,
        analysis_mode,
        language,
        &format!("content-{path}"),
    );
}

pub(super) fn seed_current_artefact(
    sqlite_path: &Path,
    repo_id: &str,
    path: &str,
    artefact_id: &str,
    canonical_kind: &str,
) {
    let conn = Connection::open(sqlite_path).expect("open sqlite for current artefact");
    seed_repository_row(&conn, repo_id);
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line,
            start_byte, end_byte, modifiers, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, 'rust',
            ?6, ?6, ?7, 1, 3, 0, 24, '[]', datetime('now')
        )",
        rusqlite::params![
            repo_id,
            path,
            format!("content-{path}"),
            format!("symbol-{artefact_id}"),
            artefact_id,
            canonical_kind,
            format!("{path}::{artefact_id}"),
        ],
    )
    .expect("insert current artefact");
}

fn seed_repository_row(conn: &Connection, repo_id: &str) {
    conn.execute(
        "INSERT OR IGNORE INTO repositories (
            repo_id, provider, organization, name, default_branch
        ) VALUES (?1, 'test', 'test', 'test', 'main')",
        rusqlite::params![repo_id],
    )
    .expect("insert repository row");
}

fn seed_current_file_state_row(
    conn: &Connection,
    repo_id: &str,
    path: &str,
    analysis_mode: &str,
    language: &str,
    content_id: &str,
) {
    conn.execute(
        "INSERT INTO current_file_state (
            repo_id, path, analysis_mode, language, effective_content_id,
            effective_source, parser_version, extractor_version, exists_in_head,
            exists_in_index, exists_in_worktree, last_synced_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            'head', 'test-parser', 'test-extractor', 1,
            1, 1, datetime('now')
        )
        ON CONFLICT (repo_id, path) DO UPDATE SET
            analysis_mode = excluded.analysis_mode,
            language = excluded.language,
            effective_content_id = excluded.effective_content_id",
        rusqlite::params![repo_id, path, analysis_mode, language, content_id],
    )
    .expect("insert current file state");
}

pub(super) fn count_rows(sqlite_path: &Path, sql: &str, repo_id: &str, path: Option<&str>) -> i64 {
    let conn = Connection::open(sqlite_path).expect("open sqlite for count");
    match path {
        Some(path) => conn
            .query_row(sql, rusqlite::params![repo_id, path], |row| row.get(0))
            .expect("count rows by path"),
        None => conn
            .query_row(sql, rusqlite::params![repo_id], |row| row.get(0))
            .expect("count rows"),
    }
}

pub(super) fn metrics_map(result: &CurrentStateConsumerResult) -> BTreeMap<String, Value> {
    result
        .metrics
        .clone()
        .and_then(|value| value.as_object().cloned())
        .expect("metrics object")
        .into_iter()
        .collect()
}
