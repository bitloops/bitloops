use anyhow::{Result, bail, ensure};
use serde_json::json;

use crate::capability_packs::knowledge::{
    AssociateKnowledgeResult, IngestKnowledgeResult, KnowledgeItemStatus, KnowledgeVersionStatus,
    RefreshSourceResult, format_knowledge_add_result, format_knowledge_associate_result,
    format_knowledge_refresh_result,
};
use crate::devql_transport::SlimCliRepoScope;

use super::graphql::execute_devql_graphql;

const ADD_KNOWLEDGE_MUTATION: &str = r#"
    mutation AddKnowledge($input: AddKnowledgeInput!) {
      addKnowledge(input: $input) {
        success
        knowledgeItemVersionId
        itemCreated
        newVersionCreated
        knowledgeItem {
          id
          provider
          sourceKind
        }
        association {
          id
          targetType
          targetId
          relationType
          associationMethod
        }
      }
    }
"#;

const ASSOCIATE_KNOWLEDGE_MUTATION: &str = r#"
    mutation AssociateKnowledge($input: AssociateKnowledgeInput!) {
      associateKnowledge(input: $input) {
        success
        relation {
          id
          targetType
          targetId
          relationType
          associationMethod
        }
      }
    }
"#;

const REFRESH_KNOWLEDGE_MUTATION: &str = r#"
    mutation RefreshKnowledge($input: RefreshKnowledgeInput!) {
      refreshKnowledge(input: $input) {
        success
        latestDocumentVersionId
        contentChanged
        newVersionCreated
        knowledgeItem {
          id
          provider
          sourceKind
        }
      }
    }
"#;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddKnowledgeMutationData {
    add_knowledge: AddKnowledgeGraphqlResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddKnowledgeGraphqlResult {
    success: bool,
    knowledge_item_version_id: String,
    item_created: bool,
    new_version_created: bool,
    knowledge_item: GraphqlKnowledgeItemSummary,
    association: Option<GraphqlKnowledgeRelationSummary>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssociateKnowledgeMutationData {
    associate_knowledge: AssociateKnowledgeGraphqlResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssociateKnowledgeGraphqlResult {
    success: bool,
    relation: GraphqlKnowledgeRelationSummary,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshKnowledgeMutationData {
    refresh_knowledge: RefreshKnowledgeGraphqlResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshKnowledgeGraphqlResult {
    success: bool,
    latest_document_version_id: String,
    content_changed: bool,
    new_version_created: bool,
    knowledge_item: GraphqlKnowledgeItemSummary,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlKnowledgeItemSummary {
    id: String,
    provider: String,
    source_kind: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlKnowledgeRelationSummary {
    id: String,
    target_type: String,
    target_id: String,
    relation_type: String,
    association_method: String,
}

pub(super) async fn run_knowledge_add_via_graphql(
    scope: &SlimCliRepoScope,
    url: &str,
    commit: Option<&str>,
) -> Result<()> {
    let response: AddKnowledgeMutationData = execute_devql_graphql(
        scope,
        ADD_KNOWLEDGE_MUTATION,
        json!({
            "input": {
                "url": url,
                "commitRef": commit,
            }
        }),
    )
    .await?;
    ensure!(
        response.add_knowledge.success,
        "GraphQL mutation `addKnowledge` reported unsuccessful execution"
    );

    let ingest = IngestKnowledgeResult {
        provider: graphql_provider_to_storage(&response.add_knowledge.knowledge_item.provider)?
            .to_string(),
        source_kind: graphql_source_kind_to_storage(
            &response.add_knowledge.knowledge_item.source_kind,
        )?
        .to_string(),
        repo_identity: scope.repo.identity.clone(),
        knowledge_item_id: response.add_knowledge.knowledge_item.id,
        knowledge_item_version_id: response.add_knowledge.knowledge_item_version_id,
        item_status: if response.add_knowledge.item_created {
            KnowledgeItemStatus::Created
        } else {
            KnowledgeItemStatus::Reused
        },
        version_status: if response.add_knowledge.new_version_created {
            KnowledgeVersionStatus::Created
        } else {
            KnowledgeVersionStatus::Reused
        },
    };
    let association = response
        .add_knowledge
        .association
        .map(graphql_relation_to_cli_result)
        .transpose()?;

    println!(
        "{}",
        format_knowledge_add_result(&ingest, association.as_ref())
    );
    Ok(())
}

pub(super) async fn run_knowledge_associate_via_graphql(
    scope: &SlimCliRepoScope,
    source_ref: &str,
    target_ref: &str,
) -> Result<()> {
    let response: AssociateKnowledgeMutationData = execute_devql_graphql(
        scope,
        ASSOCIATE_KNOWLEDGE_MUTATION,
        json!({
            "input": {
                "sourceRef": source_ref,
                "targetRef": target_ref,
            }
        }),
    )
    .await?;
    ensure!(
        response.associate_knowledge.success,
        "GraphQL mutation `associateKnowledge` reported unsuccessful execution"
    );

    let relation = graphql_relation_to_cli_result(response.associate_knowledge.relation)?;
    println!("{}", format_knowledge_associate_result(&relation));
    Ok(())
}

pub(super) async fn run_knowledge_refresh_via_graphql(
    scope: &SlimCliRepoScope,
    knowledge_ref: &str,
) -> Result<()> {
    let response: RefreshKnowledgeMutationData = execute_devql_graphql(
        scope,
        REFRESH_KNOWLEDGE_MUTATION,
        json!({
            "input": {
                "knowledgeRef": knowledge_ref,
            }
        }),
    )
    .await?;
    ensure!(
        response.refresh_knowledge.success,
        "GraphQL mutation `refreshKnowledge` reported unsuccessful execution"
    );

    let refresh = RefreshSourceResult {
        knowledge_item_id: response.refresh_knowledge.knowledge_item.id,
        latest_document_version_id: response.refresh_knowledge.latest_document_version_id,
        content_changed: response.refresh_knowledge.content_changed,
        new_version_created: response.refresh_knowledge.new_version_created,
    };
    println!("{}", format_knowledge_refresh_result(&refresh));
    Ok(())
}

fn graphql_relation_to_cli_result(
    relation: GraphqlKnowledgeRelationSummary,
) -> Result<AssociateKnowledgeResult> {
    Ok(AssociateKnowledgeResult {
        relation_assertion_id: relation.id,
        target_type: graphql_target_type_to_storage(&relation.target_type)?.to_string(),
        target_id: relation.target_id,
        relation_type: relation.relation_type,
        association_method: relation.association_method,
    })
}

fn graphql_provider_to_storage(raw: &str) -> Result<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "GITHUB" => Ok("github"),
        "JIRA" => Ok("jira"),
        "CONFLUENCE" => Ok("confluence"),
        other => bail!("unsupported GraphQL knowledge provider `{other}`"),
    }
}

fn graphql_source_kind_to_storage(raw: &str) -> Result<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "ISSUE" => Ok("github_issue"),
        "PULL_REQUEST" => Ok("github_pull_request"),
        "JIRA_ISSUE" => Ok("jira_issue"),
        "CONFLUENCE_PAGE" => Ok("confluence_page"),
        other => bail!("unsupported GraphQL knowledge source kind `{other}`"),
    }
}

fn graphql_target_type_to_storage(raw: &str) -> Result<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "COMMIT" => Ok("commit"),
        "CHECKPOINT" => Ok("checkpoint"),
        "ARTEFACT" => Ok("artefact"),
        "KNOWLEDGE" => Ok("knowledge_item"),
        other => bail!("unsupported GraphQL knowledge target type `{other}`"),
    }
}
