use async_graphql::{Context, Error, Result};
use serde::Deserialize;
use serde_json::json;

use crate::graphql::DevqlGraphqlContext;
use crate::graphql::types::{KnowledgeItem, KnowledgeRelation};

use super::errors::operation_error;
use super::inputs::{AddKnowledgeInput, AssociateKnowledgeInput, RefreshKnowledgeInput};
use super::results::{
    AddKnowledgeMutationResult, AssociateKnowledgeMutationResult, RefreshKnowledgeMutationResult,
};
use super::validation::{normalise_optional_input, require_non_empty_input};

#[derive(Debug, Deserialize)]
struct AddKnowledgeIngesterPayload {
    ingest: crate::capability_packs::knowledge::IngestKnowledgeResult,
    association: Option<crate::capability_packs::knowledge::AssociateKnowledgeResult>,
}

#[derive(Debug, Deserialize)]
struct AssociateKnowledgeIngesterPayload {
    association: crate::capability_packs::knowledge::AssociateKnowledgeResult,
}

#[derive(Debug, Deserialize)]
struct RefreshKnowledgeIngesterPayload {
    refresh: crate::capability_packs::knowledge::RefreshSourceResult,
}

pub(super) async fn add_knowledge(
    ctx: &Context<'_>,
    input: AddKnowledgeInput,
) -> Result<AddKnowledgeMutationResult> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "addKnowledge", err))?;
    let url = require_non_empty_input(input.url, "url", "addKnowledge")?;
    let commit_ref = normalise_optional_input(input.commit_ref, "commitRef", "addKnowledge")?;
    let payload: AddKnowledgeIngesterPayload = execute_knowledge_ingester(
        ctx,
        "addKnowledge",
        "knowledge.add",
        json!({
            "url": url,
            "commit": commit_ref,
        }),
    )
    .await?;

    let knowledge_item =
        load_required_knowledge_item(ctx, "addKnowledge", &payload.ingest.knowledge_item_id)
            .await?;
    let association = match payload.association {
        Some(association) => Some(
            load_required_knowledge_relation(
                ctx,
                "addKnowledge",
                &association.relation_assertion_id,
            )
            .await?,
        ),
        None => None,
    };

    Ok(AddKnowledgeMutationResult {
        success: true,
        knowledge_item_version_id: payload.ingest.knowledge_item_version_id,
        item_created: matches!(
            payload.ingest.item_status,
            crate::capability_packs::knowledge::KnowledgeItemStatus::Created
        ),
        new_version_created: matches!(
            payload.ingest.version_status,
            crate::capability_packs::knowledge::KnowledgeVersionStatus::Created
        ),
        knowledge_item,
        association,
    })
}

pub(super) async fn associate_knowledge(
    ctx: &Context<'_>,
    input: AssociateKnowledgeInput,
) -> Result<AssociateKnowledgeMutationResult> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .require_repo_write_scope()
        .map_err(|err| {
            operation_error("BAD_USER_INPUT", "validation", "associateKnowledge", err)
        })?;
    let source_ref = require_non_empty_input(input.source_ref, "sourceRef", "associateKnowledge")?;
    let target_ref = require_non_empty_input(input.target_ref, "targetRef", "associateKnowledge")?;
    let payload: AssociateKnowledgeIngesterPayload = execute_knowledge_ingester(
        ctx,
        "associateKnowledge",
        "knowledge.associate",
        json!({
            "source_ref": source_ref,
            "target_ref": target_ref,
        }),
    )
    .await?;
    let relation = load_required_knowledge_relation(
        ctx,
        "associateKnowledge",
        &payload.association.relation_assertion_id,
    )
    .await?;

    Ok(AssociateKnowledgeMutationResult {
        success: true,
        relation,
    })
}

pub(super) async fn refresh_knowledge(
    ctx: &Context<'_>,
    input: RefreshKnowledgeInput,
) -> Result<RefreshKnowledgeMutationResult> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "refreshKnowledge", err))?;
    let knowledge_ref =
        require_non_empty_input(input.knowledge_ref, "knowledgeRef", "refreshKnowledge")?;
    let payload: RefreshKnowledgeIngesterPayload = execute_knowledge_ingester(
        ctx,
        "refreshKnowledge",
        "knowledge.refresh",
        json!({
            "knowledge_ref": knowledge_ref,
        }),
    )
    .await?;
    let knowledge_item =
        load_required_knowledge_item(ctx, "refreshKnowledge", &payload.refresh.knowledge_item_id)
            .await?;

    Ok(RefreshKnowledgeMutationResult {
        success: true,
        latest_document_version_id: payload.refresh.latest_document_version_id,
        content_changed: payload.refresh.content_changed,
        new_version_created: payload.refresh.new_version_created,
        knowledge_item,
    })
}

async fn execute_knowledge_ingester<T: for<'de> Deserialize<'de>>(
    ctx: &Context<'_>,
    operation: &'static str,
    ingester_name: &'static str,
    payload: serde_json::Value,
) -> Result<T> {
    let host = ctx
        .data_unchecked::<DevqlGraphqlContext>()
        .capability_host_arc()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", operation, err))?;
    let result = host
        .invoke_ingester("knowledge", ingester_name, payload)
        .await
        .map_err(|err| map_knowledge_operation_error(operation, err))?;

    serde_json::from_value(result.payload)
        .map_err(|err| operation_error("BACKEND_ERROR", "serialization", operation, err))
}

async fn load_required_knowledge_item(
    ctx: &Context<'_>,
    operation: &'static str,
    knowledge_item_id: &str,
) -> Result<KnowledgeItem> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .find_knowledge_item_by_id(knowledge_item_id)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "knowledge", operation, err))?
        .ok_or_else(|| {
            operation_error(
                "BACKEND_ERROR",
                "knowledge",
                operation,
                format!(
                    "knowledge item `{knowledge_item_id}` was not available after `{operation}`"
                ),
            )
        })
}

async fn load_required_knowledge_relation(
    ctx: &Context<'_>,
    operation: &'static str,
    relation_assertion_id: &str,
) -> Result<KnowledgeRelation> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .find_knowledge_relation_by_id(relation_assertion_id)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "knowledge", operation, err))?
        .ok_or_else(|| {
            operation_error(
                "BACKEND_ERROR",
                "knowledge",
                operation,
                format!(
                    "knowledge relation `{relation_assertion_id}` was not available after `{operation}`"
                ),
            )
        })
}

fn map_knowledge_operation_error(operation: &'static str, error: impl std::fmt::Display) -> Error {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();

    let (code, kind) = if lower.contains("knowledge fetch failed")
        || lower.contains("sending github knowledge request")
        || lower.contains("sending jira knowledge request")
        || lower.contains("sending confluence knowledge request")
        || lower.contains("parsing github knowledge response")
        || lower.contains("parsing jira knowledge response")
        || lower.contains("parsing confluence knowledge response")
        || lower.contains("missing `knowledge.providers")
        || lower.contains("missing atlassian configuration")
    {
        ("BACKEND_ERROR", "provider")
    } else if lower.contains("target ref")
        || lower.contains("source ref")
        || lower.contains("knowledge ref")
        || lower.contains("knowledge item `")
        || lower.contains("knowledge source `")
    {
        ("BAD_USER_INPUT", "reference")
    } else if lower.contains("invalid knowledge url")
        || lower.contains("unsupported knowledge url")
        || lower.contains("must not be empty")
        || lower.contains("does not match configured")
    {
        ("BAD_USER_INPUT", "validation")
    } else {
        ("BACKEND_ERROR", "knowledge")
    };

    operation_error(code, kind, operation, message)
}
