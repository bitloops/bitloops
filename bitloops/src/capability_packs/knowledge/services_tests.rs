use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::adapters::connectors::{
    ConnectorContext, ConnectorRegistry, ExternalKnowledgeRecord, KnowledgeConnectorAdapter,
};
use crate::capability_packs::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, KnowledgeDocumentRepository,
    KnowledgeRelationalRepository, SqliteKnowledgeRelationalRepository,
};
use crate::capability_packs::knowledge::url::parse_knowledge_url;
use crate::config::{
    BlobStorageConfig, EventsBackendConfig, ProviderConfig, RelationalBackendConfig,
    StoreBackendConfig,
};
use crate::host::capability_host::config_view::CapabilityConfigView;
use crate::host::capability_host::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, CapabilityMailboxStatus,
    CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway, CapabilityWorkplaneJob,
    ProvenanceBuilder, RelationalGateway, SqliteRelationalGateway,
};
use crate::host::capability_host::{
    CapabilityExecutionContext, CapabilityIngestContext, KnowledgeExecutionContext,
    KnowledgeIngestContext, StageRequest,
};
use crate::host::devql::RepoIdentity;
use crate::storage::SqliteConnectionPool;
use crate::test_support::git_fixtures::{git_ok, init_test_repo};

use super::*;

struct StubAdapter {
    records: Arc<Mutex<VecDeque<ExternalKnowledgeRecord>>>,
}

impl KnowledgeConnectorAdapter for StubAdapter {
    fn can_handle(&self, parsed: &super::super::types::ParsedKnowledgeUrl) -> bool {
        matches!(
            parsed.provider,
            super::super::types::KnowledgeProvider::Github
        )
    }

    fn fetch<'a>(
        &'a self,
        _parsed: &'a super::super::types::ParsedKnowledgeUrl,
        _ctx: &'a dyn ConnectorContext,
    ) -> crate::adapters::connectors::types::BoxFuture<'a, Result<ExternalKnowledgeRecord>> {
        Box::pin(async move {
            let mut records = self.records.lock().expect("stub records mutex");
            let Some(record) = records.pop_front() else {
                bail!("no stub connector record available");
            };
            Ok(record)
        })
    }
}

struct StubConnectorRegistry {
    provider_config: ProviderConfig,
    adapter: StubAdapter,
}

impl ConnectorContext for StubConnectorRegistry {
    fn provider_config(&self) -> &ProviderConfig {
        &self.provider_config
    }
}

impl ConnectorRegistry for StubConnectorRegistry {
    fn knowledge_adapter_for(
        &self,
        parsed: &super::super::types::ParsedKnowledgeUrl,
    ) -> Result<&dyn KnowledgeConnectorAdapter> {
        if self.adapter.can_handle(parsed) {
            Ok(&self.adapter)
        } else {
            bail!("no stub connector for `{}`", parsed.canonical_external_id)
        }
    }
}

struct TestGraphGateway;

impl CanonicalGraphGateway for TestGraphGateway {}

struct TestProvenanceBuilder;

impl ProvenanceBuilder for TestProvenanceBuilder {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
        json!({
            "capability": capability_id,
            "operation": operation,
            "details": details,
        })
    }
}

struct CapturingKnowledgeWorkplane {
    jobs: Mutex<Vec<CapabilityWorkplaneJob>>,
}

impl CapturingKnowledgeWorkplane {
    fn new() -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
        }
    }

    fn jobs(&self) -> Vec<CapabilityWorkplaneJob> {
        self.jobs.lock().expect("jobs").clone()
    }
}

impl CapabilityWorkplaneGateway for CapturingKnowledgeWorkplane {
    fn enqueue_jobs(
        &self,
        jobs: Vec<CapabilityWorkplaneJob>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        let inserted_jobs = jobs.len() as u64;
        self.jobs.lock().expect("jobs").extend(jobs);
        Ok(CapabilityWorkplaneEnqueueResult {
            inserted_jobs,
            updated_jobs: 0,
        })
    }

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
        Ok(BTreeMap::new())
    }
}

struct TestRuntimeContext {
    repo_root: PathBuf,
    repo: RepoIdentity,
    config_root: Value,
    relational: SqliteRelationalGateway,
    knowledge_relational: SqliteKnowledgeRelationalRepository,
    documents: DuckdbKnowledgeDocumentStore,
    blobs: BlobKnowledgePayloadStore,
    connectors: StubConnectorRegistry,
    provenance: TestProvenanceBuilder,
    graph: TestGraphGateway,
    workplane: CapturingKnowledgeWorkplane,
    invoking_capability_id: Option<&'static str>,
    invoking_ingester_id: Option<&'static str>,
}

impl CapabilityExecutionContext for TestRuntimeContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    fn graph(&self) -> &dyn CanonicalGraphGateway {
        &self.graph
    }

    fn host_relational(&self) -> &dyn RelationalGateway {
        &self.relational
    }
}

impl CapabilityIngestContext for TestRuntimeContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            self.config_root.clone(),
        ))
    }

    fn blob_payloads(&self) -> &dyn BlobPayloadGateway {
        &self.blobs
    }

    fn connectors(&self) -> &dyn ConnectorRegistry {
        &self.connectors
    }

    fn connector_context(&self) -> &dyn ConnectorContext {
        &self.connectors
    }

    fn provenance(&self) -> &dyn ProvenanceBuilder {
        &self.provenance
    }

    fn host_relational(&self) -> &dyn RelationalGateway {
        &self.relational
    }

    fn invoking_capability_id(&self) -> Option<&str> {
        self.invoking_capability_id
    }

    fn invoking_ingester_id(&self) -> Option<&str> {
        self.invoking_ingester_id
    }

    fn workplane(&self) -> Option<&dyn CapabilityWorkplaneGateway> {
        Some(&self.workplane)
    }
}

impl KnowledgeExecutionContext for TestRuntimeContext {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        &self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        &self.documents
    }
}

impl KnowledgeIngestContext for TestRuntimeContext {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        &self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        &self.documents
    }
}

fn test_backends(temp: &TempDir) -> StoreBackendConfig {
    StoreBackendConfig {
        relational: RelationalBackendConfig {
            sqlite_path: Some(
                temp.path()
                    .join("knowledge-relational.sqlite")
                    .to_string_lossy()
                    .to_string(),
            ),
            postgres_dsn: None,
        },
        events: EventsBackendConfig {
            duckdb_path: Some(
                temp.path()
                    .join("knowledge-documents.duckdb")
                    .to_string_lossy()
                    .to_string(),
            ),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
        blobs: BlobStorageConfig {
            local_path: Some(
                temp.path()
                    .join("knowledge-blobs")
                    .to_string_lossy()
                    .to_string(),
            ),
            s3_bucket: None,
            s3_region: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            gcs_bucket: None,
            gcs_credentials_path: None,
        },
    }
}

fn test_repo_identity(repo_root: &Path) -> RepoIdentity {
    let identity = repo_root.to_string_lossy().to_string();
    RepoIdentity {
        provider: "local".to_string(),
        organization: "bitloops".to_string(),
        name: "knowledge-tests".to_string(),
        identity: identity.clone(),
        repo_id: crate::host::devql::deterministic_uuid(&format!("repo://{identity}")),
    }
}

fn build_record(
    parsed: &super::super::types::ParsedKnowledgeUrl,
    title: &str,
    body: &str,
    updated_at: &str,
) -> ExternalKnowledgeRecord {
    ExternalKnowledgeRecord {
        provider: "github".to_string(),
        source_kind: parsed.source_kind.as_str().to_string(),
        canonical_external_id: parsed.canonical_external_id.clone(),
        canonical_url: parsed.canonical_url.clone(),
        title: title.to_string(),
        state: Some("open".to_string()),
        author: Some("spiros".to_string()),
        updated_at: Some(updated_at.to_string()),
        body_preview: Some(body.to_string()),
        normalized_fields: json!({
            "title": title,
            "updated_at": updated_at,
        }),
        payload: super::super::types::KnowledgePayloadData {
            raw_payload: json!({ "title": title, "body": body, "updated_at": updated_at }),
            body_text: Some(body.to_string()),
            body_html: None,
            body_adf: None,
            discussion: None,
        },
    }
}

fn build_context(
    temp: &TempDir,
    records: Vec<ExternalKnowledgeRecord>,
) -> Result<TestRuntimeContext> {
    build_context_with_dispatch(temp, records, None, None)
}

fn build_context_with_dispatch(
    temp: &TempDir,
    records: Vec<ExternalKnowledgeRecord>,
    invoking_capability_id: Option<&'static str>,
    invoking_ingester_id: Option<&'static str>,
) -> Result<TestRuntimeContext> {
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root)?;
    init_test_repo(&repo_root, "main", "Bitloops Bot", "bot@bitloops.dev");
    fs::write(repo_root.join("README.md"), "# knowledge\n")?;
    git_ok(repo_root.as_path(), &["add", "."]);
    git_ok(repo_root.as_path(), &["commit", "-m", "initial commit"]);

    let repo = test_repo_identity(repo_root.as_path());
    let backends = test_backends(temp);
    let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
    let sqlite_pool = SqliteConnectionPool::connect(sqlite_path)?;
    let relational = SqliteRelationalGateway::new(sqlite_pool.clone());
    let knowledge_relational = SqliteKnowledgeRelationalRepository::new(sqlite_pool);
    knowledge_relational.initialise_schema()?;
    let documents = DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
    documents.initialise_schema()?;
    let blobs = BlobKnowledgePayloadStore::from_backend_config(repo_root.as_path(), &backends)?;
    let connectors = StubConnectorRegistry {
        provider_config: ProviderConfig::default(),
        adapter: StubAdapter {
            records: Arc::new(Mutex::new(VecDeque::from(records))),
        },
    };

    Ok(TestRuntimeContext {
        repo_root,
        repo,
        config_root: json!({
            "knowledge": {
                "providers": {
                    "github": { "configured": true }
                }
            }
        }),
        relational,
        knowledge_relational,
        documents,
        blobs,
        connectors,
        provenance: TestProvenanceBuilder,
        graph: TestGraphGateway,
        workplane: CapturingKnowledgeWorkplane::new(),
        invoking_capability_id,
        invoking_ingester_id,
    })
}

#[tokio::test]
async fn services_flow_ingests_reuses_refreshes_and_associates() -> Result<()> {
    let temp = TempDir::new()?;
    let url = "https://github.com/bitloops/bitloops/issues/42";
    let parsed = parse_knowledge_url(url)?;

    let mut ctx = build_context(
        &temp,
        vec![
            build_record(&parsed, "Issue 42", "first body", "2026-03-18T10:00:00Z"),
            build_record(&parsed, "Issue 42", "first body", "2026-03-18T10:00:00Z"),
            build_record(&parsed, "Issue 42", "second body", "2026-03-19T10:00:00Z"),
        ],
    )?;
    let services = KnowledgeServices::new();

    let first = services
        .ingestion
        .ingest_source(
            IngestKnowledgeRequest {
                url: url.to_string(),
            },
            &mut ctx,
        )
        .await?;
    assert_eq!(first.item_status, KnowledgeItemStatus::Created);
    assert_eq!(first.version_status, KnowledgeVersionStatus::Created);

    let second = services
        .ingestion
        .ingest_source(
            IngestKnowledgeRequest {
                url: url.to_string(),
            },
            &mut ctx,
        )
        .await?;
    assert_eq!(second.item_status, KnowledgeItemStatus::Reused);
    assert_eq!(second.version_status, KnowledgeVersionStatus::Reused);
    assert_eq!(
        second.knowledge_item_version_id,
        first.knowledge_item_version_id
    );

    let head_sha = git_ok(ctx.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let association = services
        .relations
        .associate_by_refs(
            &mut ctx,
            &format!("knowledge:{}", first.knowledge_item_id),
            &format!("commit:{head_sha}"),
        )
        .await?;
    assert_eq!(association.target_type, "commit");
    assert_eq!(association.target_id, head_sha);

    let path_association = services
        .relations
        .associate_by_refs(
            &mut ctx,
            &format!("knowledge:{}", first.knowledge_item_id),
            "path:axum-macros/src/from_request.rs",
        )
        .await?;
    assert_eq!(path_association.target_type, "path");
    assert_eq!(
        path_association.target_id,
        "axum-macros/src/from_request.rs"
    );

    let symbol_association = services
        .relations
        .associate_by_refs(
            &mut ctx,
            &format!("knowledge:{}", first.knowledge_item_id),
            "symbol_fqn:crate::from_request::extract_fields",
        )
        .await?;
    assert_eq!(symbol_association.target_type, "symbol_fqn");
    assert_eq!(
        symbol_association.target_id,
        "crate::from_request::extract_fields"
    );
    assert!(
        ctx.workplane
            .jobs()
            .iter()
            .any(
                |job| job.target_capability_id.as_deref() == Some("context_guidance")
                    && job.mailbox_name == "context_guidance.knowledge_distillation"
            )
    );

    let listed_before_refresh = services.retrieval.list_versions(
        ListVersionsRequest {
            knowledge_ref: format!("knowledge:{}", first.knowledge_item_id),
        },
        &mut ctx,
    );
    let listed_before_refresh = listed_before_refresh.await?;
    assert_eq!(listed_before_refresh.versions.len(), 1);

    let refreshed = services
        .ingestion
        .refresh_source(
            RefreshSourceRequest {
                knowledge_ref: format!("knowledge:{}", first.knowledge_item_id),
            },
            &mut ctx,
        )
        .await?;
    assert_eq!(refreshed.knowledge_item_id, first.knowledge_item_id);
    assert!(refreshed.content_changed);
    assert!(refreshed.new_version_created);
    assert_ne!(
        refreshed.latest_document_version_id,
        first.knowledge_item_version_id
    );

    let listed_after_refresh = services
        .retrieval
        .list_versions(
            ListVersionsRequest {
                knowledge_ref: format!("knowledge:{}", first.knowledge_item_id),
            },
            &mut ctx,
        )
        .await?;
    assert_eq!(listed_after_refresh.versions.len(), 2);

    let repo = ctx.repo.clone();
    let stage_rows = services.retrieval.list_repository_knowledge(
        &repo,
        &StageRequest::new(json!({ "limit": 10 })),
        &mut ctx,
    )?;
    assert_eq!(stage_rows.len(), 1);
    assert_eq!(
        stage_rows[0]
            .get("knowledge_item_id")
            .and_then(Value::as_str),
        Some(first.knowledge_item_id.as_str())
    );

    let found = ctx
        .documents
        .find_knowledge_item_version(&refreshed.latest_document_version_id)?
        .ok_or_else(|| anyhow!("missing refreshed version row"))?;
    assert_eq!(found.title, "Issue 42");
    assert_eq!(found.body_preview.as_deref(), Some("second body"));
    assert!(ctx.blobs.payload_exists(&found.storage_path)?);

    Ok(())
}

#[tokio::test]
async fn ingestion_provenance_includes_dispatch_metadata_when_present() -> Result<()> {
    use rusqlite::Connection;

    let temp = TempDir::new()?;
    let url = "https://github.com/bitloops/bitloops/issues/42";
    let parsed = parse_knowledge_url(url)?;
    let mut ctx = build_context_with_dispatch(
        &temp,
        vec![build_record(
            &parsed,
            "Issue 42",
            "body",
            "2026-03-18T10:00:00Z",
        )],
        Some("knowledge"),
        Some("knowledge.add"),
    )?;

    KnowledgeServices::new()
        .ingestion
        .ingest_source(
            IngestKnowledgeRequest {
                url: url.to_string(),
            },
            &mut ctx,
        )
        .await?;

    let sqlite_path = test_backends(&temp).relational.resolve_sqlite_db_path()?;
    let conn = Connection::open(&sqlite_path)?;
    let prov: String = conn.query_row(
        "SELECT provenance_json FROM knowledge_sources LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let value: Value = serde_json::from_str(&prov)?;
    assert_eq!(
        value["invoking_capability_id"],
        json!("knowledge"),
        "{value}"
    );
    assert_eq!(value["ingester_id"], json!("knowledge.add"), "{value}");
    Ok(())
}

#[tokio::test]
async fn associating_knowledge_to_path_enqueues_context_guidance_work() -> Result<()> {
    let temp = TempDir::new()?;
    let url = "https://github.com/bitloops/bitloops/issues/42";
    let parsed = parse_knowledge_url(url)?;
    let mut ctx = build_context(
        &temp,
        vec![build_record(
            &parsed,
            "Issue 42",
            "Preserve parser boundary decision.",
            "2026-03-18T10:00:00Z",
        )],
    )?;
    let services = KnowledgeServices::new();
    let ingest = services
        .ingestion
        .ingest_source(
            IngestKnowledgeRequest {
                url: url.to_string(),
            },
            &mut ctx,
        )
        .await?;

    services
        .relations
        .associate_by_refs(
            &mut ctx,
            &format!("knowledge:{}", ingest.knowledge_item_id),
            "path:src/lib.rs",
        )
        .await?;

    let jobs = ctx.workplane.jobs();
    let queued = jobs.iter().any(|job| {
        job.target_capability_id.as_deref() == Some("context_guidance")
            && job.mailbox_name == "context_guidance.knowledge_distillation"
    });
    assert!(queued);
    Ok(())
}
