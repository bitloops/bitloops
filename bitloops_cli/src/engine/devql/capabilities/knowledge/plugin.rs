use anyhow::{Context, Result};
use serde_json::json;

use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::RepoIdentity;
use crate::store_config::{
    resolve_provider_config_for_repo, resolve_store_backend_config_for_repo,
};

use super::provenance::{build_association_provenance, build_ingestion_provenance};
use super::refs::{ResolvedKnowledgeTargetRef, resolve_source_ref, resolve_target_ref};
use super::providers::{
    ConfluenceKnowledgeClient, GitHubKnowledgeClient, JiraKnowledgeClient, KnowledgeProviderClient,
};
use super::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, KnowledgeDocumentVersionRow,
    KnowledgeItemRow, KnowledgeRelationAssertionRow, KnowledgeSourceRow,
    SqliteKnowledgeRelationalStore, content_hash, document_version_id, knowledge_item_id,
    knowledge_source_id, relation_assertion_id, serialize_payload,
};
use super::types::{
    AssociateKnowledgeRequest, AssociateKnowledgeResult, BoxFuture, FetchedKnowledgeDocument,
    IngestKnowledgeRequest, IngestKnowledgeResult, KnowledgeAssociationTarget,
    KnowledgeHostContext, KnowledgeItemStatus, KnowledgeProvider, KnowledgeVersionStatus,
    ParsedKnowledgeUrl, format_knowledge_add_result, format_knowledge_associate_result,
};
use super::url::parse_knowledge_url;

pub trait KnowledgeCapability: Send + Sync {
    fn ingest_source<'a>(
        &'a self,
        host: &'a KnowledgeHostContext,
        request: IngestKnowledgeRequest,
    ) -> BoxFuture<'a, Result<IngestKnowledgeResult>>;

    fn associate<'a>(
        &'a self,
        host: &'a KnowledgeHostContext,
        request: AssociateKnowledgeRequest,
    ) -> BoxFuture<'a, Result<AssociateKnowledgeResult>>;
}

pub struct KnowledgePlugin {
    github: Box<dyn KnowledgeProviderClient>,
    jira: Box<dyn KnowledgeProviderClient>,
    confluence: Box<dyn KnowledgeProviderClient>,
}

impl KnowledgePlugin {
    pub fn builtin() -> Result<Self> {
        Ok(Self {
            github: Box::new(GitHubKnowledgeClient::new()?),
            jira: Box::new(JiraKnowledgeClient::new()?),
            confluence: Box::new(ConfluenceKnowledgeClient::new()?),
        })
    }

    #[cfg(test)]
    pub fn with_clients(
        github: Box<dyn KnowledgeProviderClient>,
        jira: Box<dyn KnowledgeProviderClient>,
        confluence: Box<dyn KnowledgeProviderClient>,
    ) -> Self {
        Self {
            github,
            jira,
            confluence,
        }
    }

    fn fetch_document<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        host: &'a KnowledgeHostContext,
    ) -> BoxFuture<'a, Result<FetchedKnowledgeDocument>> {
        Box::pin(async move {
            match parsed.provider {
                KnowledgeProvider::Github => self.github.fetch(parsed, host).await,
                KnowledgeProvider::Jira => self.jira.fetch(parsed, host).await,
                KnowledgeProvider::Confluence => self.confluence.fetch(parsed, host).await,
            }
        })
    }

    fn materialize_document(
        &self,
        host: &KnowledgeHostContext,
        parsed: &ParsedKnowledgeUrl,
        fetched: FetchedKnowledgeDocument,
    ) -> Result<IngestKnowledgeResult> {
        let payload_json = json!({
            "raw_payload": fetched.payload.raw_payload,
            "body_text": fetched.payload.body_text,
            "body_html": fetched.payload.body_html,
            "body_adf": fetched.payload.body_adf,
        });
        let payload_bytes = serialize_payload(&payload_json)?;
        let hash = content_hash(&payload_bytes);

        let source_id = knowledge_source_id(&parsed.canonical_external_id);
        let item_id = knowledge_item_id(&host.repo.repo_id, &source_id);
        let derived_document_version_id = document_version_id(&item_id, &hash);
        let provenance = build_ingestion_provenance(parsed);
        let provenance_json =
            serde_json::to_string(&provenance).context("serialising knowledge provenance")?;

        let existing_item = host
            .relational_store
            .find_item(&host.repo.repo_id, &source_id)?;
        let existing_version = host.document_store.has_document_version(&item_id, &hash)?;
        let item_status = if existing_item.is_some() {
            KnowledgeItemStatus::Reused
        } else {
            KnowledgeItemStatus::Created
        };
        let version_status = if existing_version.is_some() {
            KnowledgeVersionStatus::Reused
        } else {
            KnowledgeVersionStatus::Created
        };

        let current_document_version_id = existing_version
            .clone()
            .unwrap_or_else(|| derived_document_version_id.clone());

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
            repo_id: host.repo.repo_id.clone(),
            knowledge_source_id: source_id,
            item_kind: parsed.source_kind.as_str().to_string(),
            latest_document_version_id: current_document_version_id.clone(),
            provenance_json: provenance_json.clone(),
        };

        let mut written_payload = None;
        let mut inserted_document_version = None;

        if existing_version.is_none() {
            let payload_ref = host.payload_store.write_payload(
                &host.repo.repo_id,
                &item_id,
                &derived_document_version_id,
                &payload_bytes,
            )?;

            let document_row = KnowledgeDocumentVersionRow {
                document_version_id: derived_document_version_id.clone(),
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
            };

            if let Err(err) = host.document_store.insert_document_version(&document_row) {
                let _ = host.payload_store.delete_payload(&payload_ref);
                return Err(err);
            }

            written_payload = Some(payload_ref);
            inserted_document_version = Some(derived_document_version_id);
        }

        if let Err(err) = host
            .relational_store
            .persist_ingestion(&source_row, &item_row)
        {
            if let Some(document_version_id) = inserted_document_version.as_deref() {
                let _ = host
                    .document_store
                    .delete_document_version(document_version_id);
            }
            if let Some(payload) = written_payload.as_ref() {
                let _ = host.payload_store.delete_payload(payload);
            }
            return Err(err);
        }

        Ok(IngestKnowledgeResult {
            provider: parsed.provider.as_str().to_string(),
            source_kind: parsed.source_kind.as_str().to_string(),
            repo_identity: host.repo.identity.clone(),
            knowledge_item_id: item_id,
            document_version_id: current_document_version_id,
            item_status,
            version_status,
        })
    }
}

impl KnowledgeCapability for KnowledgePlugin {
    fn ingest_source<'a>(
        &'a self,
        host: &'a KnowledgeHostContext,
        request: IngestKnowledgeRequest,
    ) -> BoxFuture<'a, Result<IngestKnowledgeResult>> {
        Box::pin(async move {
            let parsed = parse_knowledge_url(&request.url)?;
            host.relational_store.initialise_schema()?;
            host.document_store.initialise_schema()?;

            let fetched = self.fetch_document(&parsed, host).await?;
            self.materialize_document(host, &parsed, fetched)
        })
    }

    fn associate<'a>(
        &'a self,
        host: &'a KnowledgeHostContext,
        request: AssociateKnowledgeRequest,
    ) -> BoxFuture<'a, Result<AssociateKnowledgeResult>> {
        Box::pin(async move {
            host.relational_store.initialise_schema()?;

            let target_type = request.target.target_type().to_string();
            let target_id = request.target.target_id().to_string();
            let provenance = build_association_provenance(
                &request.command,
                &request.source_document_version_id,
                &target_type,
                &target_id,
                &request.association_method,
            );
            let provenance_json = serde_json::to_string(&provenance)
                .context("serialising knowledge association provenance")?;
            let relation = KnowledgeRelationAssertionRow {
                relation_assertion_id: relation_assertion_id(
                    &request.knowledge_item_id,
                    &request.source_document_version_id,
                    &target_type,
                    &target_id,
                    &request.association_method,
                ),
                repo_id: host.repo.repo_id.clone(),
                knowledge_item_id: request.knowledge_item_id,
                source_document_version_id: request.source_document_version_id,
                target_type: target_type.clone(),
                target_id: target_id.clone(),
                relation_type: request.relation_type,
                association_method: request.association_method,
                confidence: 1.0,
                provenance_json,
            };

            host.relational_store.insert_relation_assertion(&relation)?;

            Ok(AssociateKnowledgeResult {
                relation_assertion_id: relation.relation_assertion_id,
                target_type,
                target_id,
                relation_type: relation.relation_type,
                association_method: relation.association_method,
            })
        })
    }
}

pub async fn run_add_command(
    repo_root: &std::path::Path,
    repo: &RepoIdentity,
    url: &str,
    commit: Option<&str>,
) -> Result<()> {
    let registry = crate::engine::devql::capabilities::DevqlCapabilityRegistry::builtin()?;
    let host = build_host_context(repo_root, repo)?;
    let (ingest_result, association_result) = run_add_flow(registry.knowledge(), &host, url, commit).await?;

    println!(
        "{}",
        format_knowledge_add_result(&ingest_result, association_result.as_ref())
    );
    Ok(())
}

pub fn build_host_context(
    repo_root: &std::path::Path,
    repo: &RepoIdentity,
) -> Result<KnowledgeHostContext> {
    let backends = resolve_store_backend_config_for_repo(repo_root)?;
    let provider_config = resolve_provider_config_for_repo(repo_root)?;
    let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
    let relational_store =
        SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
    let document_store =
        DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
    let payload_store = BlobKnowledgePayloadStore::from_backend_config(repo_root, &backends)?;

    Ok(KnowledgeHostContext {
        repo_root: repo_root.to_path_buf(),
        repo: repo.clone(),
        backends,
        provider_config,
        relational_store,
        document_store,
        payload_store,
    })
}

pub(crate) async fn run_add_flow(
    capability: &dyn KnowledgeCapability,
    host: &KnowledgeHostContext,
    url: &str,
    commit: Option<&str>,
) -> Result<(IngestKnowledgeResult, Option<AssociateKnowledgeResult>)> {
    let ingest_result = capability
        .ingest_source(
            host,
            IngestKnowledgeRequest {
                url: url.to_string(),
            },
        )
        .await?;
    let association_result = if let Some(commit) = commit {
        let target = resolve_target_ref(host, &format!("commit:{commit}"))?;
        Some(
            capability
                .associate(host, build_commit_association_request(&ingest_result, target))
                .await?,
        )
    } else {
        None
    };

    Ok((ingest_result, association_result))
}

fn build_commit_association_request(
    ingest_result: &IngestKnowledgeResult,
    target: ResolvedKnowledgeTargetRef,
) -> AssociateKnowledgeRequest {
    let ResolvedKnowledgeTargetRef::Commit { sha } = target;

    AssociateKnowledgeRequest {
        knowledge_item_id: ingest_result.knowledge_item_id.clone(),
        source_document_version_id: ingest_result.document_version_id.clone(),
        target: KnowledgeAssociationTarget::Commit { sha },
        relation_type: "associated_with".to_string(),
        association_method: "manual_attachment".to_string(),
        command: "bitloops devql knowledge add".to_string(),
    }
}

pub async fn run_associate_command(
    repo_root: &std::path::Path,
    repo: &RepoIdentity,
    source_ref: &str,
    target_ref: &str,
) -> Result<()> {
    let registry = crate::engine::devql::capabilities::DevqlCapabilityRegistry::builtin()?;
    let host = build_host_context(repo_root, repo)?;
    let result = run_associate_flow(registry.knowledge(), &host, source_ref, target_ref).await?;

    println!("{}", format_knowledge_associate_result(&result));
    Ok(())
}

pub(crate) async fn run_associate_flow(
    capability: &dyn KnowledgeCapability,
    host: &KnowledgeHostContext,
    source_ref: &str,
    target_ref: &str,
) -> Result<AssociateKnowledgeResult> {
    let resolved_source = resolve_source_ref(host, source_ref)?;
    let resolved_target = resolve_target_ref(host, target_ref)?;

    let target = match resolved_target {
        ResolvedKnowledgeTargetRef::Commit { sha } => KnowledgeAssociationTarget::Commit { sha },
    };

    capability
        .associate(
            host,
            AssociateKnowledgeRequest {
                knowledge_item_id: resolved_source.knowledge_item_id,
                source_document_version_id: resolved_source.source_document_version_id,
                target,
                relation_type: "associated_with".to_string(),
                association_method: "manual_attachment".to_string(),
                command: "bitloops devql knowledge associate".to_string(),
            },
        )
        .await
}
