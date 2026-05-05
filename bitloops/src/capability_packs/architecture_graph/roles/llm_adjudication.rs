use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::devql_transport::SlimCliRepoScope;
use crate::host::capability_host::CurrentStateConsumerContext;
use crate::host::inference::{StructuredGenerationRequest, StructuredGenerationService};

use super::taxonomy::{
    SeededArchitectureTaxonomy, architecture_roles_seed_schema, generic_role_family_examples,
    validate_seeded_taxonomy,
};

const MAX_FILE_EVIDENCE: usize = 120;
const MAX_ARTEFACT_EVIDENCE: usize = 200;
const MAX_EDGE_EVIDENCE: usize = 200;
const MAX_GRAPH_EVIDENCE: usize = 120;

pub(crate) async fn collect_seed_evidence(
    scope: &SlimCliRepoScope,
    context: &CurrentStateConsumerContext,
) -> Result<Value> {
    let files = context
        .relational
        .load_current_canonical_files(&scope.repo.repo_id)
        .context("loading current canonical files for architecture role seed")?;
    let artefacts = context
        .relational
        .load_current_canonical_artefacts(&scope.repo.repo_id)
        .context("loading current canonical artefacts for architecture role seed")?;
    let dependency_edges = context
        .relational
        .load_current_canonical_edges(&scope.repo.repo_id)
        .context("loading current canonical dependency edges for architecture role seed")?;

    let repo_metadata = load_repository_metadata(context, &scope.repo.repo_id).await?;
    let file_contexts = load_file_context_evidence(context, &scope.repo.repo_id).await?;
    let architecture_graph = load_architecture_graph_evidence(context, &scope.repo.repo_id).await?;
    let semantic_summaries = load_semantic_summary_evidence(context, &scope.repo.repo_id).await?;

    Ok(json!({
        "repository": {
            "repo_id": scope.repo.repo_id,
            "provider": scope.repo.provider,
            "organization": scope.repo.organization,
            "name": scope.repo.name,
            "identity": scope.repo.identity,
            "repo_root": scope.repo_root.display().to_string(),
            "branch_name": scope.branch_name,
            "project_path": scope.project_path,
            "metadata": repo_metadata,
        },
        "language_framework_signals": file_contexts,
        "canonical_files": files
            .iter()
            .take(MAX_FILE_EVIDENCE)
            .map(|file| json!({
                "path": file.path,
                "analysis_mode": file.analysis_mode,
                "file_role": file.file_role,
                "language": file.language,
                "resolved_language": file.resolved_language,
            }))
            .collect::<Vec<_>>(),
        "canonical_artefacts": artefacts
            .iter()
            .take(MAX_ARTEFACT_EVIDENCE)
            .map(|artefact| json!({
                "artefact_id": artefact.artefact_id,
                "path": artefact.path,
                "language": artefact.language,
                "canonical_kind": artefact.canonical_kind,
                "language_kind": artefact.language_kind,
                "symbol_fqn": artefact.symbol_fqn,
                "signature": artefact.signature,
                "docstring": artefact.docstring,
            }))
            .collect::<Vec<_>>(),
        "artefact_summaries": semantic_summaries,
        "dependency_graph_hints": dependency_edges
            .iter()
            .take(MAX_EDGE_EVIDENCE)
            .map(|edge| json!({
                "edge_id": edge.edge_id,
                "path": edge.path,
                "from_artefact_id": edge.from_artefact_id,
                "to_artefact_id": edge.to_artefact_id,
                "to_symbol_ref": edge.to_symbol_ref,
                "edge_kind": edge.edge_kind,
                "language": edge.language,
            }))
            .collect::<Vec<_>>(),
        "existing_architecture_graph_facts": architecture_graph
            .into_iter()
            .take(MAX_GRAPH_EVIDENCE)
            .collect::<Vec<_>>(),
        "generic_role_family_examples": generic_role_family_examples(),
    }))
}

pub(crate) fn architecture_roles_seed_system_prompt() -> &'static str {
    "You infer repository-specific architectural role taxonomies. Return JSON only. \
Do not hardcode Bitloops-specific roles. Use the supplied evidence to propose role identities and reviewable deterministic rule candidates for this repository."
}

pub(crate) fn architecture_roles_seed_user_prompt(
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> String {
    json!({
        "task": "Infer a project-specific architecture role taxonomy and candidate deterministic matching rules for this repository.",
        "rules": [
            "Return repository-specific roles, not a hardcoded generic taxonomy.",
            "Generic role families are examples only; adapt them to the repository evidence.",
            "Return only stable role identities that are justified by the repository evidence.",
            "Detection rules must be reviewable and safe for deterministic use.",
            "Prefer fewer strong roles over many weak or redundant roles."
        ],
        "repository_identity": {
            "repo_id": scope.repo.repo_id,
            "provider": scope.repo.provider,
            "organization": scope.repo.organization,
            "name": scope.repo.name,
            "identity": scope.repo.identity,
            "branch_name": scope.branch_name,
        },
        "evidence": evidence,
    })
    .to_string()
}

pub(crate) fn architecture_roles_seed_request(
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> StructuredGenerationRequest {
    let mut metadata = Map::new();
    metadata.insert(
        "capability_id".to_string(),
        Value::String("architecture_graph".to_string()),
    );
    metadata.insert(
        "slot_name".to_string(),
        Value::String("fact_synthesis".to_string()),
    );
    metadata.insert(
        "repo_id".to_string(),
        Value::String(scope.repo.repo_id.clone()),
    );

    StructuredGenerationRequest {
        system_prompt: architecture_roles_seed_system_prompt().to_string(),
        user_prompt: architecture_roles_seed_user_prompt(scope, evidence),
        json_schema: architecture_roles_seed_schema(),
        workspace_path: Some(scope.repo_root.display().to_string()),
        metadata,
    }
}

pub(crate) fn decode_seeded_taxonomy_response(value: Value) -> Result<SeededArchitectureTaxonomy> {
    let taxonomy: SeededArchitectureTaxonomy =
        serde_json::from_value(value).context("parse seeded architecture taxonomy")?;
    validate_seeded_taxonomy(&taxonomy)?;
    Ok(taxonomy)
}

pub(crate) fn run_seed_generation(
    service: &dyn StructuredGenerationService,
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> Result<SeededArchitectureTaxonomy> {
    let request = architecture_roles_seed_request(scope, evidence);
    let response = service.generate(request)?;
    decode_seeded_taxonomy_response(response)
}

async fn load_repository_metadata(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Value> {
    let rows = context
        .storage
        .query_rows(&format!(
            "SELECT metadata_json FROM repositories WHERE repo_id = '{}' LIMIT 1;",
            crate::host::devql::esc_pg(repo_id)
        ))
        .await
        .context("loading repository metadata for architecture role seed")?;
    match rows.first().and_then(|row| row.get("metadata_json")) {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            serde_json::from_str(text).context("parse repositories.metadata_json")
        }
        _ => Ok(json!({})),
    }
}

async fn load_file_context_evidence(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let rows = context
        .storage
        .query_rows(&format!(
            "SELECT path, resolved_language, primary_context_id, secondary_context_ids_json, \
                    frameworks_json, runtime_profile, classification_reason \
             FROM current_file_state \
             WHERE repo_id = '{}' \
             ORDER BY path ASC \
             LIMIT {};",
            crate::host::devql::esc_pg(repo_id),
            MAX_FILE_EVIDENCE
        ))
        .await
        .context("loading file context evidence for architecture role seed")?;
    rows.into_iter()
        .map(|row| {
            Ok(json!({
                "path": row.get("path").and_then(Value::as_str).unwrap_or_default(),
                "resolved_language": row.get("resolved_language").and_then(Value::as_str).unwrap_or_default(),
                "primary_context_id": row.get("primary_context_id").and_then(Value::as_str),
                "secondary_context_ids": parse_json_array_field(row.get("secondary_context_ids_json"))?,
                "frameworks": parse_json_array_field(row.get("frameworks_json"))?,
                "runtime_profile": row.get("runtime_profile").and_then(Value::as_str),
                "classification_reason": row.get("classification_reason").and_then(Value::as_str),
            }))
        })
        .collect()
}

async fn load_architecture_graph_evidence(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Vec<Value>> {
    context
        .storage
        .query_rows(&format!(
            "SELECT node_kind, label, artefact_id, path, entry_kind, confidence \
             FROM architecture_graph_nodes_current \
             WHERE repo_id = '{}' \
             ORDER BY confidence DESC, label ASC \
             LIMIT {};",
            crate::host::devql::esc_pg(repo_id),
            MAX_GRAPH_EVIDENCE
        ))
        .await
        .context("loading architecture graph evidence for architecture role seed")
}

async fn load_semantic_summary_evidence(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Vec<Value>> {
    context
        .storage
        .query_rows(&format!(
            "SELECT a.artefact_id AS artefact_id, a.path AS path, \
                    COALESCE(sc.summary, sh.summary) AS summary \
             FROM artefacts_current a \
             LEFT JOIN symbol_semantics_current sc \
               ON sc.repo_id = a.repo_id \
              AND sc.artefact_id = a.artefact_id \
              AND sc.content_id = a.content_id \
             LEFT JOIN symbol_semantics sh \
               ON sh.repo_id = a.repo_id \
              AND sh.artefact_id = a.artefact_id \
              AND sh.blob_sha = a.content_id \
             WHERE a.repo_id = '{}' \
               AND COALESCE(sc.summary, sh.summary, '') <> '' \
             ORDER BY a.path ASC \
             LIMIT {};",
            crate::host::devql::esc_pg(repo_id),
            MAX_ARTEFACT_EVIDENCE
        ))
        .await
        .context("loading semantic summary evidence for architecture role seed")
}

fn parse_json_array_field(value: Option<&Value>) -> Result<Value> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            serde_json::from_str(text).context("parse JSON array field")
        }
        _ => Ok(json!([])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::inference::StructuredGenerationService;

    struct FakeStructuredGenerationService {
        response: Value,
    }

    impl StructuredGenerationService for FakeStructuredGenerationService {
        fn descriptor(&self) -> String {
            "test:fake".to_string()
        }

        fn generate(&self, _request: StructuredGenerationRequest) -> Result<Value> {
            Ok(self.response.clone())
        }
    }

    fn test_scope() -> SlimCliRepoScope {
        SlimCliRepoScope {
            repo: crate::host::devql::RepoIdentity {
                repo_id: "repo-1".to_string(),
                provider: "git".to_string(),
                organization: "bitloops".to_string(),
                name: "demo".to_string(),
                identity: "git/bitloops/demo".to_string(),
            },
            repo_root: std::path::PathBuf::from("/tmp/demo"),
            branch_name: "main".to_string(),
            project_path: None,
            git_dir_relative_path: ".git".to_string(),
            config_fingerprint: "fingerprint".to_string(),
        }
    }

    #[test]
    fn seed_prompt_mentions_project_specific_inference() {
        let prompt = architecture_roles_seed_user_prompt(&test_scope(), &json!({"files": []}));
        assert!(prompt.contains("project-specific architecture role taxonomy"));
        assert!(prompt.contains("Generic role families are examples only"));
    }

    #[test]
    fn decode_seeded_taxonomy_response_rejects_invalid_payload() {
        let err = decode_seeded_taxonomy_response(json!({
            "roles": [],
            "rule_candidates": []
        }))
        .expect_err("expected empty roles to fail");
        assert!(err.to_string().contains("did not include any roles"));
    }

    #[test]
    fn run_seed_generation_validates_structured_response() {
        let service = FakeStructuredGenerationService {
            response: json!({
                "roles": [{
                    "canonical_key": "command_dispatcher",
                    "display_name": "Command Dispatcher"
                }],
                "rule_candidates": [{
                    "target_role_key": "command_dispatcher",
                    "candidate_selector": {
                        "path_prefixes": ["src/cli"]
                    }
                }]
            }),
        };
        let taxonomy =
            run_seed_generation(&service, &test_scope(), &json!({"canonical_files": []}))
                .expect("valid taxonomy");
        assert_eq!(taxonomy.roles.len(), 1);
        assert_eq!(taxonomy.rule_candidates.len(), 1);
    }
}
