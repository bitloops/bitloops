use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::capability_packs::architecture_graph::roles::RoleAdjudicationMailboxPayload;
use crate::capability_packs::architecture_graph::roles::storage::list_recent_role_adjudication_attempts;
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
};
use crate::host::devql::RelationalStorage;
use crate::host::runtime_store::{
    RepoCapabilityWorkplaneStatusReader, WorkplaneJobQuery, WorkplaneJobRecord, WorkplaneJobStatus,
};

use super::super::{DevqlArchitectureRolesStatusArgs, SlimCliRepoScope};
use super::support::{sql_text, value_json, value_str};

const ROLE_REVIEW_STATUSES: &[&str] = &["needs_review", "stale", "rejected", "unknown"];

#[derive(Debug, Clone, Serialize)]
struct RolesStatusOutput {
    queue_summary: RoleAdjudicationQueueSummary,
    queue_items: Vec<RoleAdjudicationQueueItem>,
    review_items: Vec<RoleReviewItem>,
    adjudication_attempt_summary: RoleAdjudicationAttemptSummary,
    adjudication_attempts: Vec<RoleAdjudicationAttemptItem>,
}

#[derive(Debug, Clone, Serialize)]
struct RoleAdjudicationQueueSummary {
    total: usize,
    by_status: BTreeMap<String, usize>,
    by_reason: BTreeMap<String, usize>,
    parse_errors: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RoleAdjudicationQueueItem {
    pub(super) job_id: String,
    pub(super) status: String,
    pub(super) attempts: u32,
    pub(super) updated_at_unix: u64,
    pub(super) dedupe_key: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) generation: Option<u64>,
    pub(super) artefact_id: Option<String>,
    pub(super) symbol_id: Option<String>,
    pub(super) path: Option<String>,
    pub(super) canonical_kind: Option<String>,
    pub(super) deterministic_confidence: Option<f64>,
    pub(super) parse_error: Option<String>,
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RoleAdjudicationAttemptSummary {
    total: usize,
    by_outcome: BTreeMap<String, usize>,
    persisted_assignments: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RoleAdjudicationAttemptItem {
    pub(super) attempt_id: String,
    pub(super) scope_key: String,
    pub(super) generation: u64,
    pub(super) target_kind: Option<String>,
    pub(super) artefact_id: Option<String>,
    pub(super) symbol_id: Option<String>,
    pub(super) path: Option<String>,
    pub(super) reason: String,
    pub(super) outcome: String,
    pub(super) model_descriptor: String,
    pub(super) assignment_write_persisted: bool,
    pub(super) assignment_write_source: Option<String>,
    pub(super) failure_message: Option<String>,
    pub(super) reasoning_summary: Option<String>,
    pub(super) observed_at_unix: u64,
    pub(super) updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RoleReviewItem {
    pub(super) assignment_id: String,
    pub(super) artefact_id: String,
    pub(super) path: Option<String>,
    pub(super) role_id: String,
    pub(super) source_kind: String,
    pub(super) confidence: f64,
    pub(super) status: String,
    pub(super) status_reason: String,
    pub(super) updated_at: Option<String>,
}

pub(super) async fn run_architecture_roles_status(
    scope: &SlimCliRepoScope,
    relational: &RelationalStorage,
    args: DevqlArchitectureRolesStatusArgs,
) -> Result<()> {
    let limit = usize::try_from(args.limit).context("converting --limit to usize")?;
    let queue_items = load_role_adjudication_queue_items(scope, limit)?;
    let review_items = load_role_review_items(relational, &scope.repo.repo_id, limit).await?;
    let adjudication_attempts =
        load_role_adjudication_attempt_items(relational, &scope.repo.repo_id, limit).await?;
    let summary = summarise_queue_items(&queue_items);
    let adjudication_attempt_summary = summarise_adjudication_attempts(&adjudication_attempts);
    let output = RolesStatusOutput {
        queue_summary: summary,
        queue_items,
        review_items,
        adjudication_attempt_summary,
        adjudication_attempts,
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .context("serialising roles status output as JSON")?
        );
        return Ok(());
    }

    print_roles_status_human(&output);
    Ok(())
}

fn load_role_adjudication_queue_items(
    scope: &SlimCliRepoScope,
    limit: usize,
) -> Result<Vec<RoleAdjudicationQueueItem>> {
    let Some(reader) =
        RepoCapabilityWorkplaneStatusReader::open(&scope.repo_root, &scope.repo.repo_id).context(
            "opening read-only repo runtime status reader for architecture roles status",
        )?
    else {
        return Ok(Vec::new());
    };

    let jobs = reader
        .list_capability_workplane_jobs(WorkplaneJobQuery {
            capability_id: Some(ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string()),
            mailbox_name: Some(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string()),
            statuses: vec![
                WorkplaneJobStatus::Pending,
                WorkplaneJobStatus::Running,
                WorkplaneJobStatus::Failed,
            ],
            limit: Some(limit as u64),
        })
        .context("loading architecture role adjudication queue jobs read-only")?;

    Ok(jobs
        .into_iter()
        .map(role_adjudication_queue_item_from_job)
        .collect())
}

pub(super) fn role_adjudication_queue_item_from_job(
    job: WorkplaneJobRecord,
) -> RoleAdjudicationQueueItem {
    let mut item = RoleAdjudicationQueueItem {
        job_id: job.job_id,
        status: job.status.as_str().to_string(),
        attempts: job.attempts,
        updated_at_unix: job.updated_at_unix,
        dedupe_key: job.dedupe_key,
        reason: None,
        generation: None,
        artefact_id: None,
        symbol_id: None,
        path: None,
        canonical_kind: None,
        deterministic_confidence: None,
        parse_error: None,
        last_error: job.last_error,
    };

    match serde_json::from_value::<RoleAdjudicationMailboxPayload>(job.payload) {
        Ok(payload) => {
            item.reason = Some(payload.request.reason.as_str().to_string());
            item.generation = Some(payload.request.generation);
            item.artefact_id = payload.request.artefact_id;
            item.symbol_id = payload.request.symbol_id;
            item.path = payload.request.path;
            item.canonical_kind = payload.request.canonical_kind;
            item.deterministic_confidence = payload.request.deterministic_confidence;
        }
        Err(err) => {
            item.parse_error = Some(err.to_string());
        }
    }

    item
}

fn summarise_queue_items(items: &[RoleAdjudicationQueueItem]) -> RoleAdjudicationQueueSummary {
    let mut by_status = BTreeMap::<String, usize>::new();
    let mut by_reason = BTreeMap::<String, usize>::new();
    let mut parse_errors = 0usize;
    for item in items {
        *by_status.entry(item.status.clone()).or_default() += 1;
        if let Some(reason) = item.reason.as_ref() {
            *by_reason.entry(reason.clone()).or_default() += 1;
        }
        if item.parse_error.is_some() {
            parse_errors += 1;
        }
    }
    RoleAdjudicationQueueSummary {
        total: items.len(),
        by_status,
        by_reason,
        parse_errors,
    }
}

pub(super) async fn load_role_adjudication_attempt_items(
    relational: &RelationalStorage,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<RoleAdjudicationAttemptItem>> {
    let records = list_recent_role_adjudication_attempts(relational, repo_id, limit).await?;
    Ok(records
        .into_iter()
        .map(|record| RoleAdjudicationAttemptItem {
            attempt_id: record.attempt_id,
            scope_key: record.scope_key,
            generation: record.generation,
            target_kind: record.target_kind,
            artefact_id: record.artefact_id,
            symbol_id: record.symbol_id,
            path: record.path,
            reason: record.reason,
            outcome: record.outcome,
            model_descriptor: record.model_descriptor,
            assignment_write_persisted: record.assignment_write_persisted,
            assignment_write_source: record.assignment_write_source,
            failure_message: record.failure_message,
            reasoning_summary: record.reasoning_summary,
            observed_at_unix: record.observed_at_unix,
            updated_at: record.updated_at,
        })
        .collect())
}

fn summarise_adjudication_attempts(
    items: &[RoleAdjudicationAttemptItem],
) -> RoleAdjudicationAttemptSummary {
    let mut by_outcome = BTreeMap::<String, usize>::new();
    let mut persisted_assignments = 0usize;
    for item in items {
        *by_outcome.entry(item.outcome.clone()).or_default() += 1;
        if item.assignment_write_persisted {
            persisted_assignments += 1;
        }
    }
    RoleAdjudicationAttemptSummary {
        total: items.len(),
        by_outcome,
        persisted_assignments,
    }
}

pub(super) async fn load_role_review_items(
    relational: &RelationalStorage,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<RoleReviewItem>> {
    let status_filters = ROLE_REVIEW_STATUSES
        .iter()
        .map(|status| format!("'{}'", status.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = relational
        .query_rows(&format!(
            "SELECT a.assignment_id, COALESCE(a.artefact_id, a.symbol_id, a.path) AS artefact_id, \
                    a.role_id, a.source AS source_kind, a.confidence, a.status, \
                    a.provenance_json, a.updated_at, a.path \
             FROM architecture_role_assignments_current a \
             WHERE a.repo_id = {repo_id} \
               AND a.status IN ({status_filters}) \
             ORDER BY a.updated_at DESC \
             LIMIT {limit};",
            repo_id = sql_text(repo_id),
        ))
        .await
        .context("loading architecture role review items")?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let assignment_id = value_str(&row, "assignment_id")
            .ok_or_else(|| anyhow!("missing `assignment_id` in architecture role review row"))?
            .to_string();
        let artefact_id = value_str(&row, "artefact_id")
            .ok_or_else(|| anyhow!("missing `artefact_id` in architecture role review row"))?
            .to_string();
        let role_id = value_str(&row, "role_id")
            .ok_or_else(|| anyhow!("missing `role_id` in architecture role review row"))?
            .to_string();
        let source_kind = value_str(&row, "source_kind")
            .ok_or_else(|| anyhow!("missing `source_kind` in architecture role review row"))?
            .to_string();
        let confidence = row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
        let status = value_str(&row, "status")
            .ok_or_else(|| anyhow!("missing `status` in architecture role review row"))?
            .to_string();
        let status_reason = value_json(&row, "provenance_json")
            .and_then(|value| {
                value
                    .get("statusReason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_default();
        let updated_at = value_str(&row, "updated_at").map(ToOwned::to_owned);
        let path = value_str(&row, "path").map(ToOwned::to_owned);

        items.push(RoleReviewItem {
            assignment_id,
            artefact_id,
            path,
            role_id,
            source_kind,
            confidence,
            status,
            status_reason,
            updated_at,
        });
    }

    Ok(items)
}

fn print_roles_status_human(output: &RolesStatusOutput) {
    if output.queue_items.is_empty()
        && output.review_items.is_empty()
        && output.adjudication_attempts.is_empty()
    {
        println!("no ambiguous architecture roles found");
        return;
    }

    println!("queue summary:");
    println!("  total={}", output.queue_summary.total);
    if !output.queue_summary.by_status.is_empty() {
        let entries = output
            .queue_summary
            .by_status
            .iter()
            .map(|(status, count)| format!("{status}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  by_status: {entries}");
    }
    if !output.queue_summary.by_reason.is_empty() {
        let entries = output
            .queue_summary
            .by_reason
            .iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  by_reason: {entries}");
    }
    if output.queue_summary.parse_errors > 0 {
        println!("  parse_errors={}", output.queue_summary.parse_errors);
    }

    if !output.queue_items.is_empty() {
        println!("queue items:");
        for item in &output.queue_items {
            println!(
                "  job={} status={} reason={} path={} artefact={} symbol={} generation={} confidence={} attempts={} updated_at_unix={}",
                item.job_id,
                item.status,
                item.reason.as_deref().unwrap_or("<unknown>"),
                item.path.as_deref().unwrap_or("<unknown>"),
                item.artefact_id.as_deref().unwrap_or("<unknown>"),
                item.symbol_id.as_deref().unwrap_or("<unknown>"),
                item.generation
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string()),
                item.deterministic_confidence
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "<unknown>".to_string()),
                item.attempts,
                item.updated_at_unix
            );
            if let Some(parse_error) = item.parse_error.as_deref() {
                println!("    parse_error={parse_error}");
            }
            if let Some(last_error) = item.last_error.as_deref() {
                println!("    last_error={last_error}");
            }
        }
    }

    if !output.adjudication_attempts.is_empty() {
        println!("recent adjudication attempts:");
        for item in &output.adjudication_attempts {
            println!(
                "  attempt={} outcome={} persisted={} write_source={} reason={} path={} artefact={} symbol={} generation={} model={} observed_at_unix={}",
                item.attempt_id,
                item.outcome,
                item.assignment_write_persisted,
                item.assignment_write_source
                    .as_deref()
                    .unwrap_or("<unknown>"),
                item.reason,
                item.path.as_deref().unwrap_or("<unknown>"),
                item.artefact_id.as_deref().unwrap_or("<unknown>"),
                item.symbol_id.as_deref().unwrap_or("<unknown>"),
                item.generation,
                item.model_descriptor,
                item.observed_at_unix,
            );
            if let Some(summary) = item
                .reasoning_summary
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                println!("    reasoning={summary}");
            }
            if let Some(failure) = item
                .failure_message
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                println!("    failure={failure}");
            }
        }
    }

    if !output.review_items.is_empty() {
        println!("review items:");
        for item in &output.review_items {
            println!(
                "  assignment={} status={} role={} source={} confidence={:.3} artefact={} path={} updated_at={}",
                item.assignment_id,
                item.status,
                item.role_id,
                item.source_kind,
                item.confidence,
                item.artefact_id,
                item.path.as_deref().unwrap_or("<unknown>"),
                item.updated_at.as_deref().unwrap_or("<unknown>"),
            );
            if !item.status_reason.trim().is_empty() {
                println!("    reason={}", item.status_reason);
            }
        }
    }
}
