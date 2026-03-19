use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::engine::devql::RepoIdentity;
use crate::engine::devql::capability_host::{
    CapabilityExecutionContext, CapabilityIngestContext, StageRequest,
};

use super::provenance::{build_association_provenance, build_ingestion_provenance};
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
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> super::types::BoxFuture<'a, Result<IngestKnowledgeResult>> {
        Box::pin(async move {
            let parsed = parse_knowledge_url(&request.url)?;
            let adapter = ctx.connectors().knowledge_adapter_for(&parsed)?;
            let fetched = adapter.fetch(&parsed, ctx.connector_context()).await?;
            let fetched: FetchedKnowledgeDocument = fetched.into();
            self.materialize_document(ctx, &parsed, fetched)
        })
    }

    pub fn refresh_source<'a>(
        &'a self,
        request: RefreshSourceRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> super::types::BoxFuture<'a, Result<RefreshSourceResult>> {
        Box::pin(async move {
            let resolved = resolve_source_ref(ctx, &request.knowledge_ref)?;
            let item = ctx
                .knowledge_relational()
                .find_item_by_id(&ctx.repo().repo_id, &resolved.knowledge_item_id)?
                .ok_or_else(|| {
                    anyhow!("knowledge item `{}` not found", resolved.knowledge_item_id)
                })?;
            let source = ctx
                .knowledge_relational()
                .find_source_by_id(&item.knowledge_source_id)?
                .ok_or_else(|| {
                    anyhow!(
                        "knowledge source `{}` not found for knowledge item `{}`",
                        item.knowledge_source_id,
                        item.knowledge_item_id
                    )
                })?;

            let ingest_result = self
                .ingest_source(
                    IngestKnowledgeRequest {
                        url: source.canonical_url,
                    },
                    ctx,
                )
                .await?;

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

    fn materialize_document(
        &self,
        ctx: &mut dyn CapabilityIngestContext,
        parsed: &super::types::ParsedKnowledgeUrl,
        fetched: FetchedKnowledgeDocument,
    ) -> Result<IngestKnowledgeResult> {
        let payload_value = serde_json::to_value(&fetched.payload)
            .context("serialising knowledge payload envelope")?;
        let payload_bytes = serialize_payload(&payload_value)?;
        let hash = content_hash(&payload_bytes);

        let source_id = knowledge_source_id(&parsed.canonical_external_id);
        let item_id = knowledge_item_id(&ctx.repo().repo_id, &source_id);
        let derived_knowledge_item_version_id = knowledge_item_version_id(&item_id, &hash);
        let provenance = build_ingestion_provenance(parsed);
        let provenance_json =
            serde_json::to_string(&provenance).context("serialising knowledge provenance")?;

        let existing_item = ctx
            .knowledge_relational()
            .find_item(&ctx.repo().repo_id, &source_id)?;
        let existing_knowledge_item_version = ctx
            .knowledge_documents()
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

            if let Err(err) = ctx
                .knowledge_documents()
                .insert_knowledge_item_version(&document_row)
            {
                let _ = ctx.blob_payloads().delete_payload(&payload_ref);
                return Err(err);
            }

            written_payload = Some(payload_ref);
            inserted_knowledge_item_version = Some(derived_knowledge_item_version_id);
        }

        if let Err(err) = ctx
            .knowledge_relational()
            .persist_ingestion(&source_row, &item_row)
        {
            if let Some(knowledge_item_version_id) = inserted_knowledge_item_version.as_deref() {
                let _ = ctx
                    .knowledge_documents()
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
        ctx: &'a mut dyn CapabilityIngestContext,
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
        ctx: &'a mut dyn CapabilityIngestContext,
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
                ResolvedKnowledgeTargetRef::KnowledgeItem { knowledge_item_id } => {
                    KnowledgeAssociationTarget::KnowledgeItem { knowledge_item_id }
                }
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
        ctx: &mut dyn CapabilityIngestContext,
        request: AssociateKnowledgeRequest,
    ) -> Result<AssociateKnowledgeResult> {
        let target_type = request.target.target_type().to_string();
        let target_id = request.target.target_id().to_string();
        let provenance = build_association_provenance(
            &request.command,
            &request.source_knowledge_item_version_id,
            &target_type,
            &target_id,
            &request.association_method,
        );
        let provenance_json = serde_json::to_string(&provenance)
            .context("serialising knowledge association provenance")?;

        let relation = KnowledgeRelationAssertionRow {
            relation_assertion_id: relation_assertion_id(
                &request.knowledge_item_id,
                &request.source_knowledge_item_version_id,
                &target_type,
                &target_id,
                &request.association_method,
            ),
            repo_id: ctx.repo().repo_id.clone(),
            knowledge_item_id: request.knowledge_item_id,
            source_knowledge_item_version_id: request.source_knowledge_item_version_id,
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            relation_type: request.relation_type,
            association_method: request.association_method,
            confidence: 1.0,
            provenance_json,
        };

        ctx.knowledge_relational()
            .insert_relation_assertion(&relation)?;

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
        ctx: &mut dyn CapabilityExecutionContext,
    ) -> Result<Vec<Value>> {
        let limit = request.limit().unwrap_or(100).max(1);
        let items = ctx
            .knowledge_relational()
            .list_items_for_repo(&repo.repo_id, limit)?;

        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let Some(version) = ctx
                .knowledge_documents()
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
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> super::types::BoxFuture<'a, Result<ListVersionsResult>> {
        Box::pin(async move {
            let resolved = resolve_source_ref(ctx, &request.knowledge_ref)?;
            let versions = ctx
                .knowledge_documents()
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
