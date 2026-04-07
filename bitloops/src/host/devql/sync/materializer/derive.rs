use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde_json::Value;

use super::super::content_cache::{CachedArtefact, CachedExtraction};
use super::super::types::DesiredFileState;
use super::sql::non_empty_text;
use super::types::{MaterializedArtefact, MaterializedEdge, PreparedMaterialisationRows};

pub(crate) fn prepare_materialization_rows(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    parser_version: &str,
    extractor_version: &str,
) -> Result<PreparedMaterialisationRows> {
    validate_materialization_inputs(desired, extraction, parser_version, extractor_version)?;

    let materialized_artefacts = derive_materialized_artefacts(cfg, desired, extraction)?;
    let artefacts_by_key = materialized_artefacts
        .iter()
        .map(|artefact| (artefact.artifact_key.clone(), artefact.clone()))
        .collect::<HashMap<_, _>>();
    let materialized_artefacts =
        dedupe_materialized_artefacts_by_artefact_id(materialized_artefacts);
    let materialized_edges =
        derive_materialized_edges(cfg, desired, extraction, &artefacts_by_key)?;
    let materialized_edges = dedupe_materialized_edges_by_edge_id(materialized_edges);

    Ok(PreparedMaterialisationRows {
        materialized_artefacts,
        materialized_edges,
    })
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

fn derive_materialized_artefacts(
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

pub(super) fn parse_cached_language_kind(
    language: &str,
    raw_kind: &str,
) -> Result<crate::host::language_adapter::LanguageKind> {
    use crate::host::language_adapter::{
        GoKind, JavaKind, LanguageKind, PythonKind, RustKind, TsJsKind,
    };

    let parsed = match language {
        "go" => GoKind::from_tree_sitter_kind(raw_kind).map(LanguageKind::go),
        "java" => JavaKind::from_tree_sitter_kind(raw_kind)
            .or_else(|| match raw_kind {
                // Historical caches may carry TS-flavoured names for Java class nodes.
                "class_declaration" => Some(JavaKind::Class),
                _ => None,
            })
            .map(LanguageKind::java),
        "python" => PythonKind::from_tree_sitter_kind(raw_kind).map(LanguageKind::python),
        "rust" => RustKind::from_tree_sitter_kind(raw_kind).map(LanguageKind::rust),
        "typescript" | "javascript" => {
            TsJsKind::from_tree_sitter_kind(raw_kind).map(LanguageKind::ts_js)
        }
        _ => LanguageKind::try_from(raw_kind).ok(),
    };

    parsed.ok_or_else(|| anyhow!("unsupported cached language_kind `{raw_kind}` for `{language}`"))
}

fn derive_materialized_edges(
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
