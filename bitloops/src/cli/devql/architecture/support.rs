use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::capability_packs::architecture_graph::roles::migrations::{
    ProposalApplySummary, ProposalSummary,
};

pub(super) fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn value_str<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

pub(super) fn value_json(row: &Value, key: &str) -> Option<Value> {
    match row.get(key) {
        Some(Value::String(text)) => serde_json::from_str(text).ok(),
        Some(value) => Some(value.clone()),
        None => None,
    }
}

pub(super) fn print_proposal_summary(summary: &ProposalSummary) {
    println!(
        "proposal={} type={} status={}",
        summary.proposal_id, summary.proposal_type, summary.status
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&summary.preview_payload)
            .unwrap_or_else(|_| summary.preview_payload.to_string())
    );
}

pub(super) fn print_apply_summary(summary: &ProposalApplySummary) {
    println!(
        "proposal={} type={} applied",
        summary.proposal_id, summary.proposal_type
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&summary.result_payload)
            .unwrap_or_else(|_| summary.result_payload.to_string())
    );
    if !summary.migration_records.is_empty() {
        println!("migrations={}", summary.migration_records.len());
    }
}

pub(super) fn load_json_spec<T: DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading architecture roles spec from `{}`", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing JSON spec from `{}`", path.display()))
}

pub(super) fn cli_provenance(operation: &str) -> Value {
    json!({
        "source": "devql_cli",
        "operation": operation,
    })
}
