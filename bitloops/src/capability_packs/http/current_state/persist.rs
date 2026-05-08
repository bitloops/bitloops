use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::host::capability_host::CurrentStateConsumerRequest;
use crate::host::devql::{RelationalStorage, deterministic_uuid, esc_pg, sql_json_value, sql_now};
use crate::host::language_adapter::{LanguageHttpFact, LanguageHttpFactEvidence};

use super::{
    HTTP_PROTOCOL_SEED_FINGERPRINT, HttpBundleFact, HttpEvidenceFact, HttpPrimitiveFact,
    HttpQueryIndexRow, UpstreamHttpFactBatch,
};
use crate::capability_packs::http::types::HTTP_OWNER;

pub(super) fn stale_removed_source_fact_statements(
    request: &CurrentStateConsumerRequest,
    relational: &RelationalStorage,
) -> Vec<String> {
    let removed_paths = request
        .file_removals
        .iter()
        .map(|file| file.path.clone())
        .chain(
            request
                .artefact_removals
                .iter()
                .map(|artefact| artefact.path.clone()),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let removed_artefact_ids = request
        .artefact_removals
        .iter()
        .map(|artefact| artefact.artefact_id.clone())
        .collect::<Vec<_>>();
    let removed_symbol_ids = request
        .artefact_removals
        .iter()
        .map(|artefact| artefact.symbol_id.clone())
        .collect::<Vec<_>>();

    let mut target_filters = Vec::new();
    if !removed_paths.is_empty() {
        target_filters.push(format!("path IN ({})", sql_string_list(&removed_paths)));
    }
    if !removed_artefact_ids.is_empty() {
        target_filters.push(format!(
            "artefact_id IN ({})",
            sql_string_list(&removed_artefact_ids)
        ));
    }
    if !removed_symbol_ids.is_empty() {
        target_filters.push(format!(
            "symbol_id IN ({})",
            sql_string_list(&removed_symbol_ids)
        ));
    }
    if target_filters.is_empty() {
        return Vec::new();
    }

    vec![format!(
        "UPDATE http_primitives_current
         SET status = 'stale', updated_at = {now}
         WHERE repo_id = {repo_id}
           AND owner != {http_owner}
           AND primitive_id IN (
               SELECT DISTINCT primitive_id
               FROM http_primitive_evidence_current
               WHERE repo_id = {repo_id}
                 AND ({target_filter})
           );",
        repo_id = sql_text(&request.repo_id),
        http_owner = sql_text(HTTP_OWNER),
        target_filter = target_filters.join(" OR "),
        now = sql_now(relational),
    )]
}

pub(super) fn replace_upstream_fact_statements(
    relational: &RelationalStorage,
    repo_id: &str,
    batches: &[UpstreamHttpFactBatch],
) -> Vec<String> {
    let mut statements = Vec::new();
    for batch in batches {
        statements.push(format!(
            "DELETE FROM http_primitives_current
             WHERE repo_id = {repo_id}
               AND owner = {owner}
               AND primitive_id IN (
                   SELECT primitive_id
                   FROM http_primitive_evidence_current
                   WHERE repo_id = {repo_id}
                     AND path = {path}
               );",
            repo_id = sql_text(repo_id),
            owner = sql_text(&batch.owner),
            path = sql_text(&batch.path),
        ));
        statements.push(format!(
            "DELETE FROM http_primitive_evidence_current
             WHERE repo_id = {repo_id}
               AND path = {path}
               AND primitive_id NOT IN (
                   SELECT primitive_id
                   FROM http_primitives_current
                   WHERE repo_id = {repo_id}
               );",
            repo_id = sql_text(repo_id),
            path = sql_text(&batch.path),
        ));
        for fact in &batch.facts {
            let primitive = upstream_primitive_fact(repo_id, &batch.owner, fact);
            statements.push(insert_primitive_sql(relational, &primitive));
            for evidence in &primitive.evidence {
                statements.push(insert_evidence_sql(relational, &primitive, evidence));
            }
        }
    }
    statements
}

pub(super) fn replace_protocol_seed_statements(
    relational: &RelationalStorage,
    repo_id: &str,
    primitives: &[HttpPrimitiveFact],
) -> Vec<String> {
    let mut statements = vec![
        format!(
            "DELETE FROM http_primitive_evidence_current
             WHERE repo_id = {repo_id}
               AND primitive_id IN (
                   SELECT primitive_id
                   FROM http_primitives_current
                   WHERE repo_id = {repo_id}
                     AND owner = {http_owner}
                     AND input_fingerprint = {fingerprint}
               );",
            repo_id = sql_text(repo_id),
            http_owner = sql_text(HTTP_OWNER),
            fingerprint = sql_text(HTTP_PROTOCOL_SEED_FINGERPRINT),
        ),
        format!(
            "DELETE FROM http_primitives_current
             WHERE repo_id = {repo_id}
               AND owner = {http_owner}
               AND input_fingerprint = {fingerprint};",
            repo_id = sql_text(repo_id),
            http_owner = sql_text(HTTP_OWNER),
            fingerprint = sql_text(HTTP_PROTOCOL_SEED_FINGERPRINT),
        ),
    ];
    for primitive in primitives {
        statements.push(insert_primitive_sql(relational, primitive));
        for evidence in &primitive.evidence {
            statements.push(insert_evidence_sql(relational, primitive, evidence));
        }
    }
    statements
}

pub(super) async fn load_active_http_primitives(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<HttpPrimitiveFact>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT * FROM http_primitives_current
             WHERE repo_id = {} AND status != 'stale'
             ORDER BY owner ASC, primitive_id ASC;",
            sql_text(repo_id)
        ))
        .await
        .context("loading active HTTP primitives")?;
    let evidence = load_http_evidence(relational, repo_id).await?;
    rows.into_iter()
        .map(|row| {
            let primitive_id = required_string(&row, "primitive_id")?;
            Ok(HttpPrimitiveFact {
                repo_id: required_string(&row, "repo_id")?,
                owner: required_string(&row, "owner")?,
                primitive_type: required_string(&row, "primitive_type")?,
                subject: required_string(&row, "subject")?,
                roles: json_string_array(&row, "roles_json"),
                terms: json_string_array(&row, "terms_json"),
                properties: json_value(&row, "properties_json", json!({})),
                confidence_level: string_opt(&row, "confidence_level")
                    .unwrap_or_else(|| "MEDIUM".to_string()),
                confidence_score: row
                    .get("confidence_score")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5),
                status: string_opt(&row, "status").unwrap_or_else(|| "active".to_string()),
                input_fingerprint: string_opt(&row, "input_fingerprint").unwrap_or_default(),
                evidence: evidence.get(&primitive_id).cloned().unwrap_or_default(),
                primitive_id,
            })
        })
        .collect()
}

pub(super) fn insert_bundle_sql(relational: &RelationalStorage, bundle: &HttpBundleFact) -> String {
    format!(
        "INSERT INTO http_bundles_current (
            repo_id, bundle_id, bundle_kind, risk_kind, severity, matched_roles_json,
            primitive_ids_json, upstream_facts_json, causal_chain_json,
            invalidated_assumptions_json, obligations_json, confidence_level, confidence_score,
            status, input_fingerprint, updated_at
        ) VALUES (
            {repo_id}, {bundle_id}, {bundle_kind}, {risk_kind}, {severity}, {matched_roles},
            {primitive_ids}, {upstream_facts}, {causal_chain}, {invalidated_assumptions},
            {obligations}, {confidence_level}, {confidence_score}, {status},
            {input_fingerprint}, {now}
        )
        ON CONFLICT(repo_id, bundle_id) DO UPDATE SET
            bundle_kind = excluded.bundle_kind,
            risk_kind = excluded.risk_kind,
            severity = excluded.severity,
            matched_roles_json = excluded.matched_roles_json,
            primitive_ids_json = excluded.primitive_ids_json,
            upstream_facts_json = excluded.upstream_facts_json,
            causal_chain_json = excluded.causal_chain_json,
            invalidated_assumptions_json = excluded.invalidated_assumptions_json,
            obligations_json = excluded.obligations_json,
            confidence_level = excluded.confidence_level,
            confidence_score = excluded.confidence_score,
            status = excluded.status,
            input_fingerprint = excluded.input_fingerprint,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(&bundle.repo_id),
        bundle_id = sql_text(&bundle.bundle_id),
        bundle_kind = sql_text(&bundle.bundle_kind),
        risk_kind = sql_opt_text(bundle.risk_kind.as_deref()),
        severity = sql_opt_text(bundle.severity.as_deref()),
        matched_roles = sql_json_value(relational, &json!(bundle.matched_roles)),
        primitive_ids = sql_json_value(relational, &json!(bundle.primitive_ids)),
        upstream_facts = sql_json_value(relational, &bundle.upstream_facts),
        causal_chain = sql_json_value(relational, &bundle.causal_chain),
        invalidated_assumptions = sql_json_value(relational, &bundle.invalidated_assumptions),
        obligations = sql_json_value(relational, &bundle.obligations),
        confidence_level = sql_text(&bundle.confidence_level),
        confidence_score = bundle.confidence_score,
        status = sql_text(&bundle.status),
        input_fingerprint = sql_text(&bundle.input_fingerprint),
        now = sql_now(relational),
    )
}

pub(super) fn insert_query_index_row_sql(
    relational: &RelationalStorage,
    row: &HttpQueryIndexRow,
) -> String {
    format!(
        "INSERT INTO http_query_index_current (
            repo_id, owner, fact_id, bundle_id, terms_json, roles_json, subject,
            path, symbol_id, artefact_id, rank_signals_json, updated_at
        ) VALUES (
            {repo_id}, {owner}, {fact_id}, {bundle_id}, {terms}, {roles}, {subject},
            {path}, {symbol_id}, {artefact_id}, {rank_signals}, {now}
        )
        ON CONFLICT(repo_id, owner, fact_id) DO UPDATE SET
            bundle_id = excluded.bundle_id,
            terms_json = excluded.terms_json,
            roles_json = excluded.roles_json,
            subject = excluded.subject,
            path = excluded.path,
            symbol_id = excluded.symbol_id,
            artefact_id = excluded.artefact_id,
            rank_signals_json = excluded.rank_signals_json,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(&row.repo_id),
        owner = sql_text(&row.owner),
        fact_id = sql_text(&row.fact_id),
        bundle_id = sql_opt_text(row.bundle_id.as_deref()),
        terms = sql_json_value(relational, &json!(row.terms)),
        roles = sql_json_value(relational, &json!(row.roles)),
        subject = sql_text(&row.subject),
        path = sql_opt_text(row.path.as_deref()),
        symbol_id = sql_opt_text(row.symbol_id.as_deref()),
        artefact_id = sql_opt_text(row.artefact_id.as_deref()),
        rank_signals = sql_json_value(relational, &row.rank_signals),
        now = sql_now(relational),
    )
}

pub(super) fn insert_run_sql(
    relational: &RelationalStorage,
    repo_id: &str,
    generation_seq: u64,
    metrics: &Value,
) -> String {
    format!(
        "INSERT INTO http_runs_current (repo_id, status, warnings_json, metrics_json, updated_at)
         VALUES ({repo_id}, 'fresh', '[]', {metrics}, {now})
         ON CONFLICT(repo_id) DO UPDATE SET
            status = excluded.status,
            warnings_json = excluded.warnings_json,
            metrics_json = excluded.metrics_json,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(repo_id),
        metrics = sql_json_value(
            relational,
            &json!({
                "generationSeq": generation_seq,
                "metrics": metrics,
            }),
        ),
        now = sql_now(relational),
    )
}

pub(super) fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn upstream_primitive_fact(
    repo_id: &str,
    owner: &str,
    fact: &LanguageHttpFact,
) -> HttpPrimitiveFact {
    let primitive_id = deterministic_uuid(&format!(
        "http|upstream|{repo_id}|{owner}|{}",
        fact.stable_key
    ));
    HttpPrimitiveFact {
        repo_id: repo_id.to_string(),
        primitive_id: primitive_id.clone(),
        owner: owner.to_string(),
        primitive_type: fact.primitive_type.clone(),
        subject: fact.subject.clone(),
        roles: fact.roles.clone(),
        terms: fact.terms.clone(),
        properties: fact.properties.clone(),
        confidence_level: fact.confidence_level.clone(),
        confidence_score: fact.confidence_score,
        status: "active".to_string(),
        input_fingerprint: deterministic_uuid(&format!(
            "http|upstream-input|{repo_id}|{owner}|{}",
            fact.stable_key
        )),
        evidence: fact
            .evidence
            .iter()
            .enumerate()
            .map(|(index, evidence)| {
                upstream_evidence_fact(repo_id, owner, &primitive_id, index, evidence)
            })
            .collect(),
    }
}

fn upstream_evidence_fact(
    repo_id: &str,
    owner: &str,
    primitive_id: &str,
    index: usize,
    evidence: &LanguageHttpFactEvidence,
) -> HttpEvidenceFact {
    HttpEvidenceFact {
        evidence_id: deterministic_uuid(&format!(
            "http|upstream-evidence|{repo_id}|{owner}|{primitive_id}|{index}"
        )),
        kind: "language_http_fact".to_string(),
        path: Some(evidence.path.clone()),
        artefact_id: evidence.artefact_id.clone(),
        symbol_id: evidence.symbol_id.clone(),
        content_id: Some(evidence.content_id.clone()),
        start_line: evidence.start_line.map(i64::from),
        end_line: evidence.end_line.map(i64::from),
        start_byte: evidence.start_byte.map(i64::from),
        end_byte: evidence.end_byte.map(i64::from),
        dependency_package: None,
        dependency_version: None,
        source_url: None,
        excerpt_hash: None,
        producer: Some(owner.to_string()),
        model: None,
        prompt_hash: None,
        properties: evidence.properties.clone(),
    }
}

async fn load_http_evidence(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<BTreeMap<String, Vec<HttpEvidenceFact>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT * FROM http_primitive_evidence_current
             WHERE repo_id = {}
             ORDER BY primitive_id ASC, evidence_id ASC;",
            sql_text(repo_id)
        ))
        .await
        .context("loading HTTP primitive evidence")?;
    let mut grouped = BTreeMap::<String, Vec<HttpEvidenceFact>>::new();
    for row in rows {
        let primitive_id = required_string(&row, "primitive_id")?;
        grouped
            .entry(primitive_id)
            .or_default()
            .push(HttpEvidenceFact {
                evidence_id: required_string(&row, "evidence_id")?,
                kind: required_string(&row, "kind")?,
                path: string_opt(&row, "path"),
                artefact_id: string_opt(&row, "artefact_id"),
                symbol_id: string_opt(&row, "symbol_id"),
                content_id: string_opt(&row, "content_id"),
                start_line: i64_opt(&row, "start_line"),
                end_line: i64_opt(&row, "end_line"),
                start_byte: i64_opt(&row, "start_byte"),
                end_byte: i64_opt(&row, "end_byte"),
                dependency_package: string_opt(&row, "dependency_package"),
                dependency_version: string_opt(&row, "dependency_version"),
                source_url: string_opt(&row, "source_url"),
                excerpt_hash: string_opt(&row, "excerpt_hash"),
                producer: string_opt(&row, "producer"),
                model: string_opt(&row, "model"),
                prompt_hash: string_opt(&row, "prompt_hash"),
                properties: json_value(&row, "properties_json", json!({})),
            });
    }
    Ok(grouped)
}

fn insert_primitive_sql(relational: &RelationalStorage, primitive: &HttpPrimitiveFact) -> String {
    format!(
        "INSERT INTO http_primitives_current (
            repo_id, primitive_id, owner, primitive_type, subject, roles_json, terms_json,
            properties_json, confidence_level, confidence_score, status, input_fingerprint,
            updated_at
        ) VALUES (
            {repo_id}, {primitive_id}, {owner}, {primitive_type}, {subject}, {roles},
            {terms}, {properties}, {confidence_level}, {confidence_score}, {status},
            {input_fingerprint}, {now}
        )
        ON CONFLICT(repo_id, primitive_id) DO UPDATE SET
            owner = excluded.owner,
            primitive_type = excluded.primitive_type,
            subject = excluded.subject,
            roles_json = excluded.roles_json,
            terms_json = excluded.terms_json,
            properties_json = excluded.properties_json,
            confidence_level = excluded.confidence_level,
            confidence_score = excluded.confidence_score,
            status = excluded.status,
            input_fingerprint = excluded.input_fingerprint,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(&primitive.repo_id),
        primitive_id = sql_text(&primitive.primitive_id),
        owner = sql_text(&primitive.owner),
        primitive_type = sql_text(&primitive.primitive_type),
        subject = sql_text(&primitive.subject),
        roles = sql_json_value(relational, &json!(primitive.roles)),
        terms = sql_json_value(relational, &json!(primitive.terms)),
        properties = sql_json_value(relational, &primitive.properties),
        confidence_level = sql_text(&primitive.confidence_level),
        confidence_score = primitive.confidence_score,
        status = sql_text(&primitive.status),
        input_fingerprint = sql_text(&primitive.input_fingerprint),
        now = sql_now(relational),
    )
}

fn insert_evidence_sql(
    relational: &RelationalStorage,
    primitive: &HttpPrimitiveFact,
    evidence: &HttpEvidenceFact,
) -> String {
    format!(
        "INSERT INTO http_primitive_evidence_current (
            repo_id, primitive_id, evidence_id, kind, path, artefact_id, symbol_id, content_id,
            start_line, end_line, start_byte, end_byte, dependency_package, dependency_version,
            source_url, excerpt_hash, producer, model, prompt_hash, properties_json, updated_at
        ) VALUES (
            {repo_id}, {primitive_id}, {evidence_id}, {kind}, {path}, {artefact_id},
            {symbol_id}, {content_id}, {start_line}, {end_line}, {start_byte}, {end_byte},
            {dependency_package}, {dependency_version}, {source_url}, {excerpt_hash},
            {producer}, {model}, {prompt_hash}, {properties}, {now}
        )
        ON CONFLICT(repo_id, evidence_id) DO UPDATE SET
            primitive_id = excluded.primitive_id,
            kind = excluded.kind,
            path = excluded.path,
            artefact_id = excluded.artefact_id,
            symbol_id = excluded.symbol_id,
            content_id = excluded.content_id,
            start_line = excluded.start_line,
            end_line = excluded.end_line,
            start_byte = excluded.start_byte,
            end_byte = excluded.end_byte,
            dependency_package = excluded.dependency_package,
            dependency_version = excluded.dependency_version,
            source_url = excluded.source_url,
            excerpt_hash = excluded.excerpt_hash,
            producer = excluded.producer,
            model = excluded.model,
            prompt_hash = excluded.prompt_hash,
            properties_json = excluded.properties_json,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(&primitive.repo_id),
        primitive_id = sql_text(&primitive.primitive_id),
        evidence_id = sql_text(&evidence.evidence_id),
        kind = sql_text(&evidence.kind),
        path = sql_opt_text(evidence.path.as_deref()),
        artefact_id = sql_opt_text(evidence.artefact_id.as_deref()),
        symbol_id = sql_opt_text(evidence.symbol_id.as_deref()),
        content_id = sql_opt_text(evidence.content_id.as_deref()),
        start_line = sql_opt_i64(evidence.start_line),
        end_line = sql_opt_i64(evidence.end_line),
        start_byte = sql_opt_i64(evidence.start_byte),
        end_byte = sql_opt_i64(evidence.end_byte),
        dependency_package = sql_opt_text(evidence.dependency_package.as_deref()),
        dependency_version = sql_opt_text(evidence.dependency_version.as_deref()),
        source_url = sql_opt_text(evidence.source_url.as_deref()),
        excerpt_hash = sql_opt_text(evidence.excerpt_hash.as_deref()),
        producer = sql_opt_text(evidence.producer.as_deref()),
        model = sql_opt_text(evidence.model.as_deref()),
        prompt_hash = sql_opt_text(evidence.prompt_hash.as_deref()),
        properties = sql_json_value(relational, &evidence.properties),
        now = sql_now(relational),
    )
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    string_opt(row, key).ok_or_else(|| anyhow::anyhow!("missing HTTP current-state field `{key}`"))
}

fn string_opt(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn i64_opt(row: &Value, key: &str) -> Option<i64> {
    row.get(key).and_then(Value::as_i64)
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

fn json_string_array(row: &Value, key: &str) -> Vec<String> {
    json_value(row, key, json!([]))
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

fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}

fn sql_opt_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| sql_text(value))
        .collect::<Vec<_>>()
        .join(", ")
}
