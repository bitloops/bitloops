use super::*;
use crate::adapters::languages::rust::http_facts::extract_rust_http_facts;
use crate::capability_packs::http::schema::http_sqlite_schema_sql;
use crate::host::capability_host::gateways::{
    CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway,
    CapabilityWorkplaneJob, DefaultHostServicesGateway, EmptyGitHistoryGateway,
    LanguageServicesGateway, RelationalGateway,
};
use crate::host::capability_host::{
    CurrentStateConsumerContext, CurrentStateConsumerRequest, ReconcileMode,
};
use crate::host::devql::RelationalStorage;
use crate::host::inference::EmptyInferenceGateway;
use crate::host::language_adapter::LanguageHttpFactEvidence;
use crate::models::{CurrentCanonicalEdgeRecord, ProductionArtefact};
use std::sync::Arc;

#[test]
fn compose_http_bundle_uses_generic_roles_not_ecosystem_symbols() {
    let repo_id = "repo-1";
    let mut primitives = protocol_primitives_for_repo(repo_id);
    primitives.push(HttpPrimitiveFact {
        repo_id: repo_id.to_string(),
        primitive_id: "language.fact.body-strip".to_string(),
        owner: "language.ecosystem".to_string(),
        primitive_type: "LossyTransform".to_string(),
        subject: "Response pipeline replaces a response body before output".to_string(),
        roles: vec![
            HTTP_ROLE_BODY_REPLACEMENT.to_string(),
            HTTP_ROLE_BODY_STRIPPING.to_string(),
        ],
        terms: vec!["body stripping".to_string(), "response".to_string()],
        properties: json!({"sourceCategory": "language_semantics"}),
        confidence_level: "HIGH".to_string(),
        confidence_score: 0.91,
        status: "active".to_string(),
        input_fingerprint: "source-1".to_string(),
        evidence: vec![HttpEvidenceFact {
            evidence_id: "evidence-1".to_string(),
            kind: "code_span".to_string(),
            path: Some("server/response_handler.ext".to_string()),
            artefact_id: Some("artefact-1".to_string()),
            symbol_id: Some("symbol-1".to_string()),
            content_id: Some("content-1".to_string()),
            start_line: Some(10),
            end_line: Some(12),
            start_byte: Some(100),
            end_byte: Some(180),
            dependency_package: None,
            dependency_version: None,
            source_url: None,
            excerpt_hash: None,
            producer: Some("language-pack".to_string()),
            model: None,
            prompt_hash: None,
            properties: json!({}),
        }],
    });

    let bundles = compose_http_bundles(repo_id, &primitives);

    assert_eq!(bundles.len(), 1);
    let bundle = &bundles[0];
    assert_eq!(
        bundle.bundle_id,
        HTTP_BUNDLE_CONTENT_LENGTH_LOSS_BEFORE_WIRE_SERIALISATION
    );
    assert_eq!(
        bundle.risk_kind.as_deref(),
        Some(HTTP_RISK_CONTENT_LENGTH_LOSS)
    );
    assert_eq!(bundle.path.as_deref(), Some("server/response_handler.ext"));
    assert!(
        bundle
            .matched_roles
            .contains(&HTTP_ROLE_BODY_EXACT_SIZE_SIGNAL.to_string())
    );
    let stored = format!("{bundle:?}");
    assert!(!stored.contains("axum"));
}

#[tokio::test]
async fn reconcile_seeds_protocol_facts_composes_bundle_and_refreshes_query_index() -> Result<()> {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let relational = RelationalStorage::local_only(temp.path().to_path_buf());
    relational.exec(http_sqlite_schema_sql()).await?;

    let request = CurrentStateConsumerRequest {
        run_id: Some("run-1".to_string()),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 7,
        reconcile_mode: ReconcileMode::FullReconcile,
        file_upserts: Vec::new(),
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };

    let outcome =
        reconcile_http_current_state(&relational, &request, vec![language_body_stripping_batch()])
            .await?;

    assert_eq!(outcome.protocol_primitives_seeded, 4);
    assert_eq!(outcome.bundles, 1);
    assert!(outcome.query_index_rows >= 6);
    assert_eq!(
            count_rows(
                &relational,
                "SELECT COUNT(*) AS count FROM http_primitives_current WHERE repo_id = 'repo-1' AND owner = 'http';",
            )
            .await?,
            4
        );
    assert_eq!(
            count_rows(
                &relational,
                "SELECT COUNT(*) AS count FROM http_bundles_current WHERE repo_id = 'repo-1' AND risk_kind = 'CONTENT_LENGTH_LOSS';",
            )
            .await?,
            1
        );
    assert_eq!(
            count_rows(
                &relational,
                "SELECT COUNT(*) AS count FROM http_query_index_current WHERE repo_id = 'repo-1' AND bundle_id = 'http.bundle.content_length_loss.before_wire_serialisation';",
            )
            .await?,
            1
        );
    Ok(())
}

#[tokio::test]
async fn consumer_reads_rust_body_replacement_fact_from_current_state_and_composes_bundle()
-> Result<()> {
    let temp_db = tempfile::NamedTempFile::new().expect("temp db");
    let storage = Arc::new(RelationalStorage::local_only(temp_db.path().to_path_buf()));
    storage.exec(http_sqlite_schema_sql()).await?;
    let repo = tempfile::tempdir().expect("temp repo");
    let path = "axum/src/routing/route.rs";
    let absolute_path = repo.path().join(path);
    std::fs::create_dir_all(absolute_path.parent().expect("parent"))?;
    let content = r#"
use http::Response;

impl<B> Future for RouteFuture<B> {
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Response<BoxBody>> {
        if self.method == Method::HEAD {
            let res = ready!(self.inner.poll(cx));
            Poll::Ready(res.map(|_| boxed(Empty::new())))
        } else {
            ready!(self.inner.poll(cx))
        }
    }
}
"#;
    std::fs::write(&absolute_path, content)?;
    let function_start = content.find("fn poll").expect("poll function") as i64;
    let function_end = content.len() as i64;

    let request = CurrentStateConsumerRequest {
        run_id: Some("run-1".to_string()),
        repo_id: "repo-1".to_string(),
        repo_root: repo.path().to_path_buf(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 7,
        reconcile_mode: ReconcileMode::FullReconcile,
        file_upserts: Vec::new(),
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };
    let context = CurrentStateConsumerContext {
            config_root: json!({}),
            storage: storage.clone(),
            relational: Arc::new(TestRelationalGateway {
                files: vec![CurrentCanonicalFileRecord {
                    repo_id: "repo-1".to_string(),
                    path: path.to_string(),
                    analysis_mode: "code".to_string(),
                    file_role: "source".to_string(),
                    language: "rust".to_string(),
                    resolved_language: "rust".to_string(),
                    effective_content_id: "content-1".to_string(),
                    parser_version: "tree-sitter-rust@1".to_string(),
                    extractor_version: "rust-language-pack@1".to_string(),
                    exists_in_head: true,
                    exists_in_index: true,
                    exists_in_worktree: true,
                }],
                artefacts: vec![CurrentCanonicalArtefactRecord {
                    repo_id: "repo-1".to_string(),
                    path: path.to_string(),
                    content_id: "content-1".to_string(),
                    symbol_id: "symbol-route-future-poll".to_string(),
                    artefact_id: "artefact-route-future-poll".to_string(),
                    language: "rust".to_string(),
                    extraction_fingerprint: "extract-1".to_string(),
                    canonical_kind: Some("method".to_string()),
                    language_kind: Some("method_declaration".to_string()),
                    symbol_fqn: Some("axum/src/routing/route.rs::impl@1::poll".to_string()),
                    parent_symbol_id: None,
                    parent_artefact_id: None,
                    start_line: 5,
                    end_line: 12,
                    start_byte: function_start,
                    end_byte: function_end,
                    signature: Some(
                        "fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Response<BoxBody>>"
                            .to_string(),
                    ),
                    modifiers: "[]".to_string(),
                    docstring: None,
                }],
            }),
            language_services: Arc::new(RustHttpFactLanguageServices),
            git_history: Arc::new(EmptyGitHistoryGateway),
            inference: Arc::new(EmptyInferenceGateway),
            host_services: Arc::new(DefaultHostServicesGateway::new("repo-1")),
            workplane: Arc::new(NoopCapabilityWorkplaneGateway),
            test_harness: None,
            init_session_id: None,
            parent_pid: None,
        };

    let consumer = HttpCurrentStateConsumer;
    let result = consumer.reconcile(&request, &context).await?;

    assert_eq!(result.applied_to_generation_seq, 7);
    let rows = storage
        .query_rows(
            "SELECT p.subject, p.roles_json, p.properties_json, e.path, e.artefact_id,
                        e.symbol_id, e.start_line, e.end_line, e.start_byte, e.end_byte
                 FROM http_primitives_current p
                 JOIN http_primitive_evidence_current e
                   ON p.repo_id = e.repo_id AND p.primitive_id = e.primitive_id
                 WHERE p.repo_id = 'repo-1' AND p.owner = 'rust-language-pack'
                 ORDER BY p.primitive_id ASC;",
        )
        .await?;

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    let subject = row
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        subject.contains("axum/src/routing/route.rs::RouteFuture::poll"),
        "unexpected HTTP fact subject: {subject}"
    );
    assert!(
        row.get("roles_json")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains(HTTP_ROLE_BODY_REPLACEMENT)
    );
    assert!(
        row.get("roles_json")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains(HTTP_ROLE_BODY_STRIPPING)
    );
    assert!(
        row.get("properties_json")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("body_exact_size_signal")
    );
    assert!(
        row.get("properties_json")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("body_size_hint")
    );
    assert_eq!(row.get("path").and_then(Value::as_str), Some(path));
    assert_eq!(
        row.get("artefact_id").and_then(Value::as_str),
        Some("artefact-route-future-poll")
    );
    assert_eq!(
        row.get("symbol_id").and_then(Value::as_str),
        Some("symbol-route-future-poll")
    );
    assert!(row.get("start_line").and_then(Value::as_i64).is_some());
    assert!(row.get("end_line").and_then(Value::as_i64).is_some());
    assert!(row.get("start_byte").and_then(Value::as_i64).is_some());
    assert!(row.get("end_byte").and_then(Value::as_i64).is_some());
    assert_eq!(
        count_rows(
            &storage,
            "SELECT COUNT(*) AS count
                 FROM http_bundles_current
                 WHERE repo_id = 'repo-1'
                   AND bundle_id = 'http.bundle.content_length_loss.before_wire_serialisation'
                   AND risk_kind = 'CONTENT_LENGTH_LOSS';",
        )
        .await?,
        1
    );
    Ok(())
}

fn language_body_stripping_batch() -> UpstreamHttpFactBatch {
    UpstreamHttpFactBatch {
        owner: "rust-language-pack".to_string(),
        path: "server/response_handler.rs".to_string(),
        facts: vec![LanguageHttpFact {
            stable_key: "rust.http.lossy_body_transform:server/response_handler.rs:symbol-1:10"
                .to_string(),
            primitive_type: "LossyTransform".to_string(),
            subject: "Response pipeline replaces a response body before output".to_string(),
            roles: vec![
                HTTP_ROLE_BODY_REPLACEMENT.to_string(),
                HTTP_ROLE_BODY_STRIPPING.to_string(),
            ],
            terms: vec!["body stripping".to_string(), "response".to_string()],
            properties: json!({"sourceCategory": "language_ecosystem_http_fact"}),
            confidence_level: "HIGH".to_string(),
            confidence_score: 0.91,
            evidence: vec![LanguageHttpFactEvidence {
                path: "server/response_handler.rs".to_string(),
                artefact_id: Some("artefact-1".to_string()),
                symbol_id: Some("symbol-1".to_string()),
                content_id: "content-1".to_string(),
                start_line: Some(10),
                end_line: Some(12),
                start_byte: Some(100),
                end_byte: Some(180),
                properties: json!({}),
            }],
        }],
    }
}

async fn count_rows(relational: &RelationalStorage, sql: &str) -> Result<i64> {
    let rows = relational.query_rows(sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or_default())
}

#[derive(Clone)]
struct TestRelationalGateway {
    files: Vec<CurrentCanonicalFileRecord>,
    artefacts: Vec<CurrentCanonicalArtefactRecord>,
}

impl RelationalGateway for TestRelationalGateway {
    fn resolve_checkpoint_id(&self, _repo_id: &str, checkpoint_ref: &str) -> Result<String> {
        Ok(checkpoint_ref.to_string())
    }

    fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
        Ok(false)
    }

    fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
        Ok("repo-1".to_string())
    }

    fn load_current_canonical_files(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalFileRecord>> {
        Ok(self.files.clone())
    }

    fn load_current_canonical_artefacts(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
        Ok(self.artefacts.clone())
    }

    fn load_current_canonical_edges(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalEdgeRecord>> {
        Ok(Vec::new())
    }

    fn load_current_production_artefacts(&self, _repo_id: &str) -> Result<Vec<ProductionArtefact>> {
        Ok(Vec::new())
    }

    fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        Ok(Vec::new())
    }

    fn load_artefacts_for_file_lines(
        &self,
        _commit_sha: &str,
        _file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        Ok(Vec::new())
    }
}

struct RustHttpFactLanguageServices;

impl LanguageServicesGateway for RustHttpFactLanguageServices {
    fn http_facts_for_file(
        &self,
        file: &LanguageHttpFactFile,
        content: &str,
        artefacts: &[LanguageHttpFactArtefact],
    ) -> Option<(String, Vec<LanguageHttpFact>)> {
        Some((
            "rust-language-pack".to_string(),
            extract_rust_http_facts(file, content, artefacts),
        ))
    }
}

struct NoopCapabilityWorkplaneGateway;

impl CapabilityWorkplaneGateway for NoopCapabilityWorkplaneGateway {
    fn enqueue_jobs(
        &self,
        _jobs: Vec<CapabilityWorkplaneJob>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        Ok(CapabilityWorkplaneEnqueueResult::default())
    }

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
        Ok(BTreeMap::new())
    }
}
