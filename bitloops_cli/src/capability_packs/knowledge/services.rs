use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::host::devql::RepoIdentity;
use crate::host::devql::capability_host::{
    KnowledgeExecutionContext, KnowledgeIngestContext, StageRequest,
};

use super::provenance::{
    INGEST_WRITE_ADD, INGEST_WRITE_REFRESH, IngestInvocation, IngestWriteLabels,
    build_association_provenance, build_ingestion_provenance,
};
use super::refs::{ResolvedKnowledgeTargetRef, resolve_source_ref, resolve_target_ref};
use super::storage::{
    KnowledgeDocumentVersionRow, KnowledgeItemRow, KnowledgeRelationAssertionRow,
    KnowledgeSourceRow, content_hash, knowledge_item_id, knowledge_item_version_id,
    knowledge_source_id, relation_assertion_id, serialize_payload,
};
use super::types::{
    AssociateKnowledgeRequest, AssociateKnowledgeResult, DocumentVersionSummary,
    FetchedKnowledgeDocument, IngestKnowledgeRequest, IngestKnowledgeResult,
    KnowledgeAssociationTarget, KnowledgeItemStatus, KnowledgeVersionStatus, ListVersionsRequest,
    ListVersionsResult, RefreshSourceRequest, RefreshSourceResult,
};
use super::url::parse_knowledge_url;

pub struct KnowledgeServices {
    pub ingestion: KnowledgeIngestionService,
    pub relations: KnowledgeRelationService,
    pub retrieval: KnowledgeRetrievalService,
}

impl KnowledgeServices {
    pub fn new() -> Self {
        Self {
            ingestion: KnowledgeIngestionService,
            relations: KnowledgeRelationService,
            retrieval: KnowledgeRetrievalService,
        }
    }
}

impl Default for KnowledgeServices {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KnowledgeIngestionService;

impl KnowledgeIngestionService {
    pub fn ingest_source<'a>(
        &'a self,
        request: IngestKnowledgeRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
    ) -> super::types::BoxFuture<'a, Result<IngestKnowledgeResult>> {
        Box::pin(async move {
            let (parsed, fetched) = Self::fetch_document(ctx, &request.url).await?;
            self.materialize_document(ctx, &parsed, fetched, INGEST_WRITE_ADD)
        })
    }

    pub fn refresh_source<'a>(
        &'a self,
        request: RefreshSourceRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
    ) -> super::types::BoxFuture<'a, Result<RefreshSourceResult>> {
        Box::pin(async move {
            let resolved = resolve_source_ref(ctx, &request.knowledge_ref)?;
            let item = ctx
                .relational()
                .find_item_by_id(&ctx.repo().repo_id, &resolved.knowledge_item_id)?
                .ok_or_else(|| {
                    anyhow!("knowledge item `{}` not found", resolved.knowledge_item_id)
                })?;
            let source = ctx
                .relational()
                .find_source_by_id(&item.knowledge_source_id)?
                .ok_or_else(|| {
                    anyhow!(
                        "knowledge source `{}` not found for knowledge item `{}`",
                        item.knowledge_source_id,
                        item.knowledge_item_id
                    )
                })?;

            let (parsed, fetched) = Self::fetch_document(ctx, &source.canonical_url).await?;
            let ingest_result =
                self.materialize_document(ctx, &parsed, fetched, INGEST_WRITE_REFRESH)?;

            let new_version_created = matches!(
                ingest_result.version_status,
                KnowledgeVersionStatus::Created
            );

            Ok(RefreshSourceResult {
                knowledge_item_id: ingest_result.knowledge_item_id,
                latest_document_version_id: ingest_result.knowledge_item_version_id,
                content_changed: new_version_created,
                new_version_created,
            })
        })
    }

    async fn fetch_document(
        ctx: &mut dyn KnowledgeIngestContext,
        url: &str,
    ) -> Result<(super::types::ParsedKnowledgeUrl, FetchedKnowledgeDocument)> {
        let parsed = parse_knowledge_url(url)?;
        let adapter = ctx.connectors().knowledge_adapter_for(&parsed)?;
        let fetched = adapter.fetch(&parsed, ctx.connector_context()).await?;
        Ok((parsed, fetched.into()))
    }

    fn materialize_document(
        &self,
        ctx: &mut dyn KnowledgeIngestContext,
        parsed: &super::types::ParsedKnowledgeUrl,
        fetched: FetchedKnowledgeDocument,
        labels: IngestWriteLabels,
    ) -> Result<IngestKnowledgeResult> {
        let payload_value = serde_json::to_value(&fetched.payload)
            .context("serialising knowledge payload envelope")?;
        let payload_bytes = serialize_payload(&payload_value)?;
        let hash = content_hash(&payload_bytes);

        let source_id = knowledge_source_id(&parsed.canonical_external_id);
        let item_id = knowledge_item_id(&ctx.repo().repo_id, &source_id);
        let derived_knowledge_item_version_id = knowledge_item_version_id(&item_id, &hash);
        let provenance =
            build_ingestion_provenance(parsed, labels, IngestInvocation::from_context(ctx));
        let provenance_json =
            serde_json::to_string(&provenance).context("serialising knowledge provenance")?;

        let existing_item = ctx
            .relational()
            .find_item(&ctx.repo().repo_id, &source_id)?;
        let existing_knowledge_item_version = ctx
            .documents()
            .has_knowledge_item_version(&item_id, &hash)?;
        let item_status = if existing_item.is_some() {
            KnowledgeItemStatus::Reused
        } else {
            KnowledgeItemStatus::Created
        };
        let version_status = if existing_knowledge_item_version.is_some() {
            KnowledgeVersionStatus::Reused
        } else {
            KnowledgeVersionStatus::Created
        };

        let current_knowledge_item_version_id = existing_knowledge_item_version
            .clone()
            .unwrap_or_else(|| derived_knowledge_item_version_id.clone());

        let source_row = KnowledgeSourceRow {
            knowledge_source_id: source_id.clone(),
            provider: parsed.provider.as_str().to_string(),
            source_kind: parsed.source_kind.as_str().to_string(),
            canonical_external_id: parsed.canonical_external_id.clone(),
            canonical_url: parsed.canonical_url.clone(),
            provenance_json: provenance_json.clone(),
        };
        let item_row = KnowledgeItemRow {
            knowledge_item_id: item_id.clone(),
            repo_id: ctx.repo().repo_id.clone(),
            knowledge_source_id: source_id,
            item_kind: parsed.source_kind.as_str().to_string(),
            latest_knowledge_item_version_id: current_knowledge_item_version_id.clone(),
            provenance_json: provenance_json.clone(),
        };

        let mut written_payload = None;
        let mut inserted_knowledge_item_version = None;

        if existing_knowledge_item_version.is_none() {
            let payload_ref = ctx.blob_payloads().write_payload(
                &ctx.repo().repo_id,
                &item_id,
                &derived_knowledge_item_version_id,
                &payload_bytes,
            )?;

            let document_row = KnowledgeDocumentVersionRow {
                knowledge_item_version_id: derived_knowledge_item_version_id.clone(),
                knowledge_item_id: item_id.clone(),
                provider: parsed.provider.as_str().to_string(),
                source_kind: parsed.source_kind.as_str().to_string(),
                content_hash: hash,
                title: fetched.title,
                state: fetched.state,
                author: fetched.author,
                updated_at: fetched.updated_at,
                body_preview: fetched.body_preview,
                normalized_fields_json: serde_json::to_string(&fetched.normalized_fields)
                    .context("serialising normalized knowledge fields")?,
                storage_backend: payload_ref.storage_backend.clone(),
                storage_path: payload_ref.storage_path.clone(),
                payload_mime_type: payload_ref.mime_type.clone(),
                payload_size_bytes: payload_ref.size_bytes,
                provenance_json: provenance_json.clone(),
                created_at: None,
            };

            if let Err(err) = ctx.documents().insert_knowledge_item_version(&document_row) {
                let _ = ctx.blob_payloads().delete_payload(&payload_ref);
                return Err(err);
            }

            written_payload = Some(payload_ref);
            inserted_knowledge_item_version = Some(derived_knowledge_item_version_id);
        }

        if let Err(err) = ctx.relational().persist_ingestion(&source_row, &item_row) {
            if let Some(knowledge_item_version_id) = inserted_knowledge_item_version.as_deref() {
                let _ = ctx
                    .documents()
                    .delete_knowledge_item_version(knowledge_item_version_id);
            }
            if let Some(payload) = written_payload.as_ref() {
                let _ = ctx.blob_payloads().delete_payload(payload);
            }
            return Err(err);
        }

        Ok(IngestKnowledgeResult {
            provider: parsed.provider.as_str().to_string(),
            source_kind: parsed.source_kind.as_str().to_string(),
            repo_identity: ctx.repo().identity.clone(),
            knowledge_item_id: item_id,
            knowledge_item_version_id: current_knowledge_item_version_id,
            item_status,
            version_status,
        })
    }
}

pub struct KnowledgeRelationService;

impl KnowledgeRelationService {
    pub fn associate_to_commit<'a>(
        &'a self,
        ctx: &'a mut dyn KnowledgeIngestContext,
        ingest_result: &'a IngestKnowledgeResult,
        commit: &'a str,
    ) -> super::types::BoxFuture<'a, Result<AssociateKnowledgeResult>> {
        Box::pin(async move {
            let target = resolve_target_ref(ctx, &format!("commit:{commit}"))?;
            let ResolvedKnowledgeTargetRef::Commit { sha } = target else {
                bail!("internal: expected commit target from commit ref");
            };

            self.associate(
                ctx,
                AssociateKnowledgeRequest {
                    knowledge_item_id: ingest_result.knowledge_item_id.clone(),
                    source_knowledge_item_version_id: ingest_result
                        .knowledge_item_version_id
                        .clone(),
                    target: KnowledgeAssociationTarget::Commit { sha },
                    relation_type: "associated_with".to_string(),
                    association_method: "manual_attachment".to_string(),
                    command: "bitloops devql knowledge add".to_string(),
                },
            )
        })
    }

    pub fn associate_by_refs<'a>(
        &'a self,
        ctx: &'a mut dyn KnowledgeIngestContext,
        source_ref: &'a str,
        target_ref: &'a str,
    ) -> super::types::BoxFuture<'a, Result<AssociateKnowledgeResult>> {
        Box::pin(async move {
            let resolved_source = resolve_source_ref(ctx, source_ref)?;
            let resolved_target = resolve_target_ref(ctx, target_ref)?;

            let target = match resolved_target {
                ResolvedKnowledgeTargetRef::Commit { sha } => {
                    KnowledgeAssociationTarget::Commit { sha }
                }
                ResolvedKnowledgeTargetRef::KnowledgeItem {
                    knowledge_item_id,
                    target_knowledge_item_version_id,
                } => KnowledgeAssociationTarget::KnowledgeItem {
                    knowledge_item_id,
                    target_knowledge_item_version_id,
                },
                ResolvedKnowledgeTargetRef::Checkpoint { checkpoint_id } => {
                    KnowledgeAssociationTarget::Checkpoint { checkpoint_id }
                }
                ResolvedKnowledgeTargetRef::Artefact { artefact_id } => {
                    KnowledgeAssociationTarget::Artefact { artefact_id }
                }
            };

            self.associate(
                ctx,
                AssociateKnowledgeRequest {
                    knowledge_item_id: resolved_source.knowledge_item_id,
                    source_knowledge_item_version_id: resolved_source
                        .source_knowledge_item_version_id,
                    target,
                    relation_type: "associated_with".to_string(),
                    association_method: "manual_attachment".to_string(),
                    command: "bitloops devql knowledge associate".to_string(),
                },
            )
        })
    }

    pub fn associate(
        &self,
        ctx: &mut dyn KnowledgeIngestContext,
        request: AssociateKnowledgeRequest,
    ) -> Result<AssociateKnowledgeResult> {
        let target_type = request.target.target_type().to_string();
        let target_id = request.target.target_id().to_string();
        let target_knowledge_item_version_id = request
            .target
            .target_knowledge_item_version_id()
            .map(str::to_string);
        let provenance = build_association_provenance(
            &request.command,
            &request.source_knowledge_item_version_id,
            &target_type,
            &target_id,
            target_knowledge_item_version_id.as_deref(),
            &request.association_method,
            IngestInvocation::from_context(ctx),
        );
        let provenance_json = serde_json::to_string(&provenance)
            .context("serialising knowledge association provenance")?;

        let relation = KnowledgeRelationAssertionRow {
            relation_assertion_id: relation_assertion_id(
                &request.knowledge_item_id,
                &request.source_knowledge_item_version_id,
                &target_type,
                &target_id,
                target_knowledge_item_version_id.as_deref(),
                &request.association_method,
            ),
            repo_id: ctx.repo().repo_id.clone(),
            knowledge_item_id: request.knowledge_item_id,
            source_knowledge_item_version_id: request.source_knowledge_item_version_id,
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            target_knowledge_item_version_id,
            relation_type: request.relation_type,
            association_method: request.association_method,
            confidence: 1.0,
            provenance_json,
        };

        ctx.relational().insert_relation_assertion(&relation)?;

        Ok(AssociateKnowledgeResult {
            relation_assertion_id: relation.relation_assertion_id,
            target_type,
            target_id,
            relation_type: relation.relation_type,
            association_method: relation.association_method,
        })
    }
}

pub struct KnowledgeRetrievalService;

impl KnowledgeRetrievalService {
    pub fn list_repository_knowledge(
        &self,
        repo: &RepoIdentity,
        request: &StageRequest,
        ctx: &mut dyn KnowledgeExecutionContext,
    ) -> Result<Vec<Value>> {
        let limit = request.limit().unwrap_or(100).max(1);
        let items = ctx.relational().list_items_for_repo(&repo.repo_id, limit)?;

        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let Some(version) = ctx
                .documents()
                .find_knowledge_item_version(&item.latest_knowledge_item_version_id)?
            else {
                continue;
            };

            rows.push(json!({
                "id": item.knowledge_item_id,
                "knowledge_item_id": item.knowledge_item_id,
                "knowledge_item_version_id": version.knowledge_item_version_id,
                "title": version.title,
                "source_kind": version.source_kind,
                "provider": version.provider,
                "updated_at": version.updated_at,
                "created_at": version.created_at,
                "body_preview": version.body_preview,
            }));
        }

        Ok(rows)
    }

    pub fn list_versions<'a>(
        &'a self,
        request: ListVersionsRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
    ) -> super::types::BoxFuture<'a, Result<ListVersionsResult>> {
        Box::pin(async move {
            let resolved = resolve_source_ref(ctx, &request.knowledge_ref)?;
            let versions = ctx
                .documents()
                .list_versions_for_item(&resolved.knowledge_item_id)?
                .into_iter()
                .map(|row| DocumentVersionSummary {
                    knowledge_item_version_id: row.knowledge_item_version_id,
                    content_hash: row.content_hash,
                    title: row.title,
                    updated_at: row.updated_at,
                    created_at: row.created_at,
                })
                .collect::<Vec<_>>();

            Ok(ListVersionsResult {
                knowledge_item_id: resolved.knowledge_item_id,
                versions,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
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
        BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
    };
    use crate::capability_packs::knowledge::url::parse_knowledge_url;
    use crate::config::{
        BlobStorageConfig, BlobStorageProvider, EventsBackendConfig, EventsProvider,
        ProviderConfig, RelationalBackendConfig, RelationalProvider, StoreBackendConfig,
    };
    use crate::host::devql::RepoIdentity;
    use crate::host::devql::capability_host::config_view::CapabilityConfigView;
    use crate::host::devql::capability_host::gateways::{
        BlobPayloadGateway, CanonicalGraphGateway, DocumentStoreGateway, ProvenanceBuilder,
        RelationalGateway,
    };
    use crate::host::devql::capability_host::{
        CapabilityExecutionContext, CapabilityIngestContext, KnowledgeExecutionContext,
        KnowledgeIngestContext, StageRequest,
    };
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
        ) -> crate::adapters::connectors::types::BoxFuture<'a, Result<ExternalKnowledgeRecord>>
        {
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

    struct TestRuntimeContext {
        repo_root: PathBuf,
        repo: RepoIdentity,
        config_root: Value,
        relational: SqliteKnowledgeRelationalStore,
        documents: DuckdbKnowledgeDocumentStore,
        blobs: BlobKnowledgePayloadStore,
        connectors: StubConnectorRegistry,
        provenance: TestProvenanceBuilder,
        graph: TestGraphGateway,
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
    }

    impl KnowledgeExecutionContext for TestRuntimeContext {
        fn relational(&self) -> &dyn RelationalGateway {
            &self.relational
        }

        fn documents(&self) -> &dyn DocumentStoreGateway {
            &self.documents
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

        fn invoking_capability_id(&self) -> Option<&str> {
            self.invoking_capability_id
        }

        fn invoking_ingester_id(&self) -> Option<&str> {
            self.invoking_ingester_id
        }
    }

    impl KnowledgeIngestContext for TestRuntimeContext {
        fn relational(&self) -> &dyn RelationalGateway {
            &self.relational
        }

        fn documents(&self) -> &dyn DocumentStoreGateway {
            &self.documents
        }
    }

    fn test_backends(temp: &TempDir) -> StoreBackendConfig {
        StoreBackendConfig {
            relational: RelationalBackendConfig {
                provider: RelationalProvider::Sqlite,
                sqlite_path: Some(
                    temp.path()
                        .join("knowledge-relational.sqlite")
                        .to_string_lossy()
                        .to_string(),
                ),
                postgres_dsn: None,
            },
            events: EventsBackendConfig {
                provider: EventsProvider::DuckDb,
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
                provider: BlobStorageProvider::Local,
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
        let relational =
            SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
        relational.initialise_schema()?;
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
            documents,
            blobs,
            connectors,
            provenance: TestProvenanceBuilder,
            graph: TestGraphGateway,
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
}
