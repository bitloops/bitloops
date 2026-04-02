use std::collections::HashMap;

use anyhow::Result;
use serde_json::json;

use super::content_cache::{CachedArtefact, CachedEdge, CachedExtraction};

struct ExtractionInput<'a> {
    path: &'a str,
    content_id: &'a str,
    parser_version: &'a str,
    extractor_version: &'a str,
    language: &'a str,
    content: &'a str,
    file_docstring: Option<String>,
}

pub(crate) fn extract_to_cache_format(
    cfg: &crate::host::devql::DevqlConfig,
    path: &str,
    content_id: &str,
    parser_version: &str,
    extractor_version: &str,
    content: &str,
) -> Result<Option<CachedExtraction>> {
    let language = crate::host::devql::detect_language(path);
    let rev = crate::host::devql::FileRevision {
        commit_sha: content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: content_id,
    };

    let Some((items, edges, file_docstring)) =
        crate::host::devql::extract_language_pack_artefacts_and_edges(
            cfg, &rev, &language, content,
        )?
    else {
        return Ok(None);
    };

    Ok(Some(map_extraction_to_cache_format(
        ExtractionInput {
            path,
            content_id,
            parser_version,
            extractor_version,
            language: &language,
            content,
            file_docstring,
        },
        items,
        edges,
    )))
}

fn map_extraction_to_cache_format(
    input: ExtractionInput<'_>,
    items: Vec<crate::host::language_adapter::LanguageArtefact>,
    edges: Vec<crate::host::language_adapter::DependencyEdge>,
) -> CachedExtraction {
    let file_artifact_key = file_artifact_key(input.content);
    let mut symbol_to_artifact_key =
        HashMap::from([(input.path.to_string(), file_artifact_key.clone())]);

    for (symbol_fqn, artifact_key) in assign_artifact_keys(&items) {
        symbol_to_artifact_key.insert(symbol_fqn, artifact_key);
    }

    let mut artefacts = vec![CachedArtefact {
        artifact_key: file_artifact_key.clone(),
        canonical_kind: Some("file".to_string()),
        language_kind: "file".to_string(),
        name: input.path.to_string(),
        parent_artifact_key: None,
        start_line: 1,
        end_line: file_end_line(input.content),
        start_byte: 0,
        end_byte: input.content.len() as i32,
        signature: String::new(),
        modifiers: Vec::new(),
        docstring: input.file_docstring,
        metadata: json!({ "symbol_fqn": input.path }),
    }];

    artefacts.extend(items.iter().map(|item| {
        CachedArtefact {
            artifact_key: symbol_to_artifact_key
                .get(&item.symbol_fqn)
                .cloned()
                .expect("artifact key should exist for extracted item"),
            canonical_kind: item.canonical_kind.clone(),
            language_kind: item.language_kind.to_string(),
            name: item.name.clone(),
            parent_artifact_key: item
                .parent_symbol_fqn
                .as_ref()
                .and_then(|fqn| symbol_to_artifact_key.get(fqn))
                .cloned()
                .or_else(|| Some(file_artifact_key.clone())),
            start_line: item.start_line,
            end_line: item.end_line,
            start_byte: item.start_byte,
            end_byte: item.end_byte,
            signature: item.signature.clone(),
            modifiers: item.modifiers.clone(),
            docstring: item.docstring.clone(),
            metadata: json!({ "symbol_fqn": item.symbol_fqn }),
        }
    }));

    let mut edges = edges
        .into_iter()
        .filter_map(|edge| cached_edge_from_extraction(edge, &symbol_to_artifact_key))
        .collect::<Vec<_>>();

    artefacts.sort_by(|lhs, rhs| lhs.artifact_key.cmp(&rhs.artifact_key));
    edges.sort_by(|lhs, rhs| lhs.edge_key.cmp(&rhs.edge_key));

    CachedExtraction {
        content_id: input.content_id.to_string(),
        language: input.language.to_string(),
        parser_version: input.parser_version.to_string(),
        extractor_version: input.extractor_version.to_string(),
        parse_status: "parsed".to_string(),
        artefacts,
        edges,
    }
}

fn assign_artifact_keys(
    items: &[crate::host::language_adapter::LanguageArtefact],
) -> Vec<(String, String)> {
    let mut ranked = items
        .iter()
        .map(|item| (local_artifact_fingerprint(item), item.symbol_fqn.clone()))
        .collect::<Vec<_>>();
    ranked.sort();

    let mut counters: HashMap<String, usize> = HashMap::new();
    ranked
        .into_iter()
        .map(|(fingerprint, symbol_fqn)| {
            let ordinal = counters
                .entry(fingerprint.clone())
                .and_modify(|count| *count += 1)
                .or_insert(0);
            let artifact_key = crate::host::devql::deterministic_uuid(&format!(
                "cache-artefact|{fingerprint}|{}",
                *ordinal
            ));
            (symbol_fqn, artifact_key)
        })
        .collect()
}

fn cached_edge_from_extraction(
    edge: crate::host::language_adapter::DependencyEdge,
    symbol_to_artifact_key: &HashMap<String, String>,
) -> Option<CachedEdge> {
    let from_artifact_key = symbol_to_artifact_key.get(&edge.from_symbol_fqn)?.clone();
    let to_artifact_key = edge
        .to_target_symbol_fqn
        .as_ref()
        .and_then(|fqn| symbol_to_artifact_key.get(fqn))
        .cloned();
    let to_symbol_ref = if to_artifact_key.is_some() {
        None
    } else {
        edge.to_symbol_ref
            .clone()
            .or(edge.to_target_symbol_fqn.clone())
    };
    if to_artifact_key.is_none() && to_symbol_ref.is_none() {
        return None;
    }

    let metadata = edge.metadata.to_value();
    let edge_key = crate::host::devql::deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}|{}",
        from_artifact_key,
        edge.edge_kind.as_str(),
        to_artifact_key.clone().unwrap_or_default(),
        to_symbol_ref.clone().unwrap_or_default(),
        edge.start_line.unwrap_or(-1),
        edge.end_line.unwrap_or(-1),
        metadata,
    ));

    Some(CachedEdge {
        edge_key,
        from_artifact_key,
        to_artifact_key,
        to_symbol_ref,
        edge_kind: edge.edge_kind.as_str().to_string(),
        start_line: edge.start_line,
        end_line: edge.end_line,
        metadata,
    })
}

fn file_artifact_key(content: &str) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "cache-file|{}|{}",
        file_end_line(content),
        content.len()
    ))
}

fn local_artifact_fingerprint(item: &crate::host::language_adapter::LanguageArtefact) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        item.canonical_kind.as_deref().unwrap_or("<null>"),
        item.language_kind,
        crate::host::devql::semantic_name_for_artefact(item),
        item.start_line,
        item.end_line,
        item.start_byte,
        item.end_byte,
        crate::host::devql::normalize_identity_fragment(
            &crate::host::devql::identity_signature_for_artefact(item)
        ),
        serde_json::to_string(&item.modifiers).unwrap_or_else(|_| "[]".to_string()),
        item.docstring.as_deref().unwrap_or("")
    )
}

fn file_end_line(content: &str) -> i32 {
    (content.lines().count() as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::devql::EdgeKind;
    use crate::host::language_adapter::{
        DependencyEdge, EdgeMetadata, LanguageArtefact, LanguageKind, TsJsKind,
    };

    fn class_artefact() -> LanguageArtefact {
        LanguageArtefact {
            canonical_kind: None,
            language_kind: LanguageKind::ts_js(TsJsKind::ClassDeclaration),
            name: "Service".to_string(),
            symbol_fqn: "src/sample.ts::Service".to_string(),
            parent_symbol_fqn: None,
            start_line: 1,
            end_line: 5,
            start_byte: 0,
            end_byte: 72,
            signature: "class Service {".to_string(),
            modifiers: vec![],
            docstring: None,
        }
    }

    fn method_artefact() -> LanguageArtefact {
        LanguageArtefact {
            canonical_kind: Some("method".to_string()),
            language_kind: LanguageKind::ts_js(TsJsKind::MethodDefinition),
            name: "run".to_string(),
            symbol_fqn: "src/sample.ts::Service::run".to_string(),
            parent_symbol_fqn: Some("src/sample.ts::Service".to_string()),
            start_line: 2,
            end_line: 4,
            start_byte: 18,
            end_byte: 68,
            signature: "run(): number {".to_string(),
            modifiers: vec![],
            docstring: None,
        }
    }

    fn helper_artefact() -> LanguageArtefact {
        LanguageArtefact {
            canonical_kind: Some("function".to_string()),
            language_kind: LanguageKind::ts_js(TsJsKind::FunctionDeclaration),
            name: "localHelper".to_string(),
            symbol_fqn: "src/sample.ts::localHelper".to_string(),
            parent_symbol_fqn: None,
            start_line: 7,
            end_line: 9,
            start_byte: 74,
            end_byte: 118,
            signature: "function localHelper(): number {".to_string(),
            modifiers: vec![],
            docstring: None,
        }
    }

    fn sample_edges() -> Vec<DependencyEdge> {
        vec![
            DependencyEdge {
                edge_kind: EdgeKind::Calls,
                from_symbol_fqn: "src/sample.ts::Service::run".to_string(),
                to_target_symbol_fqn: Some("src/sample.ts::localHelper".to_string()),
                to_symbol_ref: None,
                start_line: Some(3),
                end_line: Some(3),
                metadata: EdgeMetadata::none(),
            },
            DependencyEdge {
                edge_kind: EdgeKind::Calls,
                from_symbol_fqn: "src/sample.ts::Service::run".to_string(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some("./remote::remoteFoo".to_string()),
                start_line: Some(3),
                end_line: Some(3),
                metadata: EdgeMetadata::none(),
            },
        ]
    }

    #[test]
    fn sync_extraction_parent_mapping_is_order_independent() {
        let content = "class Service {\n  run(): number {\n    return localHelper() + remoteFoo();\n  }\n}\n\nfunction localHelper(): number {\n  return 1;\n}\n";
        let ordered = map_extraction_to_cache_format(
            ExtractionInput {
                path: "src/sample.ts",
                content_id: "content-id",
                parser_version: "parser-v1",
                extractor_version: "extractor-v1",
                language: "typescript",
                content,
                file_docstring: None,
            },
            vec![class_artefact(), method_artefact(), helper_artefact()],
            sample_edges(),
        );
        let reversed = map_extraction_to_cache_format(
            ExtractionInput {
                path: "src/sample.ts",
                content_id: "content-id",
                parser_version: "parser-v1",
                extractor_version: "extractor-v1",
                language: "typescript",
                content,
                file_docstring: None,
            },
            vec![method_artefact(), helper_artefact(), class_artefact()],
            sample_edges(),
        );

        assert_eq!(ordered, reversed);
    }
}
