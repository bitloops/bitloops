#![allow(private_interfaces)]

use std::collections::HashMap;

use super::*;
use crate::host::devql::sync::content_cache::{CachedArtefact, CachedEdge, CachedExtraction};
use crate::host::devql::sync::types::{DesiredFileState, EffectiveSource};
use serde_json::{Map, Value};

const COMPAT_PARSER_VERSION: &str = "compat-parser";
const COMPAT_EXTRACTOR_VERSION: &str = "compat-extractor";

pub(crate) async fn upsert_current_state_for_content(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    content: &str,
) -> Result<()> {
    let content_id = rev.blob_sha.to_string();
    let language = detect_language(rev.path);
    let extraction = match crate::host::devql::sync::extraction::extract_to_cache_format(
        cfg,
        rev.path,
        &content_id,
        COMPAT_PARSER_VERSION,
        COMPAT_EXTRACTOR_VERSION,
        content,
    )? {
        Some(value) => value,
        None => {
            let file_artefact = build_file_artefact_row_from_content(
                &cfg.repo.repo_id,
                rev.path,
                rev.blob_sha,
                Some(content),
            );
            let file_record =
                build_file_current_record(rev.path, rev.blob_sha, &file_artefact, None);
            CachedExtraction {
                content_id: content_id.clone(),
                language: language.to_string(),
                parser_version: COMPAT_PARSER_VERSION.to_string(),
                extractor_version: COMPAT_EXTRACTOR_VERSION.to_string(),
                parse_status: "ok".to_string(),
                artefacts: vec![cached_artefact_from_record(&file_record)],
                edges: Vec::new(),
            }
        }
    };

    let desired = DesiredFileState {
        path: rev.path.to_string(),
        language: language.to_string(),
        head_content_id: Some(content_id.clone()),
        index_content_id: Some(content_id.clone()),
        worktree_content_id: Some(content_id.clone()),
        effective_content_id: content_id,
        effective_source: EffectiveSource::Worktree,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    };
    crate::host::devql::sync::materializer::materialize_path(
        cfg,
        relational,
        &desired,
        &extraction,
        COMPAT_PARSER_VERSION,
        COMPAT_EXTRACTOR_VERSION,
    )
    .await
}

pub(crate) async fn refresh_current_state_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
    file_docstring: Option<String>,
    symbol_records: &[PersistedArtefactRecord],
    edges: Vec<DependencyEdge>,
) -> Result<()> {
    let mut all_records = Vec::with_capacity(symbol_records.len() + 1);
    all_records.push(build_file_current_record(
        rev.path,
        rev.blob_sha,
        file_artefact,
        file_docstring,
    ));
    all_records.extend(symbol_records.iter().cloned());

    let mut by_fqn = HashMap::new();
    for record in &all_records {
        by_fqn.insert(record.symbol_fqn.clone(), record.clone());
    }
    let external_targets: HashMap<String, (String, String)> = HashMap::new();
    let edge_records = build_current_edge_records(
        cfg,
        rev.path,
        &file_artefact.language,
        edges,
        &by_fqn,
        &external_targets,
    );

    let extraction = CachedExtraction {
        content_id: rev.blob_sha.to_string(),
        language: file_artefact.language.clone(),
        parser_version: COMPAT_PARSER_VERSION.to_string(),
        extractor_version: COMPAT_EXTRACTOR_VERSION.to_string(),
        parse_status: "ok".to_string(),
        artefacts: all_records.iter().map(cached_artefact_from_record).collect(),
        edges: edge_records
            .iter()
            .map(cached_edge_from_record)
            .collect::<Vec<_>>(),
    };
    let desired = DesiredFileState {
        path: rev.path.to_string(),
        language: file_artefact.language.clone(),
        head_content_id: Some(rev.blob_sha.to_string()),
        index_content_id: Some(rev.blob_sha.to_string()),
        worktree_content_id: Some(rev.blob_sha.to_string()),
        effective_content_id: rev.blob_sha.to_string(),
        effective_source: EffectiveSource::Worktree,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    };
    crate::host::devql::sync::materializer::materialize_path(
        cfg,
        relational,
        &desired,
        &extraction,
        COMPAT_PARSER_VERSION,
        COMPAT_EXTRACTOR_VERSION,
    )
    .await
}

fn cached_artefact_from_record(record: &PersistedArtefactRecord) -> CachedArtefact {
    CachedArtefact {
        artifact_key: record.symbol_id.clone(),
        canonical_kind: record.canonical_kind.clone(),
        language_kind: record.language_kind.clone(),
        name: record
            .symbol_fqn
            .rsplit("::")
            .next()
            .unwrap_or(record.symbol_fqn.as_str())
            .to_string(),
        parent_artifact_key: record.parent_symbol_id.clone(),
        start_line: record.start_line,
        end_line: record.end_line,
        start_byte: record.start_byte,
        end_byte: record.end_byte,
        signature: record.signature.clone().unwrap_or_default(),
        modifiers: record.modifiers.clone(),
        docstring: record.docstring.clone(),
        metadata: Value::Object(Map::new()),
    }
}

fn cached_edge_from_record(record: &PersistedEdgeRecord) -> CachedEdge {
    CachedEdge {
        edge_key: record.edge_id.clone(),
        from_artifact_key: record.from_symbol_id.clone(),
        to_artifact_key: record.to_symbol_id.clone(),
        to_symbol_ref: record.to_symbol_ref.clone(),
        edge_kind: record.edge_kind.clone(),
        start_line: record.start_line,
        end_line: record.end_line,
        metadata: record.metadata.clone(),
    }
}

pub(crate) fn build_current_edge_records(
    cfg: &DevqlConfig,
    path: &str,
    language: &str,
    edges: Vec<DependencyEdge>,
    current_by_fqn: &HashMap<String, PersistedArtefactRecord>,
    external_targets: &HashMap<String, (String, String)>,
) -> Vec<PersistedEdgeRecord> {
    let mut out = Vec::new();

    for edge in edges {
        let Some(from_record) = current_by_fqn.get(&edge.from_symbol_fqn) else {
            continue;
        };

        let fallback_ref = edge
            .to_symbol_ref
            .clone()
            .or_else(|| edge.to_target_symbol_fqn.clone());
        let resolved_target = edge
            .to_target_symbol_fqn
            .as_ref()
            .and_then(|fqn| current_by_fqn.get(fqn))
            .map(|record| (record.symbol_id.clone(), record.artefact_id.clone()))
            .or_else(|| {
                fallback_ref
                    .as_ref()
                    .and_then(|symbol_ref| external_targets.get(symbol_ref).cloned())
            });

        if resolved_target.is_none() && fallback_ref.is_none() {
            continue;
        }

        let to_symbol_id = resolved_target
            .as_ref()
            .map(|(symbol_id, _)| symbol_id.clone());
        let to_artefact_id = resolved_target
            .as_ref()
            .map(|(_, artefact_id)| artefact_id.clone());
        let to_symbol_ref = fallback_ref.clone();
        let metadata = edge.metadata.to_value();
        let metadata_key = metadata.to_string();

        out.push(PersistedEdgeRecord {
            edge_id: deterministic_uuid(&format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                cfg.repo.repo_id,
                path,
                from_record.symbol_id,
                edge.edge_kind.as_str(),
                to_symbol_id.clone().unwrap_or_default(),
                to_symbol_ref.clone().unwrap_or_default(),
                edge.start_line.unwrap_or(-1),
                edge.end_line.unwrap_or(-1),
                metadata_key,
            )),
            from_symbol_id: from_record.symbol_id.clone(),
            from_artefact_id: from_record.artefact_id.clone(),
            to_symbol_id,
            to_artefact_id,
            to_symbol_ref,
            edge_kind: edge.edge_kind.as_str().to_string(),
            language: language.to_string(),
            start_line: edge.start_line,
            end_line: edge.end_line,
            metadata,
        });
    }

    out
}

pub(crate) fn incoming_revision_is_newer(
    existing: Option<&CurrentFileRevisionRecord>,
    revision_kind: TemporalRevisionKind,
    revision_id: &str,
    revision_unix: i64,
) -> bool {
    match existing {
        None => true,
        Some(existing) => match (revision_kind, existing.revision_kind) {
            (TemporalRevisionKind::Commit, TemporalRevisionKind::Temporary) => true,
            (TemporalRevisionKind::Temporary, TemporalRevisionKind::Commit) => {
                revision_unix >= existing.updated_at_unix
            }
            _ => {
                revision_unix > existing.updated_at_unix
                    || (revision_unix == existing.updated_at_unix
                        && revision_id_is_newer(revision_id, &existing.revision_id))
            }
        },
    }
}

fn revision_id_is_newer(incoming: &str, existing: &str) -> bool {
    match (
        incoming
            .strip_prefix("temp:")
            .and_then(|value| value.parse::<u64>().ok()),
        existing
            .strip_prefix("temp:")
            .and_then(|value| value.parse::<u64>().ok()),
    ) {
        (Some(incoming_idx), Some(existing_idx)) => incoming_idx > existing_idx,
        _ => incoming > existing,
    }
}

pub(crate) async fn promote_temporary_current_rows_for_head_commit(
    _cfg: &DevqlConfig,
    _relational: &RelationalStorage,
) -> Result<usize> {
    Ok(0)
}
