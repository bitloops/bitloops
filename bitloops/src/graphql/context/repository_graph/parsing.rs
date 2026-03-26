use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::Value;

use crate::graphql::types::{
    Artefact, CanonicalKind, DateTimeScalar, DependencyEdge, EdgeKind, FileContext,
};

pub(super) fn file_context_from_value(row: Value) -> Result<FileContext> {
    let path = string_field(&row, "path")?;
    Ok(FileContext {
        path,
        language: optional_string_field(&row, "language"),
        blob_sha: optional_string_field(&row, "blob_sha"),
    })
}

pub(super) fn artefact_from_value(row: Value) -> Result<Artefact> {
    let canonical_kind_raw = string_field(&row, "canonical_kind")?;
    Ok(Artefact {
        id: async_graphql::ID(string_field(&row, "artefact_id")?),
        symbol_id: string_field(&row, "symbol_id")?,
        path: string_field(&row, "path")?,
        language: string_field(&row, "language")?,
        canonical_kind: parse_canonical_kind(canonical_kind_raw.as_str())?,
        language_kind: optional_string_field(&row, "language_kind"),
        symbol_fqn: optional_string_field(&row, "symbol_fqn"),
        parent_artefact_id: optional_string_field(&row, "parent_artefact_id")
            .map(async_graphql::ID),
        start_line: required_i32_field(&row, "start_line")?,
        end_line: required_i32_field(&row, "end_line")?,
        start_byte: required_i32_field(&row, "start_byte")?,
        end_byte: required_i32_field(&row, "end_byte")?,
        signature: optional_string_field(&row, "signature"),
        modifiers: parse_string_array_field(&row, "modifiers"),
        docstring: optional_string_field(&row, "docstring"),
        content_hash: optional_string_field(&row, "content_hash"),
        blob_sha: string_field(&row, "blob_sha")?,
        created_at: parse_storage_datetime(string_field(&row, "created_at")?.as_str())?,
    })
}

pub(super) fn dependency_edge_from_value(row: Value) -> Result<DependencyEdge> {
    let edge_kind_raw = string_field(&row, "edge_kind")?;
    Ok(DependencyEdge {
        id: async_graphql::ID(string_field(&row, "edge_id")?),
        edge_kind: parse_edge_kind(edge_kind_raw.as_str())?,
        language: string_field(&row, "language")?,
        from_artefact_id: async_graphql::ID(string_field(&row, "from_artefact_id")?),
        to_artefact_id: optional_string_field(&row, "to_artefact_id").map(async_graphql::ID),
        to_symbol_ref: optional_string_field(&row, "to_symbol_ref"),
        start_line: optional_i32_field(&row, "start_line"),
        end_line: optional_i32_field(&row, "end_line"),
        metadata: parse_json_field(&row, "metadata").map(async_graphql::types::Json),
    })
}

fn string_field(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("missing string field `{key}`"))
}

fn optional_string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_i32_field(row: &Value, key: &str) -> Result<i32> {
    optional_i32_field(row, key).with_context(|| format!("missing integer field `{key}`"))
}

fn optional_i32_field(row: &Value, key: &str) -> Option<i32> {
    row.get(key)
        .and_then(Value::as_i64)
        .map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

fn parse_json_field(row: &Value, key: &str) -> Option<serde_json::Value> {
    let value = row.get(key)?;
    if value.is_null() {
        return None;
    }
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).ok()
    } else {
        Some(value.clone())
    }
}

fn parse_string_array_field(row: &Value, key: &str) -> Vec<String> {
    let Some(value) = row.get(key) else {
        return Vec::new();
    };
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    value
        .as_str()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

fn parse_storage_datetime(value: &str) -> Result<DateTimeScalar> {
    if let Ok(timestamp) = DateTimeScalar::from_rfc3339(value.to_string()) {
        return Ok(timestamp);
    }

    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("parsing storage timestamp `{value}`"))?;
    let zero_offset = FixedOffset::east_opt(0).expect("zero offset is valid");
    DateTimeScalar::from_rfc3339(
        DateTime::<FixedOffset>::from_naive_utc_and_offset(parsed, zero_offset).to_rfc3339(),
    )
    .with_context(|| format!("normalising storage timestamp `{value}`"))
}

fn parse_canonical_kind(value: &str) -> Result<CanonicalKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "file" => Ok(CanonicalKind::File),
        "namespace" => Ok(CanonicalKind::Namespace),
        "module" => Ok(CanonicalKind::Module),
        "import" => Ok(CanonicalKind::Import),
        "type" => Ok(CanonicalKind::Type),
        "interface" => Ok(CanonicalKind::Interface),
        "enum" => Ok(CanonicalKind::Enum),
        "callable" => Ok(CanonicalKind::Callable),
        "function" => Ok(CanonicalKind::Function),
        "method" => Ok(CanonicalKind::Method),
        "value" | "constant" => Ok(CanonicalKind::Value),
        "variable" => Ok(CanonicalKind::Variable),
        "member" => Ok(CanonicalKind::Member),
        "parameter" => Ok(CanonicalKind::Parameter),
        "type_parameter" => Ok(CanonicalKind::TypeParameter),
        "alias" => Ok(CanonicalKind::Alias),
        other => anyhow::bail!("unsupported canonical kind `{other}`"),
    }
}

fn parse_edge_kind(value: &str) -> Result<EdgeKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "imports" => Ok(EdgeKind::Imports),
        "calls" => Ok(EdgeKind::Calls),
        "references" => Ok(EdgeKind::References),
        "extends" | "inherits" => Ok(EdgeKind::Extends),
        "implements" => Ok(EdgeKind::Implements),
        "exports" => Ok(EdgeKind::Exports),
        other => anyhow::bail!("unsupported dependency edge kind `{other}`"),
    }
}
