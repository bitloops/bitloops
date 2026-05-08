use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};
use crate::host::devql::{RelationalStorage, deterministic_uuid};
use crate::host::language_adapter::{
    LanguageHttpFact, LanguageHttpFactArtefact, LanguageHttpFactFile,
};
use crate::models::{CurrentCanonicalArtefactRecord, CurrentCanonicalFileRecord};

use super::types::{
    HTTP_BUNDLE_CONTENT_LENGTH_LOSS_BEFORE_WIRE_SERIALISATION, HTTP_CAPABILITY_ID,
    HTTP_CONSUMER_ID, HTTP_OWNER, HTTP_PRIMITIVE_BEHAVIOUR_INVARIANT,
    HTTP_PRIMITIVE_DERIVED_VALUE_RULE, HTTP_PRIMITIVE_HEADER_SEMANTIC,
    HTTP_PRIMITIVE_LIFECYCLE_PHASE_RULE, HTTP_RISK_CONTENT_LENGTH_LOSS,
    HTTP_ROLE_BODY_EXACT_SIZE_SIGNAL, HTTP_ROLE_BODY_REPLACEMENT, HTTP_ROLE_BODY_STRIPPING,
    HTTP_ROLE_CONTENT_LENGTH_HEADER, HTTP_ROLE_FRAMEWORK_RUNTIME_BOUNDARY,
    HTTP_ROLE_GET_EQUIVALENT_HEADERS, HTTP_ROLE_HEAD_METHOD, HTTP_ROLE_HEADER_DERIVATION,
    HTTP_ROLE_WIRE_SERIALISATION_BOUNDARY,
};

const HTTP_PROTOCOL_SEED_FINGERPRINT: &str = "http-protocol-v1";
const HTTP_BUNDLE_KIND_CAUSAL_RISK: &str = "CAUSAL_RISK";

mod persist;

#[cfg(test)]
mod tests;

pub struct HttpCurrentStateConsumer;

impl CurrentStateConsumer for HttpCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        HTTP_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        HTTP_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let upstream_batches = collect_language_http_fact_batches(request, context)
                .context("collecting language-owned HTTP facts")?;
            let outcome = reconcile_http_current_state(&context.storage, request, upstream_batches)
                .await
                .context("reconciling HTTP current state")?;
            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings: Vec::new(),
                metrics: Some(json!({
                    "protocol_primitives_seeded": outcome.protocol_primitives_seeded,
                    "source_primitives": outcome.source_primitives,
                    "bundles": outcome.bundles,
                    "query_index_rows": outcome.query_index_rows,
                    "reconcile_mode": format!("{:?}", request.reconcile_mode),
                })),
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HttpReconcileOutcome {
    pub protocol_primitives_seeded: usize,
    pub source_primitives: usize,
    pub bundles: usize,
    pub query_index_rows: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct HttpPrimitiveFact {
    repo_id: String,
    primitive_id: String,
    owner: String,
    primitive_type: String,
    subject: String,
    roles: Vec<String>,
    terms: Vec<String>,
    properties: Value,
    confidence_level: String,
    confidence_score: f64,
    status: String,
    input_fingerprint: String,
    evidence: Vec<HttpEvidenceFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpEvidenceFact {
    evidence_id: String,
    kind: String,
    path: Option<String>,
    artefact_id: Option<String>,
    symbol_id: Option<String>,
    content_id: Option<String>,
    start_line: Option<i64>,
    end_line: Option<i64>,
    start_byte: Option<i64>,
    end_byte: Option<i64>,
    dependency_package: Option<String>,
    dependency_version: Option<String>,
    source_url: Option<String>,
    excerpt_hash: Option<String>,
    producer: Option<String>,
    model: Option<String>,
    prompt_hash: Option<String>,
    properties: Value,
}

#[derive(Debug, Clone, PartialEq)]
struct HttpBundleFact {
    repo_id: String,
    bundle_id: String,
    bundle_kind: String,
    risk_kind: Option<String>,
    severity: Option<String>,
    matched_roles: Vec<String>,
    primitive_ids: Vec<String>,
    upstream_facts: Value,
    causal_chain: Value,
    invalidated_assumptions: Value,
    obligations: Value,
    confidence_level: String,
    confidence_score: f64,
    status: String,
    input_fingerprint: String,
    path: Option<String>,
    artefact_id: Option<String>,
    symbol_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct HttpQueryIndexRow {
    repo_id: String,
    owner: String,
    fact_id: String,
    bundle_id: Option<String>,
    terms: Vec<String>,
    roles: Vec<String>,
    subject: String,
    path: Option<String>,
    symbol_id: Option<String>,
    artefact_id: Option<String>,
    rank_signals: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UpstreamHttpFactBatch {
    owner: String,
    path: String,
    facts: Vec<LanguageHttpFact>,
}

pub(crate) async fn reconcile_http_current_state(
    relational: &RelationalStorage,
    request: &CurrentStateConsumerRequest,
    upstream_batches: Vec<UpstreamHttpFactBatch>,
) -> Result<HttpReconcileOutcome> {
    let protocol_primitives = protocol_primitives_for_repo(&request.repo_id);
    let mut seed_statements = persist::stale_removed_source_fact_statements(request, relational);
    seed_statements.extend(persist::replace_upstream_fact_statements(
        relational,
        &request.repo_id,
        &upstream_batches,
    ));
    seed_statements.extend(persist::replace_protocol_seed_statements(
        relational,
        &request.repo_id,
        &protocol_primitives,
    ));
    relational
        .exec_serialized_batch_transactional(&seed_statements)
        .await
        .context("seeding HTTP protocol primitives")?;

    let primitives = persist::load_active_http_primitives(relational, &request.repo_id).await?;
    let bundles = compose_http_bundles(&request.repo_id, &primitives);
    let index_rows = build_query_index_rows(&primitives, &bundles);

    let mut statements = vec![
        format!(
            "DELETE FROM http_bundles_current WHERE repo_id = {};",
            persist::sql_text(&request.repo_id)
        ),
        format!(
            "DELETE FROM http_query_index_current WHERE repo_id = {};",
            persist::sql_text(&request.repo_id)
        ),
    ];
    for bundle in &bundles {
        statements.push(persist::insert_bundle_sql(relational, bundle));
    }
    for row in &index_rows {
        statements.push(persist::insert_query_index_row_sql(relational, row));
    }
    statements.push(persist::insert_run_sql(
        relational,
        &request.repo_id,
        request.to_generation_seq_inclusive,
        &json!({
            "protocol_primitives_seeded": protocol_primitives.len(),
            "source_primitives": primitives.len(),
            "bundles": bundles.len(),
            "query_index_rows": index_rows.len(),
        }),
    ));
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
        .context("refreshing HTTP bundles and query index")?;

    Ok(HttpReconcileOutcome {
        protocol_primitives_seeded: protocol_primitives.len(),
        source_primitives: primitives.len(),
        bundles: bundles.len(),
        query_index_rows: index_rows.len(),
    })
}

fn collect_language_http_fact_batches(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> Result<Vec<UpstreamHttpFactBatch>> {
    let files = context
        .relational
        .load_current_canonical_files(&request.repo_id)
        .context("loading current files for HTTP language facts")?;
    let artefacts = context
        .relational
        .load_current_canonical_artefacts(&request.repo_id)
        .context("loading current artefacts for HTTP language facts")?;
    let artefacts_by_path = artefacts_by_path(&artefacts);
    let mut batches = Vec::new();

    for file in files.iter().filter(|file| file.analysis_mode == "code") {
        let absolute_path = request.repo_root.join(&file.path);
        let Ok(content) = fs::read_to_string(&absolute_path) else {
            continue;
        };
        let http_file = language_http_file(file);
        let http_artefacts = artefacts_by_path
            .get(&file.path)
            .map(|artefacts| {
                artefacts
                    .iter()
                    .filter_map(|artefact| language_http_artefact(artefact))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some((owner, facts)) =
            context
                .language_services
                .http_facts_for_file(&http_file, &content, &http_artefacts)
        {
            batches.push(UpstreamHttpFactBatch {
                owner,
                path: file.path.clone(),
                facts,
            });
        }
    }

    Ok(batches)
}

fn artefacts_by_path(
    artefacts: &[CurrentCanonicalArtefactRecord],
) -> BTreeMap<String, Vec<&CurrentCanonicalArtefactRecord>> {
    let mut grouped = BTreeMap::<String, Vec<&CurrentCanonicalArtefactRecord>>::new();
    for artefact in artefacts {
        grouped
            .entry(artefact.path.clone())
            .or_default()
            .push(artefact);
    }
    grouped
}

fn language_http_file(file: &CurrentCanonicalFileRecord) -> LanguageHttpFactFile {
    LanguageHttpFactFile {
        repo_id: file.repo_id.clone(),
        path: file.path.clone(),
        language: if file.resolved_language.is_empty() {
            file.language.clone()
        } else {
            file.resolved_language.clone()
        },
        content_id: file.effective_content_id.clone(),
        parser_version: file.parser_version.clone(),
        extractor_version: file.extractor_version.clone(),
    }
}

fn language_http_artefact(
    artefact: &CurrentCanonicalArtefactRecord,
) -> Option<LanguageHttpFactArtefact> {
    Some(LanguageHttpFactArtefact {
        symbol_id: artefact.symbol_id.clone(),
        artefact_id: artefact.artefact_id.clone(),
        symbol_fqn: artefact.symbol_fqn.clone()?,
        canonical_kind: artefact.canonical_kind.clone(),
        language_kind: artefact.language_kind.clone().unwrap_or_default(),
        start_line: i32::try_from(artefact.start_line).ok()?,
        end_line: i32::try_from(artefact.end_line).ok()?,
        start_byte: i32::try_from(artefact.start_byte).ok()?,
        end_byte: i32::try_from(artefact.end_byte).ok()?,
        signature: artefact.signature.clone(),
    })
}

fn protocol_primitives_for_repo(repo_id: &str) -> Vec<HttpPrimitiveFact> {
    let seed_specs = [
        ProtocolSeedSpec {
            key: "head_get_header_equivalence",
            primitive_type: HTTP_PRIMITIVE_BEHAVIOUR_INVARIANT,
            subject: "HEAD responses preserve GET-equivalent response headers while omitting the response body",
            roles: &[HTTP_ROLE_HEAD_METHOD, HTTP_ROLE_GET_EQUIVALENT_HEADERS],
            terms: &["HEAD", "GET", "headers", "header equivalence"],
            properties: json!({
                "scope": "protocol",
                "behaviour": "head_get_header_equivalence",
                "sourceCategory": "curated_protocol_knowledge",
            }),
        },
        ProtocolSeedSpec {
            key: "content_length_header_semantic",
            primitive_type: HTTP_PRIMITIVE_HEADER_SEMANTIC,
            subject: "Content-Length represents the selected HTTP representation body length",
            roles: &[HTTP_ROLE_CONTENT_LENGTH_HEADER],
            terms: &["Content-Length", "headers", "body length"],
            properties: json!({
                "scope": "protocol",
                "headerName": "content-length",
                "sourceCategory": "curated_protocol_knowledge",
            }),
        },
        ProtocolSeedSpec {
            key: "content_length_exact_size_derivation",
            primitive_type: HTTP_PRIMITIVE_DERIVED_VALUE_RULE,
            subject: "Content-Length may be derived from an exact response body size signal before wire serialisation",
            roles: &[
                HTTP_ROLE_CONTENT_LENGTH_HEADER,
                HTTP_ROLE_HEADER_DERIVATION,
                HTTP_ROLE_BODY_EXACT_SIZE_SIGNAL,
            ],
            terms: &[
                "Content-Length",
                "body size",
                "exact size",
                "size hint",
                "header derivation",
            ],
            properties: json!({
                "scope": "protocol",
                "producerKind": "HEADER_DERIVATION",
                "sourceSignal": "exact_body_size",
                "phase": "before_wire_serialisation",
                "sourceCategory": "curated_protocol_knowledge",
            }),
        },
        ProtocolSeedSpec {
            key: "framework_runtime_serialisation_boundary",
            primitive_type: HTTP_PRIMITIVE_LIFECYCLE_PHASE_RULE,
            subject: "Framework response construction can precede runtime wire serialisation and header derivation",
            roles: &[
                HTTP_ROLE_HEADER_DERIVATION,
                HTTP_ROLE_WIRE_SERIALISATION_BOUNDARY,
                HTTP_ROLE_FRAMEWORK_RUNTIME_BOUNDARY,
            ],
            terms: &[
                "framework",
                "runtime",
                "wire serialisation",
                "serialisation",
                "header derivation",
            ],
            properties: json!({
                "scope": "protocol",
                "phase": "framework_to_runtime_serialisation",
                "sourceCategory": "curated_protocol_knowledge",
            }),
        },
    ];

    seed_specs
        .into_iter()
        .map(|spec| {
            let primitive_id = http_primitive_id(repo_id, spec.key);
            HttpPrimitiveFact {
                repo_id: repo_id.to_string(),
                primitive_id: primitive_id.clone(),
                owner: HTTP_OWNER.to_string(),
                primitive_type: spec.primitive_type.to_string(),
                subject: spec.subject.to_string(),
                roles: spec.roles.iter().map(|role| role.to_string()).collect(),
                terms: spec.terms.iter().map(|term| term.to_string()).collect(),
                properties: spec.properties,
                confidence_level: "HIGH".to_string(),
                confidence_score: 0.95,
                status: "active".to_string(),
                input_fingerprint: HTTP_PROTOCOL_SEED_FINGERPRINT.to_string(),
                evidence: vec![HttpEvidenceFact {
                    evidence_id: http_evidence_id(repo_id, spec.key),
                    kind: "curated_protocol_knowledge".to_string(),
                    path: None,
                    artefact_id: None,
                    symbol_id: None,
                    content_id: None,
                    start_line: None,
                    end_line: None,
                    start_byte: None,
                    end_byte: None,
                    dependency_package: None,
                    dependency_version: None,
                    source_url: None,
                    excerpt_hash: None,
                    producer: Some(HTTP_CAPABILITY_ID.to_string()),
                    model: None,
                    prompt_hash: None,
                    properties: json!({
                        "packVersion": HTTP_PROTOCOL_SEED_FINGERPRINT,
                        "sourceCategory": "curated_protocol_knowledge",
                    }),
                }],
            }
        })
        .collect()
}

fn compose_http_bundles(repo_id: &str, primitives: &[HttpPrimitiveFact]) -> Vec<HttpBundleFact> {
    let Some(head_headers) = first_with_any_role(
        primitives,
        &[HTTP_ROLE_HEAD_METHOD, HTTP_ROLE_GET_EQUIVALENT_HEADERS],
    ) else {
        return Vec::new();
    };
    let Some(header_rule) = first_with_any_role(
        primitives,
        &[HTTP_ROLE_CONTENT_LENGTH_HEADER, HTTP_ROLE_HEADER_DERIVATION],
    ) else {
        return Vec::new();
    };
    let Some(exact_size) = first_with_role(primitives, HTTP_ROLE_BODY_EXACT_SIZE_SIGNAL) else {
        return Vec::new();
    };
    let Some(body_replacement) = first_with_any_role(
        primitives,
        &[HTTP_ROLE_BODY_REPLACEMENT, HTTP_ROLE_BODY_STRIPPING],
    ) else {
        return Vec::new();
    };
    let Some(boundary) = first_with_any_role(
        primitives,
        &[
            HTTP_ROLE_WIRE_SERIALISATION_BOUNDARY,
            HTTP_ROLE_FRAMEWORK_RUNTIME_BOUNDARY,
        ],
    ) else {
        return Vec::new();
    };

    let selected = dedup_primitive_refs([
        head_headers,
        header_rule,
        exact_size,
        body_replacement,
        boundary,
    ]);
    let matched_roles = selected
        .iter()
        .flat_map(|primitive| primitive.roles.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let primitive_ids = selected
        .iter()
        .map(|primitive| primitive.primitive_id.clone())
        .collect::<Vec<_>>();
    let input_fingerprint = deterministic_uuid(&format!(
        "http|bundle-input|{repo_id}|{}|{}",
        HTTP_BUNDLE_CONTENT_LENGTH_LOSS_BEFORE_WIRE_SERIALISATION,
        primitive_ids.join("|")
    ));
    let (path, artefact_id, symbol_id) = preferred_anchor(&selected);
    let confidence_score = selected
        .iter()
        .map(|primitive| primitive.confidence_score)
        .fold(1.0_f64, f64::min)
        .min(0.92);

    vec![HttpBundleFact {
        repo_id: repo_id.to_string(),
        bundle_id: HTTP_BUNDLE_CONTENT_LENGTH_LOSS_BEFORE_WIRE_SERIALISATION.to_string(),
        bundle_kind: HTTP_BUNDLE_KIND_CAUSAL_RISK.to_string(),
        risk_kind: Some(HTTP_RISK_CONTENT_LENGTH_LOSS.to_string()),
        severity: Some("HIGH".to_string()),
        matched_roles,
        primitive_ids: primitive_ids.clone(),
        upstream_facts: upstream_facts_json(&selected),
        causal_chain: causal_chain_json(&selected),
        invalidated_assumptions: json!([{
            "id": "http.assumption.body_replacement_preserves_header_inputs",
            "assumption": "Body replacement preserves all signals needed by later HTTP header derivation",
            "invalidatedByPrimitiveIds": primitive_ids,
            "scope": "response_pipeline",
        }]),
        obligations: json!([{
            "id": "http.obligation.propagate_exact_body_length_before_replacement",
            "requiredFollowUp": "Preserve exact response-body size metadata before replacing a body that can still affect header derivation",
            "targetSymbols": selected
                .iter()
                .flat_map(|primitive| primitive.evidence.iter())
                .filter_map(|evidence| evidence.symbol_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            "blocking": true,
        }]),
        confidence_level: confidence_level(confidence_score).to_string(),
        confidence_score,
        status: "active".to_string(),
        input_fingerprint,
        path,
        artefact_id,
        symbol_id,
    }]
}

fn build_query_index_rows(
    primitives: &[HttpPrimitiveFact],
    bundles: &[HttpBundleFact],
) -> Vec<HttpQueryIndexRow> {
    let mut rows = Vec::with_capacity(primitives.len() + bundles.len());
    for primitive in primitives {
        let anchor = primitive.evidence.first();
        rows.push(HttpQueryIndexRow {
            repo_id: primitive.repo_id.clone(),
            owner: primitive.owner.clone(),
            fact_id: primitive.primitive_id.clone(),
            bundle_id: None,
            terms: primitive.terms.clone(),
            roles: primitive.roles.clone(),
            subject: primitive.subject.clone(),
            path: anchor.and_then(|evidence| evidence.path.clone()),
            symbol_id: anchor.and_then(|evidence| evidence.symbol_id.clone()),
            artefact_id: anchor.and_then(|evidence| evidence.artefact_id.clone()),
            rank_signals: json!({
                "kind": "primitive",
                "primitiveType": primitive.primitive_type,
                "confidenceScore": primitive.confidence_score,
                "status": primitive.status,
                "hasEvidenceAnchor": anchor.is_some_and(|evidence| evidence.path.is_some() || evidence.symbol_id.is_some() || evidence.artefact_id.is_some()),
            }),
        });
    }
    for bundle in bundles {
        rows.push(HttpQueryIndexRow {
            repo_id: bundle.repo_id.clone(),
            owner: HTTP_OWNER.to_string(),
            fact_id: bundle.bundle_id.clone(),
            bundle_id: Some(bundle.bundle_id.clone()),
            terms: vec![
                "HEAD".to_string(),
                "Content-Length".to_string(),
                "body replacement".to_string(),
                "body stripping".to_string(),
                "body size".to_string(),
                "wire serialisation".to_string(),
                "framework runtime".to_string(),
            ],
            roles: bundle.matched_roles.clone(),
            subject: bundle
                .risk_kind
                .clone()
                .unwrap_or_else(|| bundle.bundle_kind.clone()),
            path: bundle.path.clone(),
            symbol_id: bundle.symbol_id.clone(),
            artefact_id: bundle.artefact_id.clone(),
            rank_signals: json!({
                "kind": "bundle",
                "riskKind": bundle.risk_kind,
                "severity": bundle.severity,
                "confidenceScore": bundle.confidence_score,
                "matchedRoleCount": bundle.matched_roles.len(),
            }),
        });
    }
    rows
}

struct ProtocolSeedSpec {
    key: &'static str,
    primitive_type: &'static str,
    subject: &'static str,
    roles: &'static [&'static str],
    terms: &'static [&'static str],
    properties: Value,
}

fn first_with_role<'a>(
    primitives: &'a [HttpPrimitiveFact],
    role: &str,
) -> Option<&'a HttpPrimitiveFact> {
    first_with_any_role(primitives, &[role])
}

fn first_with_any_role<'a>(
    primitives: &'a [HttpPrimitiveFact],
    roles: &[&str],
) -> Option<&'a HttpPrimitiveFact> {
    primitives
        .iter()
        .filter(|primitive| primitive.status != "stale")
        .find(|primitive| {
            roles
                .iter()
                .any(|role| primitive.roles.iter().any(|candidate| candidate == role))
        })
}

fn dedup_primitive_refs(primitives: [&HttpPrimitiveFact; 5]) -> Vec<&HttpPrimitiveFact> {
    let mut seen = BTreeSet::new();
    primitives
        .into_iter()
        .filter(|primitive| seen.insert(primitive.primitive_id.clone()))
        .collect()
}

fn upstream_facts_json(primitives: &[&HttpPrimitiveFact]) -> Value {
    Value::Array(
        primitives
            .iter()
            .filter(|primitive| primitive.owner != HTTP_OWNER)
            .map(|primitive| {
                json!({
                    "owner": primitive.owner,
                    "factId": primitive.primitive_id,
                    "primitiveType": primitive.primitive_type,
                    "subject": primitive.subject,
                    "roles": primitive.roles,
                })
            })
            .collect(),
    )
}

fn causal_chain_json(primitives: &[&HttpPrimitiveFact]) -> Value {
    let mut links = Vec::new();
    for primitive in primitives {
        for role in &primitive.roles {
            links.push(json!({
                "owner": primitive.owner,
                "factId": primitive.primitive_id,
                "role": role,
                "primitiveType": primitive.primitive_type,
                "subject": primitive.subject,
            }));
        }
    }
    Value::Array(links)
}

fn preferred_anchor(
    primitives: &[&HttpPrimitiveFact],
) -> (Option<String>, Option<String>, Option<String>) {
    let evidence = primitives
        .iter()
        .filter(|primitive| primitive.owner != HTTP_OWNER)
        .flat_map(|primitive| primitive.evidence.iter())
        .find(|evidence| {
            evidence.path.is_some()
                || evidence.artefact_id.is_some()
                || evidence.symbol_id.is_some()
        })
        .or_else(|| {
            primitives
                .iter()
                .flat_map(|primitive| primitive.evidence.iter())
                .find(|evidence| {
                    evidence.path.is_some()
                        || evidence.artefact_id.is_some()
                        || evidence.symbol_id.is_some()
                })
        });
    evidence
        .map(|evidence| {
            (
                evidence.path.clone(),
                evidence.artefact_id.clone(),
                evidence.symbol_id.clone(),
            )
        })
        .unwrap_or((None, None, None))
}

fn confidence_level(score: f64) -> &'static str {
    if score >= 0.85 {
        "HIGH"
    } else if score >= 0.6 {
        "MEDIUM"
    } else {
        "LOW"
    }
}

fn http_primitive_id(repo_id: &str, key: &str) -> String {
    deterministic_uuid(&format!("http|primitive|{repo_id}|{key}"))
}

fn http_evidence_id(repo_id: &str, key: &str) -> String {
    deterministic_uuid(&format!("http|evidence|{repo_id}|{key}"))
}
