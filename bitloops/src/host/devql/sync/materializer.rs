#![allow(dead_code)]

mod derive;
mod persist;
mod sql;
#[cfg(test)]
mod tests;
mod types;
use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde_json::Value;

pub(crate) use self::types::{MaterializedArtefact, MaterializedEdge, PreparedMaterialisationRows};
use super::content_cache::{CachedArtefact, CachedExtraction};
use super::types::DesiredFileState;

pub(crate) async fn materialize_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    persist::materialize_path(
        cfg,
        relational,
        desired,
        extraction,
        parser_version,
        extractor_version,
    )
    .await
}

pub(crate) async fn remove_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    path: &str,
) -> Result<()> {
    persist::remove_path(cfg, relational, path).await
}

fn validate_materialization_inputs(
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    if desired.effective_content_id != extraction.content_id {
        return Err(anyhow!(
            "content mismatch for `{}`: desired effective_content_id `{}` != cached content_id `{}`",
            desired.path,
            desired.effective_content_id,
            extraction.content_id
        ));
    }
    if desired.language != extraction.language {
        return Err(anyhow!(
            "language mismatch for `{}`: desired `{}` != cached `{}`",
            desired.path,
            desired.language,
            extraction.language
        ));
    }
    if desired.extraction_fingerprint != extraction.extraction_fingerprint {
        return Err(anyhow!(
            "extraction fingerprint mismatch for `{}`: desired `{}` != cached `{}`",
            desired.path,
            desired.extraction_fingerprint,
            extraction.extraction_fingerprint
        ));
    }
    if extraction.parser_version != parser_version {
        return Err(anyhow!(
            "parser version mismatch for `{}`: expected `{}` != cached `{}`",
            desired.path,
            parser_version,
            extraction.parser_version
        ));
    }
    if extraction.extractor_version != extractor_version {
        return Err(anyhow!(
            "extractor version mismatch for `{}`: expected `{}` != cached `{}`",
            desired.path,
            extractor_version,
            extraction.extractor_version
        ));
    }
    Ok(())
}

pub(crate) fn derive_materialized_artefacts(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
) -> Result<Vec<MaterializedArtefact>> {
    let by_key = extraction
        .artefacts
        .iter()
        .map(|artefact| (artefact.artifact_key.clone(), artefact))
        .collect::<HashMap<_, _>>();
    let mut resolved = HashMap::<String, MaterializedArtefact>::new();

    for artefact in &extraction.artefacts {
        resolve_artefact(
            cfg,
            desired,
            &extraction.language,
            artefact.artifact_key.as_str(),
            &by_key,
            &mut resolved,
        )?;
    }

    let mut artefacts = resolved.into_values().collect::<Vec<_>>();
    artefacts.sort_by(|lhs, rhs| {
        lhs.symbol_fqn
            .cmp(&rhs.symbol_fqn)
            .then(lhs.artefact_id.cmp(&rhs.artefact_id))
            .then(lhs.artifact_key.cmp(&rhs.artifact_key))
    });
    Ok(artefacts)
}

fn dedupe_materialized_artefacts_by_artefact_id(
    artefacts: Vec<MaterializedArtefact>,
) -> Vec<MaterializedArtefact> {
    let mut seen = HashSet::<String>::new();
    let mut deduped = Vec::new();

    for artefact in artefacts.into_iter().rev() {
        if seen.insert(artefact.artefact_id.clone()) {
            deduped.push(artefact);
        }
    }

    deduped.reverse();
    deduped
}

fn resolve_artefact(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    language: &str,
    artifact_key: &str,
    by_key: &HashMap<String, &CachedArtefact>,
    resolved: &mut HashMap<String, MaterializedArtefact>,
) -> Result<MaterializedArtefact> {
    if let Some(existing) = resolved.get(artifact_key) {
        return Ok(existing.clone());
    }

    let artefact = by_key
        .get(artifact_key)
        .copied()
        .ok_or_else(|| anyhow!("missing cached artefact for key `{artifact_key}`"))?;
    let parent_cached = artefact
        .parent_artifact_key
        .as_deref()
        .and_then(|parent_key| by_key.get(parent_key).copied());
    let parent = artefact
        .parent_artifact_key
        .as_deref()
        .map(|parent_key| resolve_artefact(cfg, desired, language, parent_key, by_key, resolved))
        .transpose()?;
    let symbol_fqn =
        reconstruct_symbol_fqn(artefact, parent_cached, parent.as_ref(), &desired.path);
    let semantic_parent_symbol_id = parent
        .as_ref()
        .filter(|parent| !is_file_artefact(parent))
        .map(|parent| parent.symbol_id.as_str());
    let parent_symbol_id = parent.as_ref().map(|parent| parent.symbol_id.clone());
    let parent_artefact_id = parent.as_ref().map(|parent| parent.artefact_id.clone());
    let symbol_id = if is_file_cached_artefact(artefact) {
        crate::host::devql::file_symbol_id(&desired.path)
    } else {
        let language_kind = parse_cached_language_kind(language, &artefact.language_kind)?;
        let language_artefact = crate::host::language_adapter::LanguageArtefact {
            canonical_kind: artefact.canonical_kind.clone(),
            language_kind,
            name: artefact.name.clone(),
            symbol_fqn: symbol_fqn.clone(),
            parent_symbol_fqn: None,
            start_line: artefact.start_line,
            end_line: artefact.end_line,
            start_byte: artefact.start_byte,
            end_byte: artefact.end_byte,
            signature: artefact.signature.clone(),
            modifiers: artefact.modifiers.clone(),
            docstring: artefact.docstring.clone(),
        };
        crate::host::devql::structural_symbol_id_for_artefact(
            &language_artefact,
            semantic_parent_symbol_id,
        )
    };
    let materialized = MaterializedArtefact {
        artifact_key: artefact.artifact_key.clone(),
        artefact_id: crate::host::devql::revision_artefact_id(
            &cfg.repo.repo_id,
            &desired.effective_content_id,
            &symbol_id,
        ),
        symbol_id,
        canonical_kind: artefact.canonical_kind.clone(),
        language_kind: artefact.language_kind.clone(),
        symbol_fqn,
        parent_symbol_id,
        parent_artefact_id,
        start_line: artefact.start_line,
        end_line: artefact.end_line,
        start_byte: artefact.start_byte,
        end_byte: artefact.end_byte,
        signature: non_empty_text(&artefact.signature),
        modifiers: artefact.modifiers.clone(),
        docstring: artefact.docstring.clone(),
    };
    resolved.insert(artifact_key.to_string(), materialized.clone());
    Ok(materialized)
}

pub(crate) fn derive_materialized_edges(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    artefacts_by_key: &HashMap<String, MaterializedArtefact>,
) -> Result<Vec<MaterializedEdge>> {
    let mut deduped = HashMap::<String, MaterializedEdge>::new();
    for edge in &extraction.edges {
        let Some(from) = artefacts_by_key.get(&edge.from_artifact_key) else {
            continue;
        };
        let to = edge
            .to_artifact_key
            .as_ref()
            .and_then(|artifact_key| artefacts_by_key.get(artifact_key));
        let to_symbol_id = to.as_ref().map(|artefact| artefact.symbol_id.clone());
        let to_artefact_id = to.as_ref().map(|artefact| artefact.artefact_id.clone());
        let to_symbol_ref = edge.to_symbol_ref.clone();
        if to_symbol_id.is_none() && to_symbol_ref.is_none() {
            continue;
        }

        let metadata_key = edge.metadata.to_string();
        let materialized = MaterializedEdge {
            edge_id: crate::host::devql::deterministic_uuid(&format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                cfg.repo.repo_id,
                desired.path,
                from.symbol_id,
                edge.edge_kind,
                to_symbol_id.clone().unwrap_or_default(),
                to_symbol_ref.clone().unwrap_or_default(),
                edge.start_line.unwrap_or(-1),
                edge.end_line.unwrap_or(-1),
                metadata_key,
            )),
            from_symbol_id: from.symbol_id.clone(),
            from_artefact_id: from.artefact_id.clone(),
            to_symbol_id,
            to_artefact_id,
            to_symbol_ref,
            edge_kind: edge.edge_kind.clone(),
            language: extraction.language.clone(),
            start_line: edge.start_line,
            end_line: edge.end_line,
            metadata: edge.metadata.clone(),
        };
        deduped.insert(materialized.edge_id.clone(), materialized);
    }

    let mut edges = deduped.into_values().collect::<Vec<_>>();
    edges.sort_by(|lhs, rhs| lhs.edge_id.cmp(&rhs.edge_id));
    Ok(edges)
}

fn parse_cached_language_kind(
    language: &str,
    raw_kind: &str,
) -> Result<crate::host::language_adapter::LanguageKind> {
    derive::parse_cached_language_kind(language, raw_kind)
}

fn dedupe_materialized_edges_by_edge_id(edges: Vec<MaterializedEdge>) -> Vec<MaterializedEdge> {
    let mut deduped = HashMap::<String, MaterializedEdge>::new();
    for edge in edges {
        deduped.insert(edge.edge_id.clone(), edge);
    }

    let mut edges = deduped.into_values().collect::<Vec<_>>();
    edges.sort_by(|lhs, rhs| lhs.edge_id.cmp(&rhs.edge_id));
    edges
}

fn reconstruct_symbol_fqn(
    artefact: &CachedArtefact,
    parent_cached: Option<&CachedArtefact>,
    parent_materialized: Option<&MaterializedArtefact>,
    path: &str,
) -> String {
    if is_file_cached_artefact(artefact) {
        return path.to_string();
    }

    let helper_suffix = cached_symbol_fqn_helper(artefact).and_then(|helper| {
        if let Some(parent_helper) = parent_cached.and_then(cached_symbol_fqn_helper) {
            helper
                .strip_prefix(&format!("{parent_helper}::"))
                .map(str::to_string)
        } else {
            helper
                .split_once("::")
                .map(|(_, suffix)| suffix.to_string())
        }
    });
    let local_suffix = helper_suffix
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or_else(|| fallback_local_symbol_suffix(artefact));

    if let Some(parent) = parent_materialized {
        format!("{}::{}", parent.symbol_fqn, local_suffix)
    } else {
        format!("{path}::{}", local_suffix)
    }
}

fn cached_symbol_fqn_helper(artefact: &CachedArtefact) -> Option<&str> {
    artefact.metadata.get("symbol_fqn").and_then(Value::as_str)
}

fn fallback_local_symbol_suffix(artefact: &CachedArtefact) -> String {
    if is_import_like_artefact(artefact) {
        format!("import::{}", artefact.name)
    } else {
        artefact.name.clone()
    }
}

fn is_import_like_artefact(artefact: &CachedArtefact) -> bool {
    artefact.canonical_kind.as_deref() == Some("import")
        || artefact.language_kind.contains("import")
}

fn is_file_cached_artefact(artefact: &CachedArtefact) -> bool {
    artefact.canonical_kind.as_deref() == Some("file") && artefact.language_kind == "file"
}

fn is_file_artefact(artefact: &MaterializedArtefact) -> bool {
    artefact.canonical_kind.as_deref() == Some("file") && artefact.language_kind == "file"
}

fn insert_edge_sql(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
    edge: &MaterializedEdge,
    now_sql: &str,
) -> String {
    let to_symbol_id_sql = nullable_text_sql(edge.to_symbol_id.as_deref());
    let to_artefact_id_sql = nullable_text_sql(edge.to_artefact_id.as_deref());
    let to_symbol_ref_sql = nullable_text_sql(edge.to_symbol_ref.as_deref());
    let start_line_sql = nullable_i32_sql(edge.start_line);
    let end_line_sql = nullable_i32_sql(edge.end_line);
    let metadata_sql = crate::host::devql::sql_json_value(relational, &edge.metadata);

    format!(
        "INSERT INTO artefact_edges_current (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, {})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(&edge.edge_id),
        crate::host::devql::esc_pg(path),
        crate::host::devql::esc_pg(content_id),
        crate::host::devql::esc_pg(&edge.from_symbol_id),
        crate::host::devql::esc_pg(&edge.from_artefact_id),
        to_symbol_id_sql,
        to_artefact_id_sql,
        to_symbol_ref_sql,
        crate::host::devql::esc_pg(&edge.edge_kind),
        crate::host::devql::esc_pg(&edge.language),
        start_line_sql,
        end_line_sql,
        metadata_sql,
        now_sql,
    )
}

fn upsert_current_file_state_sql(
    repo_id: &str,
    desired: &DesiredFileState,
    parser_version: &str,
    extractor_version: &str,
    now_sql: &str,
) -> String {
    let head_content_id_sql = nullable_text_sql(desired.head_content_id.as_deref());
    let index_content_id_sql = nullable_text_sql(desired.index_content_id.as_deref());
    let worktree_content_id_sql = nullable_text_sql(desired.worktree_content_id.as_deref());
    let dialect_sql = nullable_text_sql(desired.dialect.as_deref());
    let primary_context_sql = nullable_text_sql(desired.primary_context_id.as_deref());
    let runtime_profile_sql = nullable_text_sql(desired.runtime_profile.as_deref());
    let context_fingerprint_sql = nullable_text_sql(desired.context_fingerprint.as_deref());
    format!(
        "INSERT INTO current_file_state (repo_id, path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect, primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason, context_fingerprint, extraction_fingerprint, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree, last_synced_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, '{}', '{}', {}, '{}', {}, '{}', {}, {}, {}, '{}', '{}', '{}', '{}', {}, {}, {}, {}) \
ON CONFLICT (repo_id, path) DO UPDATE SET analysis_mode = EXCLUDED.analysis_mode, file_role = EXCLUDED.file_role, text_index_mode = EXCLUDED.text_index_mode, language = EXCLUDED.language, resolved_language = EXCLUDED.resolved_language, dialect = EXCLUDED.dialect, primary_context_id = EXCLUDED.primary_context_id, secondary_context_ids_json = EXCLUDED.secondary_context_ids_json, frameworks_json = EXCLUDED.frameworks_json, runtime_profile = EXCLUDED.runtime_profile, classification_reason = EXCLUDED.classification_reason, context_fingerprint = EXCLUDED.context_fingerprint, extraction_fingerprint = EXCLUDED.extraction_fingerprint, head_content_id = EXCLUDED.head_content_id, index_content_id = EXCLUDED.index_content_id, worktree_content_id = EXCLUDED.worktree_content_id, effective_content_id = EXCLUDED.effective_content_id, effective_source = EXCLUDED.effective_source, parser_version = EXCLUDED.parser_version, extractor_version = EXCLUDED.extractor_version, exists_in_head = EXCLUDED.exists_in_head, exists_in_index = EXCLUDED.exists_in_index, exists_in_worktree = EXCLUDED.exists_in_worktree, last_synced_at = EXCLUDED.last_synced_at",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(&desired.path),
        crate::host::devql::esc_pg(desired.analysis_mode.as_str()),
        crate::host::devql::esc_pg(desired.file_role.as_str()),
        crate::host::devql::esc_pg(desired.text_index_mode.as_str()),
        crate::host::devql::esc_pg(&desired.language),
        crate::host::devql::esc_pg(&desired.resolved_language),
        dialect_sql,
        primary_context_sql,
        crate::host::devql::esc_pg(
            &serde_json::to_string(&desired.secondary_context_ids)
                .unwrap_or_else(|_| "[]".to_string()),
        ),
        crate::host::devql::esc_pg(
            &serde_json::to_string(&desired.frameworks).unwrap_or_else(|_| "[]".to_string()),
        ),
        runtime_profile_sql,
        crate::host::devql::esc_pg(&desired.classification_reason),
        context_fingerprint_sql,
        crate::host::devql::esc_pg(&desired.extraction_fingerprint),
        head_content_id_sql,
        index_content_id_sql,
        worktree_content_id_sql,
        crate::host::devql::esc_pg(&desired.effective_content_id),
        crate::host::devql::esc_pg(desired.effective_source.as_str()),
        crate::host::devql::esc_pg(parser_version),
        crate::host::devql::esc_pg(extractor_version),
        bool_sql(desired.exists_in_head),
        bool_sql(desired.exists_in_index),
        bool_sql(desired.exists_in_worktree),
        now_sql,
    )
}

fn delete_edges_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND path = '{}'",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
    )
}

fn delete_artefacts_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM artefacts_current WHERE repo_id = '{}' AND path = '{}'",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
    )
}

fn delete_current_file_state_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM current_file_state WHERE repo_id = '{}' AND path = '{}'",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
    )
}

fn nullable_text_sql(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", crate::host::devql::esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn nullable_i32_sql(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn bool_sql(value: bool) -> i32 {
    if value { 1 } else { 0 }
}

fn non_empty_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) use self::derive::prepare_materialization_rows;
pub(crate) use self::persist::{
    persist_prepared_materialisation_tx, remove_paths_tx,
    resolve_prepared_local_edges_with_connection,
};
