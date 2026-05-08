use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use async_graphql::types::Json;
use serde_json::{Value, json};

use super::DevqlGraphqlContext;
use crate::capability_packs::http::types::{
    HTTP_PRIMITIVE_DERIVED_VALUE_RULE, HTTP_PRIMITIVE_HEADER_SEMANTIC,
    HTTP_PRIMITIVE_LIFECYCLE_PHASE_RULE, HTTP_PRIMITIVE_LOSSY_TRANSFORM,
    HTTP_PRIMITIVE_RUNTIME_BOUNDARY,
};
use crate::graphql::ResolverScope;
use crate::graphql::types::{
    HttpBundle, HttpCausalChainLink, HttpConfidence, HttpContextResult, HttpEvidence,
    HttpHeaderProducer, HttpInvalidatedAssumption, HttpLossyTransformAroundInput,
    HttpPatchImpactInput, HttpPatchImpactResult, HttpPrimitive, HttpPropagationObligation,
    HttpSearchResult, HttpUpstreamFact,
};
use crate::host::devql::esc_pg;

impl DevqlGraphqlContext {
    pub(crate) async fn http_search(
        &self,
        scope: &ResolverScope,
        terms: &[String],
        first: usize,
    ) -> Result<HttpSearchResult> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let limit = first.max(1);
        let rows = match self
            .query_devql_sqlite_rows(&http_search_index_sql(&repo_id, terms, limit))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };

        let primitive_ids = dedup(rows.iter().filter_map(|row| string_opt(row, "fact_id")));
        let bundle_ids = dedup(rows.iter().filter_map(|row| string_opt(row, "bundle_id")));
        let bundles = self.load_http_bundles(&repo_id, &bundle_ids, limit).await?;
        let matched_facts = self
            .load_http_primitives(&repo_id, &primitive_ids, limit)
            .await?;
        Ok(HttpSearchResult {
            overview: Json(http_overview(&bundles, terms)),
            bundles,
            matched_facts,
        })
    }

    pub(crate) async fn http_context_for_targets(
        &self,
        scope: &ResolverScope,
        artefact_ids: &[String],
        symbol_ids: &[String],
        paths: &[String],
        first: usize,
    ) -> Result<HttpContextResult> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let limit = first.max(1);
        let rows = match self
            .query_devql_sqlite_rows(&http_target_index_sql(
                &repo_id,
                artefact_ids,
                symbol_ids,
                paths,
                limit,
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let evidence_rows = match self
            .query_devql_sqlite_rows(&http_target_evidence_sql(
                &repo_id,
                artefact_ids,
                symbol_ids,
                paths,
                limit,
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };

        let primitive_ids = dedup(
            rows.iter()
                .filter_map(|row| string_opt(row, "fact_id"))
                .chain(
                    evidence_rows
                        .iter()
                        .filter_map(|row| string_opt(row, "primitive_id")),
                ),
        );
        let bundle_ids = dedup(rows.iter().filter_map(|row| string_opt(row, "bundle_id")));
        let bundles = self.load_http_bundles(&repo_id, &bundle_ids, limit).await?;
        let primitives = self
            .load_http_primitives(&repo_id, &primitive_ids, limit)
            .await?;
        let obligations = dedup_obligations(
            bundles
                .iter()
                .flat_map(|bundle| bundle.obligations.iter().cloned()),
        );
        Ok(HttpContextResult {
            overview: Json(http_context_overview(&bundles)),
            bundles,
            primitives,
            obligations,
        })
    }

    pub(crate) async fn http_context_for_terms(
        &self,
        scope: &ResolverScope,
        terms: &[String],
        first: usize,
    ) -> Result<HttpContextResult> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let limit = first.max(1);
        let rows = match self
            .query_devql_sqlite_rows(&http_search_index_sql(&repo_id, terms, limit))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let primitive_ids = dedup(rows.iter().filter_map(|row| string_opt(row, "fact_id")));
        let bundle_ids = dedup(rows.iter().filter_map(|row| string_opt(row, "bundle_id")));
        let bundles = self.load_http_bundles(&repo_id, &bundle_ids, limit).await?;
        let primitives = self
            .load_http_primitives(&repo_id, &primitive_ids, limit)
            .await?;
        let obligations = dedup_obligations(
            bundles
                .iter()
                .flat_map(|bundle| bundle.obligations.iter().cloned()),
        );
        Ok(HttpContextResult {
            overview: Json(http_context_overview(&bundles)),
            bundles,
            primitives,
            obligations,
        })
    }

    pub(crate) async fn http_header_producers(
        &self,
        scope: &ResolverScope,
        header_name: &str,
        first: usize,
    ) -> Result<Vec<HttpHeaderProducer>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let header = header_name.trim().to_ascii_lowercase();
        if header.is_empty() {
            return Ok(Vec::new());
        }
        let terms = vec![header];
        let primitives = self
            .query_http_primitives(
                &repo_id,
                Some(&[
                    HTTP_PRIMITIVE_DERIVED_VALUE_RULE,
                    HTTP_PRIMITIVE_HEADER_SEMANTIC,
                ]),
                &terms,
                None,
                first,
            )
            .await?;
        Ok(primitives
            .into_iter()
            .map(|primitive| {
                let properties = primitive
                    .properties
                    .0
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                HttpHeaderProducer {
                    primitive_id: primitive.id,
                    producer_kind: properties
                        .get("producerKind")
                        .or_else(|| properties.get("producer_kind"))
                        .and_then(Value::as_str)
                        .unwrap_or(&primitive.primitive_type)
                        .to_string(),
                    source_signal: properties
                        .get("sourceSignal")
                        .or_else(|| properties.get("source_signal"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    phase: properties
                        .get("phase")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    preconditions: string_array_from_value(
                        properties
                            .get("preconditions")
                            .cloned()
                            .unwrap_or(Value::Array(Vec::new())),
                    ),
                    confidence: primitive.confidence,
                }
            })
            .collect())
    }

    pub(crate) async fn http_lifecycle_boundaries(
        &self,
        scope: &ResolverScope,
        terms: &[String],
        first: usize,
    ) -> Result<Vec<HttpPrimitive>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        self.query_http_primitives(
            &repo_id,
            Some(&[
                HTTP_PRIMITIVE_RUNTIME_BOUNDARY,
                HTTP_PRIMITIVE_LIFECYCLE_PHASE_RULE,
            ]),
            terms,
            None,
            first,
        )
        .await
    }

    pub(crate) async fn http_lossy_transforms(
        &self,
        scope: &ResolverScope,
        around: Option<&HttpLossyTransformAroundInput>,
        first: usize,
    ) -> Result<Vec<HttpPrimitive>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let terms = Vec::new();
        self.query_http_primitives(
            &repo_id,
            Some(&[HTTP_PRIMITIVE_LOSSY_TRANSFORM]),
            &terms,
            around,
            first,
        )
        .await
    }

    pub(crate) async fn http_patch_impact(
        &self,
        scope: &ResolverScope,
        input: &HttpPatchImpactInput,
    ) -> Result<HttpPatchImpactResult> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let bundles = self
            .load_http_bundles_for_patch(&repo_id, input.patch_fingerprint.trim(), 100)
            .await?;
        Ok(HttpPatchImpactResult {
            patch_fingerprint: input.patch_fingerprint.clone(),
            invalidated_assumptions: dedup_assumptions(
                bundles
                    .iter()
                    .flat_map(|bundle| bundle.invalidated_assumptions.iter().cloned()),
            ),
            propagation_obligations: dedup_obligations(
                bundles
                    .iter()
                    .flat_map(|bundle| bundle.obligations.iter().cloned()),
            ),
        })
    }

    async fn query_http_primitives(
        &self,
        repo_id: &str,
        primitive_types: Option<&[&str]>,
        terms: &[String],
        around: Option<&HttpLossyTransformAroundInput>,
        first: usize,
    ) -> Result<Vec<HttpPrimitive>> {
        let rows = match self
            .query_devql_sqlite_rows(&http_primitive_query_sql(
                repo_id,
                primitive_types,
                terms,
                around,
                first.max(1),
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let primitive_ids = dedup(
            rows.iter()
                .filter_map(|row| string_opt(row, "primitive_id")),
        );
        self.load_http_primitives(repo_id, &primitive_ids, first)
            .await
    }

    async fn load_http_bundles_for_patch(
        &self,
        repo_id: &str,
        patch_fingerprint: &str,
        first: usize,
    ) -> Result<Vec<HttpBundle>> {
        let rows = match self
            .query_devql_sqlite_rows(&format!(
                "SELECT * FROM http_bundles_current \
                 WHERE repo_id = '{}' AND input_fingerprint = '{}' AND status != 'stale' \
                 ORDER BY confidence_score DESC, updated_at DESC, bundle_id ASC LIMIT {};",
                esc_pg(repo_id),
                esc_pg(patch_fingerprint),
                first.max(1)
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        Ok(rows.into_iter().map(bundle_from_row).collect())
    }

    async fn load_http_bundles(
        &self,
        repo_id: &str,
        bundle_ids: &[String],
        first: usize,
    ) -> Result<Vec<HttpBundle>> {
        if bundle_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = match self
            .query_devql_sqlite_rows(&format!(
                "SELECT * FROM http_bundles_current \
                 WHERE repo_id = '{}' AND bundle_id IN ({}) \
                 ORDER BY confidence_score DESC, updated_at DESC, bundle_id ASC LIMIT {};",
                esc_pg(repo_id),
                sql_string_list(bundle_ids),
                first.max(1)
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        Ok(rows.into_iter().map(bundle_from_row).collect())
    }

    async fn load_http_primitives(
        &self,
        repo_id: &str,
        primitive_ids: &[String],
        first: usize,
    ) -> Result<Vec<HttpPrimitive>> {
        if primitive_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = match self
            .query_devql_sqlite_rows(&format!(
                "SELECT * FROM http_primitives_current \
                 WHERE repo_id = '{}' AND primitive_id IN ({}) \
                 ORDER BY confidence_score DESC, updated_at DESC, primitive_id ASC LIMIT {};",
                esc_pg(repo_id),
                sql_string_list(primitive_ids),
                first.max(1)
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let mut evidence = self.load_http_evidence(repo_id, primitive_ids).await?;
        rows.into_iter()
            .map(|row| {
                let id = required_string(&row, "primitive_id")?;
                Ok(HttpPrimitive {
                    evidence: evidence.remove(&id).unwrap_or_default(),
                    id,
                    owner: required_string(&row, "owner")?,
                    primitive_type: required_string(&row, "primitive_type")?,
                    subject: required_string(&row, "subject")?,
                    roles: json_string_array(&row, "roles_json"),
                    terms: json_string_array(&row, "terms_json"),
                    status: required_string(&row, "status")?,
                    confidence: confidence_from_row(&row),
                    properties: Json(json_value(&row, "properties_json", json!({}))),
                })
            })
            .collect()
    }

    async fn load_http_evidence(
        &self,
        repo_id: &str,
        primitive_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<HttpEvidence>>> {
        if primitive_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let rows = match self
            .query_devql_sqlite_rows(&format!(
                "SELECT * FROM http_primitive_evidence_current \
                 WHERE repo_id = '{}' AND primitive_id IN ({}) \
                 ORDER BY primitive_id ASC, evidence_id ASC;",
                esc_pg(repo_id),
                sql_string_list(primitive_ids)
            ))
            .await
        {
            Ok(rows) => rows,
            Err(err) if is_missing_http_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let mut grouped = BTreeMap::<String, Vec<HttpEvidence>>::new();
        for row in rows {
            let primitive_id = required_string(&row, "primitive_id")?;
            grouped.entry(primitive_id).or_default().push(HttpEvidence {
                kind: required_string(&row, "kind")?,
                path: string_opt(&row, "path"),
                artefact_id: string_opt(&row, "artefact_id"),
                symbol_id: string_opt(&row, "symbol_id"),
                content_id: string_opt(&row, "content_id"),
                start_line: i32_opt(&row, "start_line"),
                end_line: i32_opt(&row, "end_line"),
                dependency_package: string_opt(&row, "dependency_package"),
                dependency_version: string_opt(&row, "dependency_version"),
                source_url: string_opt(&row, "source_url"),
            });
        }
        Ok(grouped)
    }
}

fn http_search_index_sql(repo_id: &str, terms: &[String], limit: usize) -> String {
    let filter = text_match_filter(terms, &["subject", "terms_json", "roles_json"]);
    format!(
        "SELECT * FROM http_query_index_current \
         WHERE repo_id = '{}' {} \
         ORDER BY \
            CASE WHEN bundle_id IS NOT NULL THEN 0 ELSE 1 END, \
            LENGTH(subject) ASC, updated_at DESC, fact_id ASC \
         LIMIT {};",
        esc_pg(repo_id),
        filter,
        limit
    )
}

fn http_target_index_sql(
    repo_id: &str,
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
    limit: usize,
) -> String {
    let target_filter = target_filter_sql(artefact_ids, symbol_ids, paths);
    format!(
        "SELECT * FROM http_query_index_current \
         WHERE repo_id = '{}' AND ({target_filter}) \
         ORDER BY \
            CASE WHEN bundle_id IS NOT NULL THEN 0 ELSE 1 END, \
            updated_at DESC, fact_id ASC \
         LIMIT {};",
        esc_pg(repo_id),
        limit
    )
}

fn http_target_evidence_sql(
    repo_id: &str,
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
    limit: usize,
) -> String {
    let target_filter = target_filter_sql(artefact_ids, symbol_ids, paths);
    format!(
        "SELECT primitive_id FROM http_primitive_evidence_current \
         WHERE repo_id = '{}' AND ({target_filter}) \
         ORDER BY updated_at DESC, primitive_id ASC LIMIT {};",
        esc_pg(repo_id),
        limit
    )
}

fn http_primitive_query_sql(
    repo_id: &str,
    primitive_types: Option<&[&str]>,
    terms: &[String],
    around: Option<&HttpLossyTransformAroundInput>,
    limit: usize,
) -> String {
    let mut filters = vec![format!("repo_id = '{}'", esc_pg(repo_id))];
    if let Some(primitive_types) = primitive_types
        && !primitive_types.is_empty()
    {
        filters.push(format!(
            "primitive_type IN ({})",
            sql_str_list(primitive_types)
        ));
    }
    let term_filter = text_match_filter(
        terms,
        &["subject", "terms_json", "roles_json", "properties_json"],
    );
    if !term_filter.is_empty() {
        filters.push(term_filter.trim_start_matches("AND ").to_string());
    }
    if let Some(around) = around {
        filters.push(around_filter_sql(repo_id, around));
    }
    format!(
        "SELECT * FROM http_primitives_current WHERE {} \
         ORDER BY confidence_score DESC, updated_at DESC, primitive_id ASC LIMIT {};",
        filters.join(" AND "),
        limit
    )
}

fn target_filter_sql(artefact_ids: &[String], symbol_ids: &[String], paths: &[String]) -> String {
    let mut filters = Vec::new();
    if !artefact_ids.is_empty() {
        filters.push(format!(
            "artefact_id IN ({})",
            sql_string_list(artefact_ids)
        ));
    }
    if !symbol_ids.is_empty() {
        filters.push(format!("symbol_id IN ({})", sql_string_list(symbol_ids)));
    }
    if !paths.is_empty() {
        filters.push(format!("path IN ({})", sql_string_list(paths)));
    }
    if filters.is_empty() {
        "1 = 0".to_string()
    } else {
        filters.join(" OR ")
    }
}

fn around_filter_sql(repo_id: &str, around: &HttpLossyTransformAroundInput) -> String {
    let mut filters = Vec::new();
    if let Some(value) = around
        .symbol_fqn
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let escaped = esc_pg(&value.to_ascii_lowercase());
        filters.push(format!(
            "(LOWER(subject) LIKE '%{escaped}%' OR LOWER(properties_json) LIKE '%{escaped}%')"
        ));
    }
    if let Some(value) = around
        .symbol_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        filters.push(format!(
            "primitive_id IN (SELECT primitive_id FROM http_primitive_evidence_current WHERE repo_id = '{}' AND symbol_id = '{}')",
            esc_pg(repo_id),
            esc_pg(value),
        ));
    }
    if let Some(value) = around
        .artefact_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        filters.push(format!(
            "primitive_id IN (SELECT primitive_id FROM http_primitive_evidence_current WHERE repo_id = '{}' AND artefact_id = '{}')",
            esc_pg(repo_id),
            esc_pg(value),
        ));
    }
    if let Some(value) = around
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        filters.push(format!(
            "primitive_id IN (SELECT primitive_id FROM http_primitive_evidence_current WHERE repo_id = '{}' AND path = '{}')",
            esc_pg(repo_id),
            esc_pg(value),
        ));
    }
    if filters.is_empty() {
        "1 = 1".to_string()
    } else {
        format!("({})", filters.join(" OR "))
    }
}

fn text_match_filter(terms: &[String], columns: &[&str]) -> String {
    let terms = terms
        .iter()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return String::new();
    }
    let clauses = terms
        .iter()
        .map(|term| {
            let term = esc_pg(term);
            let column_clauses = columns
                .iter()
                .map(|column| format!("LOWER({column}) LIKE '%{term}%'"))
                .collect::<Vec<_>>();
            format!("({})", column_clauses.join(" OR "))
        })
        .collect::<Vec<_>>();
    format!("AND ({})", clauses.join(" OR "))
}

fn bundle_from_row(row: Value) -> HttpBundle {
    HttpBundle {
        bundle_id: required_string(&row, "bundle_id").unwrap_or_default(),
        kind: required_string(&row, "bundle_kind").unwrap_or_default(),
        risk_kind: string_opt(&row, "risk_kind"),
        severity: string_opt(&row, "severity"),
        matched_roles: json_string_array(&row, "matched_roles_json"),
        status: required_string(&row, "status").unwrap_or_else(|_| "active".to_string()),
        confidence: confidence_from_row(&row),
        upstream_facts: json_value(&row, "upstream_facts_json", json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(upstream_fact_from_value)
            .collect(),
        causal_chain: json_value(&row, "causal_chain_json", json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(causal_chain_link_from_value)
            .collect(),
        invalidated_assumptions: json_value(&row, "invalidated_assumptions_json", json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(assumption_from_value)
            .collect(),
        obligations: json_value(&row, "obligations_json", json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(obligation_from_value)
            .collect(),
    }
}

fn upstream_fact_from_value(value: Value) -> HttpUpstreamFact {
    HttpUpstreamFact {
        owner: value_string(&value, &["owner"]).unwrap_or_default(),
        fact_id: value_string(&value, &["factId", "fact_id"]).unwrap_or_default(),
        primitive_type: value_string(&value, &["primitiveType", "primitive_type"]),
        subject: value_string(&value, &["subject"]),
        roles: string_array_from_value(
            value
                .get("roles")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
    }
}

fn causal_chain_link_from_value(value: Value) -> HttpCausalChainLink {
    HttpCausalChainLink {
        owner: value_string(&value, &["owner"]).unwrap_or_default(),
        fact_id: value_string(&value, &["factId", "fact_id"]).unwrap_or_default(),
        role: value_string(&value, &["role"]).unwrap_or_default(),
        primitive_type: value_string(&value, &["primitiveType", "primitive_type"]),
        subject: value_string(&value, &["subject"]),
    }
}

fn assumption_from_value(value: Value) -> HttpInvalidatedAssumption {
    HttpInvalidatedAssumption {
        id: value_string(&value, &["id"]).unwrap_or_default(),
        assumption: value_string(&value, &["assumption"]).unwrap_or_default(),
        invalidated_by_primitive_ids: string_array_from_value(
            value
                .get("invalidatedByPrimitiveIds")
                .or_else(|| value.get("invalidated_by_primitive_ids"))
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        scope: value_string(&value, &["scope"]),
    }
}

fn obligation_from_value(value: Value) -> HttpPropagationObligation {
    HttpPropagationObligation {
        id: value_string(&value, &["id"]).unwrap_or_default(),
        required_follow_up: value_string(&value, &["requiredFollowUp", "required_follow_up"])
            .unwrap_or_default(),
        target_symbols: string_array_from_value(
            value
                .get("targetSymbols")
                .or_else(|| value.get("target_symbols"))
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        blocking: value
            .get("blocking")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn http_overview(bundles: &[HttpBundle], terms: &[String]) -> Value {
    let top_risks = top_risks(bundles);
    json!({
        "terms": terms,
        "bundleCount": bundles.len(),
        "riskCount": bundles.iter().filter(|bundle| bundle.risk_kind.is_some()).count(),
        "topRisks": top_risks,
    })
}

pub(crate) fn http_context_overview(bundles: &[HttpBundle]) -> Value {
    json!({
        "bundleCount": bundles.len(),
        "riskCount": bundles.iter().filter(|bundle| bundle.risk_kind.is_some()).count(),
        "topRisks": top_risks(bundles),
        "expandHint": {
            "template": "selectArtefacts(...){ httpContext { bundles { bundleId riskKind causalChain { factId role owner } } } }"
        }
    })
}

fn top_risks(bundles: &[HttpBundle]) -> Vec<Value> {
    bundles
        .iter()
        .filter(|bundle| bundle.risk_kind.is_some())
        .take(3)
        .map(|bundle| {
            json!({
                "bundleId": bundle.bundle_id,
                "riskKind": bundle.risk_kind,
                "severity": bundle.severity,
                "matchedRoles": bundle.matched_roles,
            })
        })
        .collect()
}

fn confidence_from_row(row: &Value) -> HttpConfidence {
    HttpConfidence {
        level: string_opt(row, "confidence_level").unwrap_or_else(|| "MEDIUM".to_string()),
        score: row
            .get("confidence_score")
            .and_then(Value::as_f64)
            .unwrap_or(0.5),
    }
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    string_opt(row, key).ok_or_else(|| anyhow!("missing required HTTP row field `{key}`"))
}

fn string_opt(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn i32_opt(row: &Value, key: &str) -> Option<i32> {
    row.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn json_string_array(row: &Value, key: &str) -> Vec<String> {
    string_array_from_value(json_value(row, key, json!([])))
}

fn json_value(row: &Value, key: &str, default: Value) -> Value {
    let Some(value) = row.get(key) else {
        return default;
    };
    match value {
        Value::String(raw) => serde_json::from_str(raw).unwrap_or(default),
        Value::Null => default,
        other => other.clone(),
    }
}

fn string_array_from_value(value: Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_str_list(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn dedup(values: impl Iterator<Item = String>) -> Vec<String> {
    values.collect::<BTreeSet<_>>().into_iter().collect()
}

fn dedup_assumptions(
    values: impl Iterator<Item = HttpInvalidatedAssumption>,
) -> Vec<HttpInvalidatedAssumption> {
    let mut seen = BTreeSet::new();
    values
        .filter(|value| seen.insert(value.id.clone()))
        .collect()
}

fn dedup_obligations(
    values: impl Iterator<Item = HttpPropagationObligation>,
) -> Vec<HttpPropagationObligation> {
    let mut seen = BTreeSet::new();
    values
        .filter(|value| seen.insert(value.id.clone()))
        .collect()
}

fn is_missing_http_table_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("http_primitives_current")
        || message.contains("http_primitive_evidence_current")
        || message.contains("http_bundles_current")
        || message.contains("http_query_index_current")
        || message.contains("no such table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_overview_is_compact_and_points_to_http_context() {
        let bundles = vec![HttpBundle {
            bundle_id: "bundle-1".to_string(),
            kind: "CAUSAL_RISK".to_string(),
            risk_kind: Some("CONTENT_LENGTH_LOSS".to_string()),
            severity: Some("HIGH".to_string()),
            matched_roles: vec![
                "http.response.body_replacement".to_string(),
                "http.body.exact_size_signal".to_string(),
            ],
            status: "active".to_string(),
            confidence: HttpConfidence {
                level: "HIGH".to_string(),
                score: 0.91,
            },
            upstream_facts: Vec::new(),
            causal_chain: Vec::new(),
            invalidated_assumptions: Vec::new(),
            obligations: Vec::new(),
        }];

        let overview = http_context_overview(&bundles);

        assert_eq!(overview["bundleCount"], 1);
        assert_eq!(overview["riskCount"], 1);
        assert_eq!(overview["topRisks"][0]["riskKind"], "CONTENT_LENGTH_LOSS");
        assert!(
            overview["expandHint"]["template"]
                .as_str()
                .expect("expand hint")
                .contains("httpContext")
        );
        assert!(overview.get("causalChain").is_none());
    }
}
