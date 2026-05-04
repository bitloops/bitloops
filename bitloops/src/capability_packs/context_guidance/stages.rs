use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRegistration, StageRequest,
    StageResponse,
};

use super::descriptor::{CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_STAGE_ID};
use super::storage::{
    ListSelectedContextGuidanceInput, PersistedGuidanceFact, PersistedGuidanceSource,
    PersistedGuidanceTarget, PersistedGuidanceTargetSummary,
};
use super::types::{GuidanceFactCategory, GuidanceFactConfidence};

pub fn build_context_guidance_stage() -> StageRegistration {
    StageRegistration::new(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        CONTEXT_GUIDANCE_STAGE_ID,
        std::sync::Arc::new(ContextGuidanceStageHandler),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextGuidanceStageItem {
    pub category: String,
    pub kind: String,
    pub confidence: String,
    pub generated_at: Option<String>,
    pub sources: Vec<ContextGuidanceStageSource>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextGuidanceStageSource {
    pub source_type: String,
}

pub fn build_context_guidance_summary(
    items: &[ContextGuidanceStageItem],
    target_summaries: &[PersistedGuidanceTargetSummary],
) -> Value {
    let mut category_counts = BTreeMap::<String, usize>::new();
    let mut kind_counts = BTreeMap::<String, usize>::new();
    let mut source_type_counts = BTreeMap::<String, usize>::new();
    let mut confidence_counts = BTreeMap::<String, usize>::new();
    let mut latest_generated_at: Option<String> = None;

    for item in items {
        *category_counts.entry(item.category.clone()).or_default() += 1;
        *kind_counts.entry(item.kind.clone()).or_default() += 1;
        *confidence_counts
            .entry(item.confidence.clone())
            .or_default() += 1;
        for source in &item.sources {
            *source_type_counts
                .entry(source.source_type.clone())
                .or_default() += 1;
        }
        if let Some(generated_at) = item
            .generated_at
            .as_deref()
            .map(normalize_datetime_for_graphql)
            && latest_generated_at
                .as_deref()
                .is_none_or(|latest| generated_at.as_str() > latest)
        {
            latest_generated_at = Some(generated_at);
        }
    }

    json!({
        "totalCount": items.len(),
        "categoryCounts": category_counts,
        "kindCounts": kind_counts,
        "sourceTypeCounts": source_type_counts,
        "confidenceCounts": confidence_counts,
        "latestGeneratedAt": latest_generated_at,
        "targetSummaries": target_summaries.iter().map(target_summary_json).collect::<Vec<_>>(),
        "expandHint": (!items.is_empty()).then(|| {
            json!({
                "intent": "Inspect context guidance for selected artefacts",
                "template": "bitloops devql query '{ selectArtefacts(by: { path: \"src/lib.rs\" }) { contextGuidance { overview items(first: 20) { category kind guidance evidenceExcerpt sources { sourceType checkpointId turnId } } } } }'"
            })
        }),
    })
}

struct ContextGuidanceStageHandler;

impl StageHandler for ContextGuidanceStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>> {
        Box::pin(async move {
            let Some(store) = ctx.context_guidance_store() else {
                bail!("context guidance store is not available");
            };
            let input = list_input_from_request(&request)?;
            let repo_id = input.repo_id.clone();
            let selected_targets = selected_targets_from_input(&input);
            let facts = store.list_selected_context_guidance(input)?;
            let items = facts.iter().map(stage_item_from_fact).collect::<Vec<_>>();
            let summary_items = facts.iter().map(summary_item_from_fact).collect::<Vec<_>>();
            let target_summaries = store.list_target_summaries(&repo_id, &selected_targets)?;
            let schema = (!items.is_empty()).then(|| "context_guidance.schema".to_string());
            Ok(StageResponse::json(json!({
                "overview": build_context_guidance_summary(&summary_items, &target_summaries),
                "schema": schema,
                "items": items,
            })))
        })
    }
}

fn list_input_from_request(request: &StageRequest) -> Result<ListSelectedContextGuidanceInput> {
    let repo_id = request
        .payload
        .get("query_context")
        .and_then(|ctx| ctx.get("repo_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("context guidance stage requires query_context.repo_id"))?
        .to_string();
    let input_rows = request
        .payload
        .get("input_rows")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("context guidance stage requires input_rows"))?;
    let args = request.payload.get("args").and_then(Value::as_object);
    let kind = optional_string_arg(args, "kind")?;
    if let Some(kind) = kind.as_deref()
        && kind.trim().is_empty()
    {
        bail!("`kind` must be non-empty");
    }

    Ok(ListSelectedContextGuidanceInput {
        repo_id,
        selected_paths: selected_values(input_rows, "path"),
        selected_symbol_ids: selected_values(input_rows, "symbol_id"),
        selected_symbol_fqns: selected_values(input_rows, "symbol_fqn"),
        agent: optional_string_arg(args, "agent")?,
        since: optional_string_arg(args, "since")?,
        evidence_kind: optional_string_arg(args, "evidenceKind")?,
        category: optional_string_arg(args, "category")?
            .as_deref()
            .map(parse_category_arg)
            .transpose()?,
        kind: kind.map(|value| value.trim().to_string()),
        limit: request.limit().unwrap_or(100).max(1),
    })
}

fn selected_targets_from_input(
    input: &ListSelectedContextGuidanceInput,
) -> Vec<PersistedGuidanceTarget> {
    let mut targets = Vec::new();
    targets.extend(
        input
            .selected_paths
            .iter()
            .map(|path| PersistedGuidanceTarget {
                target_type: "path".to_string(),
                target_value: path.clone(),
            }),
    );
    targets.extend(
        input
            .selected_symbol_ids
            .iter()
            .map(|symbol| PersistedGuidanceTarget {
                target_type: "symbol_id".to_string(),
                target_value: symbol.clone(),
            }),
    );
    targets.extend(
        input
            .selected_symbol_fqns
            .iter()
            .map(|symbol| PersistedGuidanceTarget {
                target_type: "symbol_fqn".to_string(),
                target_value: symbol.clone(),
            }),
    );
    targets
}

fn selected_values(input_rows: &[Value], key: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut values = Vec::new();
    for value in input_rows
        .iter()
        .filter_map(|row| row.get(key))
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if seen.insert(value.to_string()) {
            values.push(value.to_string());
        }
    }
    values
}

fn optional_string_arg(
    args: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> Result<Option<String>> {
    let Some(value) = args.and_then(|args| args.get(key)) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_str() else {
        bail!("`{key}` must be a string");
    };
    Ok(Some(value.trim().to_string()))
}

fn parse_category_arg(value: &str) -> Result<GuidanceFactCategory> {
    match value.trim().to_ascii_uppercase().as_str() {
        "DECISION" => Ok(GuidanceFactCategory::Decision),
        "CONSTRAINT" => Ok(GuidanceFactCategory::Constraint),
        "PATTERN" => Ok(GuidanceFactCategory::Pattern),
        "RISK" => Ok(GuidanceFactCategory::Risk),
        "VERIFICATION" => Ok(GuidanceFactCategory::Verification),
        "CONTEXT" => Ok(GuidanceFactCategory::Context),
        other => bail!("unsupported context guidance category `{other}`"),
    }
}

fn stage_item_from_fact(fact: &PersistedGuidanceFact) -> Value {
    let category = category_name(fact.category);
    let confidence = confidence_name(fact.confidence);
    json!({
        "id": fact.guidance_id,
        "category": category,
        "kind": fact.kind,
        "label": context_guidance_label(fact.category, fact.kind.as_str()),
        "guidance": fact.guidance,
        "evidenceExcerpt": fact.evidence_excerpt,
        "confidence": confidence,
        "relevanceScore": relevance_score(fact),
        "generatedAt": fact.generated_at.as_deref().map(normalize_datetime_for_graphql),
        "sourceModel": fact.source_model,
        "sourceCount": fact.sources.len() as i32,
        "sources": fact.sources.iter().map(source_json).collect::<Vec<_>>(),
    })
}

fn summary_item_from_fact(fact: &PersistedGuidanceFact) -> ContextGuidanceStageItem {
    ContextGuidanceStageItem {
        category: category_name(fact.category).to_string(),
        kind: fact.kind.clone(),
        confidence: confidence_name(fact.confidence).to_string(),
        generated_at: fact.generated_at.clone(),
        sources: fact
            .sources
            .iter()
            .map(|source| ContextGuidanceStageSource {
                source_type: source.source_type.clone(),
            })
            .collect(),
    }
}

fn source_json(source: &PersistedGuidanceSource) -> Value {
    json!({
        "sourceType": source.source_type,
        "sourceId": source.source_id,
        "checkpointId": source.checkpoint_id,
        "sessionId": source.session_id,
        "turnId": source.turn_id,
        "toolKind": source.tool_kind,
        "knowledgeItemId": source.knowledge_item_id,
        "knowledgeItemVersionId": source.knowledge_item_version_id,
        "relationAssertionId": source.relation_assertion_id,
        "provider": source.provider,
        "sourceKind": source.source_kind,
        "title": source.title,
        "url": source.url,
        "excerpt": source.excerpt,
    })
}

fn target_summary_json(summary: &PersistedGuidanceTargetSummary) -> Value {
    json!({
        "targetType": summary.target_type,
        "targetValue": summary.target_value,
        "summary": summary.summary_json,
        "activeGuidanceCount": summary.active_guidance_count,
        "generatedAt": summary.generated_at.as_deref().map(normalize_datetime_for_graphql),
    })
}

fn context_guidance_label(category: GuidanceFactCategory, kind: &str) -> String {
    let category = match category {
        GuidanceFactCategory::Decision => "Decision",
        GuidanceFactCategory::Constraint => "Constraint",
        GuidanceFactCategory::Pattern => "Pattern",
        GuidanceFactCategory::Risk => "Risk",
        GuidanceFactCategory::Verification => "Verification",
        GuidanceFactCategory::Context => "Context",
    };
    format!("{category}: {}", kind.replace('_', " "))
}

fn relevance_score(fact: &PersistedGuidanceFact) -> f64 {
    let has_symbol_target = fact
        .targets
        .iter()
        .any(|target| target.target_type == "symbol_id" || target.target_type == "symbol_fqn");
    super::quality::guidance_value_score(fact.category, fact.confidence, has_symbol_target)
}

fn category_name(category: GuidanceFactCategory) -> &'static str {
    match category {
        GuidanceFactCategory::Decision => "DECISION",
        GuidanceFactCategory::Constraint => "CONSTRAINT",
        GuidanceFactCategory::Pattern => "PATTERN",
        GuidanceFactCategory::Risk => "RISK",
        GuidanceFactCategory::Verification => "VERIFICATION",
        GuidanceFactCategory::Context => "CONTEXT",
    }
}

fn confidence_name(confidence: GuidanceFactConfidence) -> &'static str {
    match confidence {
        GuidanceFactConfidence::High => "HIGH",
        GuidanceFactConfidence::Medium => "MEDIUM",
        GuidanceFactConfidence::Low => "LOW",
    }
}

fn normalize_datetime_for_graphql(value: &str) -> String {
    if value.contains('T') {
        value.to_string()
    } else if value.len() == 19 && value.as_bytes().get(10) == Some(&b' ') {
        format!("{}T{}Z", &value[..10], &value[11..])
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use anyhow::{Result, bail};
    use serde_json::json;

    use super::*;
    use crate::capability_packs::context_guidance::storage::{
        ContextGuidanceRepository, PersistGuidanceOutcome, PersistedGuidanceSource,
        PersistedGuidanceTarget,
    };
    use crate::host::capability_host::gateways::{CanonicalGraphGateway, RelationalGateway};
    use crate::host::capability_host::{CapabilityExecutionContext, StageHandler};
    use crate::host::devql::RepoIdentity;

    struct FakeStore {
        rows: Vec<PersistedGuidanceFact>,
        target_summaries: Vec<PersistedGuidanceTargetSummary>,
        last_input: Mutex<Option<ListSelectedContextGuidanceInput>>,
    }

    impl ContextGuidanceRepository for FakeStore {
        fn persist_history_guidance_distillation(
            &self,
            _repo_id: &str,
            _input: &crate::capability_packs::context_guidance::distillation::GuidanceDistillationInput,
            _output: &crate::capability_packs::context_guidance::types::GuidanceDistillationOutput,
            _source_model: Option<&str>,
            _source_profile: Option<&str>,
        ) -> Result<PersistGuidanceOutcome> {
            bail!("not used")
        }

        fn persist_knowledge_guidance_distillation(
            &self,
            _repo_id: &str,
            _input: &crate::capability_packs::context_guidance::distillation::KnowledgeGuidanceDistillationInput,
            _output: &crate::capability_packs::context_guidance::types::GuidanceDistillationOutput,
            _source_model: Option<&str>,
            _source_profile: Option<&str>,
        ) -> Result<PersistGuidanceOutcome> {
            bail!("not used")
        }

        fn list_selected_context_guidance(
            &self,
            input: ListSelectedContextGuidanceInput,
        ) -> Result<Vec<PersistedGuidanceFact>> {
            *self.last_input.lock().expect("lock") = Some(input);
            Ok(self.rows.clone())
        }

        fn list_active_guidance_for_target(
            &self,
            _repo_id: &str,
            _target_type: &str,
            _target_value: &str,
            _limit: usize,
        ) -> Result<Vec<PersistedGuidanceFact>> {
            bail!("not used")
        }

        fn apply_target_compaction(
            &self,
            _repo_id: &str,
            _input: crate::capability_packs::context_guidance::storage::ApplyTargetCompactionInput,
        ) -> Result<crate::capability_packs::context_guidance::storage::ApplyTargetCompactionOutcome>
        {
            bail!("not used")
        }

        fn list_target_summaries(
            &self,
            _repo_id: &str,
            _targets: &[PersistedGuidanceTarget],
        ) -> Result<Vec<PersistedGuidanceTargetSummary>> {
            Ok(self.target_summaries.clone())
        }

        fn health_check(&self, _repo_id: &str) -> Result<()> {
            Ok(())
        }
    }

    struct DummyCtx<'a> {
        repo: RepoIdentity,
        store: Option<&'a dyn ContextGuidanceRepository>,
    }

    impl CapabilityExecutionContext for DummyCtx<'_> {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            Path::new(".")
        }

        fn graph(&self) -> &dyn CanonicalGraphGateway {
            panic!("graph is not used")
        }

        fn host_relational(&self) -> &dyn RelationalGateway {
            panic!("relational is not used")
        }

        fn context_guidance_store(&self) -> Option<&dyn ContextGuidanceRepository> {
            self.store
        }
    }

    fn repo() -> RepoIdentity {
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        }
    }

    fn fact() -> PersistedGuidanceFact {
        PersistedGuidanceFact {
            guidance_id: "guidance-1".to_string(),
            run_id: "run-1".to_string(),
            repo_id: "repo-1".to_string(),
            active: true,
            category: GuidanceFactCategory::Decision,
            kind: "rejected_approach".to_string(),
            guidance: "Avoid the rejected approach.".to_string(),
            evidence_excerpt: "Rejected in prior session.".to_string(),
            confidence: GuidanceFactConfidence::High,
            lifecycle_status: "active".to_string(),
            fact_fingerprint: "fingerprint-1".to_string(),
            value_score: 0.99,
            superseded_by_guidance_id: None,
            source_model: Some("model".to_string()),
            generated_at: Some("2026-04-29T10:00:00Z".to_string()),
            targets: vec![PersistedGuidanceTarget {
                target_type: "path".to_string(),
                target_value: "src/lib.rs".to_string(),
            }],
            sources: vec![PersistedGuidanceSource {
                source_type: "history.turn".to_string(),
                source_id: "session-1:turn-1".to_string(),
                checkpoint_id: Some("checkpoint-1".to_string()),
                session_id: Some("session-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                tool_invocation_id: None,
                tool_kind: None,
                event_time: Some("2026-04-29T10:00:00Z".to_string()),
                agent_type: Some("codex".to_string()),
                model: Some("gpt-5.4".to_string()),
                evidence_kind: Some("FILE_RELATION".to_string()),
                match_strength: Some("HIGH".to_string()),
                knowledge_item_id: None,
                knowledge_item_version_id: None,
                relation_assertion_id: None,
                provider: None,
                source_kind: None,
                title: None,
                url: None,
                excerpt: Some("Rejected in prior session.".to_string()),
            }],
        }
    }

    #[test]
    fn context_guidance_summary_counts_items() {
        let summary = build_context_guidance_summary(
            &[ContextGuidanceStageItem {
                category: "DECISION".to_string(),
                kind: "rejected_approach".to_string(),
                confidence: "HIGH".to_string(),
                generated_at: Some("2026-04-29T10:00:00Z".to_string()),
                sources: vec![ContextGuidanceStageSource {
                    source_type: "history.turn".to_string(),
                }],
            }],
            &[],
        );

        assert_eq!(summary["totalCount"], json!(1));
        assert_eq!(summary["categoryCounts"]["DECISION"], json!(1));
        assert_eq!(summary["kindCounts"]["rejected_approach"], json!(1));
        assert_eq!(summary["sourceTypeCounts"]["history.turn"], json!(1));
        assert_eq!(summary["confidenceCounts"]["HIGH"], json!(1));
        assert_eq!(summary["latestGeneratedAt"], json!("2026-04-29T10:00:00Z"));
        assert!(
            summary["expandHint"]["template"]
                .as_str()
                .unwrap()
                .contains("contextGuidance")
        );
    }

    #[test]
    fn context_guidance_summary_normalizes_latest_generated_at_before_comparing() {
        let summary = build_context_guidance_summary(
            &[
                ContextGuidanceStageItem {
                    category: "DECISION".to_string(),
                    kind: "older_iso".to_string(),
                    confidence: "HIGH".to_string(),
                    generated_at: Some("2026-04-29T10:00:00Z".to_string()),
                    sources: Vec::new(),
                },
                ContextGuidanceStageItem {
                    category: "DECISION".to_string(),
                    kind: "newer_sqlite".to_string(),
                    confidence: "HIGH".to_string(),
                    generated_at: Some("2026-04-29 11:00:00".to_string()),
                    sources: Vec::new(),
                },
                ContextGuidanceStageItem {
                    category: "DECISION".to_string(),
                    kind: "missing_time".to_string(),
                    confidence: "LOW".to_string(),
                    generated_at: None,
                    sources: Vec::new(),
                },
            ],
            &[],
        );

        assert_eq!(summary["latestGeneratedAt"], json!("2026-04-29T11:00:00Z"));
    }

    #[test]
    fn selected_values_deduplicates_trimmed_values() {
        let rows = vec![
            json!({ "path": " src/lib.rs " }),
            json!({ "path": "src/lib.rs" }),
            json!({ "path": "src/other.rs" }),
        ];

        assert_eq!(
            selected_values(&rows, "path"),
            vec!["src/lib.rs".to_string(), "src/other.rs".to_string()]
        );
    }

    #[test]
    fn relevance_score_reflects_category_value() {
        let mut decision = fact();
        decision.category = GuidanceFactCategory::Decision;
        decision.confidence = GuidanceFactConfidence::High;

        let mut verification = fact();
        verification.category = GuidanceFactCategory::Verification;
        verification.confidence = GuidanceFactConfidence::High;

        assert!(relevance_score(&decision) > relevance_score(&verification));
    }

    #[tokio::test]
    async fn stage_returns_empty_payload_when_store_has_no_rows() {
        let store = FakeStore {
            rows: Vec::new(),
            target_summaries: Vec::new(),
            last_input: Mutex::new(None),
        };
        let mut ctx = DummyCtx {
            repo: repo(),
            store: Some(&store),
        };
        let handler = ContextGuidanceStageHandler;
        let response = handler
            .execute(
                StageRequest::new(json!({
                    "input_rows": [],
                    "args": {},
                    "limit": 100,
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await
            .expect("stage");

        assert_eq!(response.payload["overview"]["totalCount"], json!(0));
        assert_eq!(response.payload["schema"], Value::Null);
        assert_eq!(response.payload["items"], json!([]));
    }

    #[tokio::test]
    async fn stage_rejects_empty_kind() {
        let store = FakeStore {
            rows: Vec::new(),
            target_summaries: Vec::new(),
            last_input: Mutex::new(None),
        };
        let mut ctx = DummyCtx {
            repo: repo(),
            store: Some(&store),
        };
        let handler = ContextGuidanceStageHandler;

        let err = handler
            .execute(
                StageRequest::new(json!({
                    "input_rows": [],
                    "args": { "kind": "   " },
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await
            .expect_err("empty kind should fail");

        assert!(err.to_string().contains("`kind` must be non-empty"));
    }

    #[tokio::test]
    async fn stage_passes_selected_targets_to_repository() {
        let store = FakeStore {
            rows: vec![fact()],
            target_summaries: Vec::new(),
            last_input: Mutex::new(None),
        };
        let mut ctx = DummyCtx {
            repo: repo(),
            store: Some(&store),
        };
        let handler = ContextGuidanceStageHandler;
        let response = handler
            .execute(
                StageRequest::new(json!({
                    "input_rows": [{
                        "path": "src/lib.rs",
                        "symbol_id": "symbol-1",
                        "symbol_fqn": "src/lib.rs::target"
                    }],
                    "args": { "category": "DECISION", "kind": "rejected_approach" },
                    "limit": 10,
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await
            .expect("stage");

        assert_eq!(response.payload["items"][0]["sourceModel"], json!("model"));
        let input = store
            .last_input
            .lock()
            .expect("lock")
            .take()
            .expect("input");
        assert_eq!(input.selected_paths, vec!["src/lib.rs".to_string()]);
        assert_eq!(input.selected_symbol_ids, vec!["symbol-1".to_string()]);
        assert_eq!(
            input.selected_symbol_fqns,
            vec!["src/lib.rs::target".to_string()]
        );
        assert_eq!(input.kind.as_deref(), Some("rejected_approach"));
    }

    #[tokio::test]
    async fn stage_returns_knowledge_source_provenance() {
        let mut fact = fact();
        let source = &mut fact.sources[0];
        source.source_type = "knowledge.item_version".to_string();
        source.source_id = "item-1:version-1:relation-1".to_string();
        source.knowledge_item_id = Some("item-1".to_string());
        source.knowledge_item_version_id = Some("version-1".to_string());
        source.relation_assertion_id = Some("relation-1".to_string());
        source.provider = Some("github".to_string());
        source.source_kind = Some("github_issue".to_string());
        source.title = Some("Issue title".to_string());
        source.url = Some("https://github.com/org/repo/issues/1".to_string());
        let store = FakeStore {
            rows: vec![fact],
            target_summaries: Vec::new(),
            last_input: Mutex::new(None),
        };
        let mut ctx = DummyCtx {
            repo: repo(),
            store: Some(&store),
        };
        let handler = ContextGuidanceStageHandler;
        let response = handler
            .execute(
                StageRequest::new(json!({
                    "input_rows": [{ "path": "src/lib.rs" }],
                    "args": {},
                    "limit": 10,
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await
            .expect("stage");

        let source = &response.payload["items"][0]["sources"][0];
        assert_eq!(source["sourceType"], json!("knowledge.item_version"));
        assert_eq!(source["knowledgeItemId"], json!("item-1"));
        assert_eq!(source["knowledgeItemVersionId"], json!("version-1"));
        assert_eq!(source["relationAssertionId"], json!("relation-1"));
        assert_eq!(source["provider"], json!("github"));
        assert_eq!(source["sourceKind"], json!("github_issue"));
    }

    #[tokio::test]
    async fn stage_overview_includes_target_summaries_when_available() {
        let store = FakeStore {
            rows: vec![fact()],
            target_summaries: vec![PersistedGuidanceTargetSummary {
                target_type: "path".to_string(),
                target_value: "src/lib.rs".to_string(),
                summary_json: json!({ "summary": "Keep parser boundary guidance." }),
                active_guidance_count: 4,
                generated_at: Some("2026-04-30 10:00:00".to_string()),
            }],
            last_input: Mutex::new(None),
        };
        let mut ctx = DummyCtx {
            repo: repo(),
            store: Some(&store),
        };
        let handler = ContextGuidanceStageHandler;
        let response = handler
            .execute(
                StageRequest::new(json!({
                    "input_rows": [{ "path": "src/lib.rs" }],
                    "args": {},
                    "limit": 10,
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await
            .expect("stage");

        assert_eq!(
            response.payload["overview"]["targetSummaries"][0]["targetValue"],
            json!("src/lib.rs")
        );
        assert_eq!(
            response.payload["overview"]["targetSummaries"][0]["summary"]["summary"],
            json!("Keep parser boundary guidance.")
        );
        assert_eq!(
            response.payload["overview"]["targetSummaries"][0]["generatedAt"],
            json!("2026-04-30T10:00:00Z")
        );
    }

    #[tokio::test]
    async fn stage_fails_when_store_is_unavailable() {
        let mut ctx = DummyCtx {
            repo: repo(),
            store: None,
        };
        let handler = ContextGuidanceStageHandler;
        let err = handler
            .execute(
                StageRequest::new(json!({
                    "input_rows": [],
                    "args": {},
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await
            .expect_err("missing store should fail");

        assert!(
            err.to_string()
                .contains("context guidance store is not available")
        );
    }
}
