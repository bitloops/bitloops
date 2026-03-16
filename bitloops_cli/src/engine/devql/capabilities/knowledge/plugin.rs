use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::RepoIdentity;
use crate::engine::strategy::manual_commit::run_git;
use crate::store_config::{resolve_provider_config_for_repo, resolve_store_backend_config_for_repo};

use super::providers::{
    ConfluenceKnowledgeClient, GitHubKnowledgeClient, JiraKnowledgeClient, KnowledgeProviderClient,
};
use super::provenance::build_provenance;
use super::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, KnowledgeDocumentVersionRow,
    KnowledgeItemRow, KnowledgeRelationAssertionRow, KnowledgeSourceRow,
    SqliteKnowledgeRelationalStore, content_hash, document_version_id, knowledge_item_id,
    knowledge_source_id, relation_assertion_id, serialize_payload,
};
use super::types::{
    BoxFuture, IngestKnowledgeRequest, IngestKnowledgeResult, KnowledgeHostContext,
    KnowledgeItemStatus, KnowledgeProvider, KnowledgeVersionStatus,
    format_knowledge_add_result,
};
use super::url::parse_knowledge_url;

pub trait KnowledgeCapability: Send + Sync {
    fn ingest_source<'a>(
        &'a self,
        host: &'a KnowledgeHostContext,
        request: IngestKnowledgeRequest,
    ) -> BoxFuture<'a, Result<IngestKnowledgeResult>>;
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
}

impl KnowledgeCapability for KnowledgePlugin {
    fn ingest_source<'a>(
        &'a self,
        host: &'a KnowledgeHostContext,
        request: IngestKnowledgeRequest,
    ) -> BoxFuture<'a, Result<IngestKnowledgeResult>> {
        Box::pin(async move {
            let parsed = parse_knowledge_url(&request.url)?;
            if let Some(commit) = request.commit.as_deref() {
                validate_commit_exists(&host.repo_root, commit)?;
            }

            host.relational_store.initialise_schema()?;
            host.document_store.initialise_schema()?;

            let fetched = match parsed.provider {
                KnowledgeProvider::Github => self.github.fetch(&parsed, host).await?,
                KnowledgeProvider::Jira => self.jira.fetch(&parsed, host).await?,
                KnowledgeProvider::Confluence => self.confluence.fetch(&parsed, host).await?,
            };

            let payload_json = json!({
                "raw_payload": fetched.payload.raw_payload.clone(),
                "body_text": fetched.payload.body_text.clone(),
                "body_html": fetched.payload.body_html.clone(),
                "body_adf": fetched.payload.body_adf.clone(),
            });
            let payload_bytes = serialize_payload(&payload_json)?;
            let hash = content_hash(&payload_bytes);

            let source_id = knowledge_source_id(&parsed.canonical_external_id);
            let item_id = knowledge_item_id(&host.repo.repo_id, &source_id);
            let derived_document_version_id = document_version_id(&item_id, &hash);
            let provenance = build_provenance(&parsed);
            let provenance_json =
                serde_json::to_string(&provenance).context("serialising knowledge provenance")?;

            let existing_item = host
                .relational_store
                .find_item(&host.repo.repo_id, &source_id)?;
            let existing_version = host
                .document_store
                .has_document_version(&item_id, &hash)?;
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
                knowledge_source_id: source_id.clone(),
                item_kind: parsed.source_kind.as_str().to_string(),
                latest_document_version_id: current_document_version_id.clone(),
                provenance_json: provenance_json.clone(),
            };

            let relation_row = request.commit.as_ref().map(|commit| KnowledgeRelationAssertionRow {
                relation_assertion_id: relation_assertion_id(
                    &item_id,
                    &current_document_version_id,
                    "commit",
                    commit,
                    "manual_commit_flag",
                ),
                repo_id: host.repo.repo_id.clone(),
                knowledge_item_id: item_id.clone(),
                source_document_version_id: current_document_version_id.clone(),
                target_type: "commit".to_string(),
                target_id: commit.clone(),
                relation_type: "associated_with".to_string(),
                association_method: "manual_commit_flag".to_string(),
                confidence: 1.0,
                provenance_json: provenance_json.clone(),
            });

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
                    content_hash: hash.clone(),
                    title: fetched.title.clone(),
                    state: fetched.state.clone(),
                    author: fetched.author.clone(),
                    updated_at: fetched.updated_at.clone(),
                    body_preview: fetched.body_preview.clone(),
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
                inserted_document_version = Some(derived_document_version_id.clone());
            }

            if let Err(err) = host.relational_store.persist_ingestion(
                &source_row,
                &item_row,
                relation_row.as_ref(),
            ) {
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
                relation_assertion_id: relation_row.map(|relation| relation.relation_assertion_id),
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
    let request = IngestKnowledgeRequest {
        url: url.to_string(),
        commit: commit.map(ToString::to_string),
    };

    let result = registry.knowledge().ingest_source(&host, request).await?;
    println!("{}", format_knowledge_add_result(&result));
    Ok(())
}

pub fn build_host_context(repo_root: &std::path::Path, repo: &RepoIdentity) -> Result<KnowledgeHostContext> {
    let backends = resolve_store_backend_config_for_repo(repo_root)?;
    let provider_config = resolve_provider_config_for_repo(repo_root)?;
    let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
    let relational_store =
        SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
    let document_store = DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
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

fn validate_commit_exists(repo_root: &std::path::Path, commit: &str) -> Result<()> {
    let trimmed = commit.trim();
    if trimmed.is_empty() {
        bail!("commit sha must not be empty");
    }

    run_git(repo_root, &["rev-parse", "--verify", trimmed])
        .with_context(|| format!("validating commit `{trimmed}`"))?;
    Ok(())
}
