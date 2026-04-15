use std::collections::HashMap;

use anyhow::Result;
use serde_json::json;

use super::content_cache::{CachedArtefact, CachedEdge, CachedExtraction};

pub(crate) const PARSE_STATUS_OK: &str = "ok";
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const PARSE_STATUS_PARSE_ERROR: &str = "parse_error";
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const PARSE_STATUS_DECODE_ERROR: &str = "decode_error";
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const PARSE_STATUS_DEGRADED_FILE_ONLY: &str = "degraded_file_only";

struct ExtractionInput<'a> {
    path: &'a str,
    content_id: &'a str,
    extraction_fingerprint: &'a str,
    parser_version: &'a str,
    extractor_version: &'a str,
    language: &'a str,
    content: &'a str,
    file_docstring: Option<String>,
}

pub(crate) struct CacheExtractionRequest<'a> {
    pub(crate) path: &'a str,
    pub(crate) language: &'a str,
    pub(crate) content_id: &'a str,
    pub(crate) extraction_fingerprint: &'a str,
    pub(crate) parser_version: &'a str,
    pub(crate) extractor_version: &'a str,
    pub(crate) content: &'a str,
}

pub(crate) fn extract_to_cache_format(
    cfg: &crate::host::devql::DevqlConfig,
    request: CacheExtractionRequest<'_>,
) -> Result<Option<CachedExtraction>> {
    if request.language == crate::host::devql::PLAIN_TEXT_LANGUAGE_ID {
        if !crate::host::devql::plain_text_content_is_allowed(request.content) {
            return Ok(None);
        }
        return Ok(Some(map_extraction_to_cache_format(
            ExtractionInput {
                path: request.path,
                content_id: request.content_id,
                extraction_fingerprint: request.extraction_fingerprint,
                parser_version: request.parser_version,
                extractor_version: request.extractor_version,
                language: request.language,
                content: request.content,
                file_docstring: None,
            },
            Vec::new(),
            Vec::new(),
        )));
    }

    let rev = crate::host::devql::FileRevision {
        commit_sha: request.content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: request.content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path: request.path,
        blob_sha: request.content_id,
    };

    let Some((items, edges, file_docstring)) =
        crate::host::devql::extract_language_pack_artefacts_and_edges(
            cfg,
            &rev,
            request.language,
            request.content,
        )?
    else {
        return Ok(None);
    };

    Ok(Some(map_extraction_to_cache_format(
        ExtractionInput {
            path: request.path,
            content_id: request.content_id,
            extraction_fingerprint: request.extraction_fingerprint,
            parser_version: request.parser_version,
            extractor_version: request.extractor_version,
            language: request.language,
            content: request.content,
            file_docstring,
        },
        items,
        edges,
    )))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_error_to_cache_format(
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
) -> CachedExtraction {
    CachedExtraction {
        content_id: content_id.to_string(),
        language: language.to_string(),
        extraction_fingerprint: extraction_fingerprint.to_string(),
        parser_version: parser_version.to_string(),
        extractor_version: extractor_version.to_string(),
        parse_status: PARSE_STATUS_PARSE_ERROR.to_string(),
        artefacts: Vec::new(),
        edges: Vec::new(),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_error_file_only_to_cache_format(
    path: &str,
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
    raw_bytes: &[u8],
) -> CachedExtraction {
    file_only_to_cache_format(
        PARSE_STATUS_DECODE_ERROR,
        path,
        content_id,
        language,
        extraction_fingerprint,
        parser_version,
        extractor_version,
        raw_bytes,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn degraded_file_only_to_cache_format(
    path: &str,
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
    raw_bytes: &[u8],
) -> CachedExtraction {
    file_only_to_cache_format(
        PARSE_STATUS_DEGRADED_FILE_ONLY,
        path,
        content_id,
        language,
        extraction_fingerprint,
        parser_version,
        extractor_version,
        raw_bytes,
    )
}

fn file_only_to_cache_format(
    parse_status: &str,
    path: &str,
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
    raw_bytes: &[u8],
) -> CachedExtraction {
    CachedExtraction {
        content_id: content_id.to_string(),
        language: language.to_string(),
        extraction_fingerprint: extraction_fingerprint.to_string(),
        parser_version: parser_version.to_string(),
        extractor_version: extractor_version.to_string(),
        parse_status: parse_status.to_string(),
        artefacts: vec![CachedArtefact {
            artifact_key: file_artifact_key_from_bytes(raw_bytes),
            canonical_kind: Some("file".to_string()),
            language_kind: "file".to_string(),
            name: path.to_string(),
            parent_artifact_key: None,
            start_line: 1,
            end_line: file_end_line_from_bytes(raw_bytes),
            start_byte: 0,
            end_byte: i32::try_from(raw_bytes.len()).unwrap_or(i32::MAX),
            signature: String::new(),
            modifiers: Vec::new(),
            docstring: None,
            metadata: json!({ "symbol_fqn": path }),
        }],
        edges: Vec::new(),
    }
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
        extraction_fingerprint: input.extraction_fingerprint.to_string(),
        parser_version: input.parser_version.to_string(),
        extractor_version: input.extractor_version.to_string(),
        parse_status: PARSE_STATUS_OK.to_string(),
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
    file_artifact_key_from_bytes(content.as_bytes())
}

fn file_artifact_key_from_bytes(raw_bytes: &[u8]) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "cache-file|{}|{}",
        file_end_line_from_bytes(raw_bytes),
        raw_bytes.len()
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
    file_end_line_from_bytes(content.as_bytes())
}

fn file_end_line_from_bytes(raw_bytes: &[u8]) -> i32 {
    crate::host::devql::line_count_from_bytes(raw_bytes)
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
                extraction_fingerprint: "fingerprint-v1",
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
                extraction_fingerprint: "fingerprint-v1",
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
        assert_eq!(ordered.parse_status, PARSE_STATUS_OK);
    }

    #[test]
    fn parse_error_cache_payload_has_empty_extraction_data() {
        let parse_error = parse_error_to_cache_format(
            "content-id",
            "typescript",
            "fingerprint-v1",
            "parser-v1",
            "extractor-v1",
        );

        assert_eq!(parse_error.content_id, "content-id");
        assert_eq!(parse_error.language, "typescript");
        assert_eq!(parse_error.extraction_fingerprint, "fingerprint-v1");
        assert_eq!(parse_error.parser_version, "parser-v1");
        assert_eq!(parse_error.extractor_version, "extractor-v1");
        assert_eq!(parse_error.parse_status, PARSE_STATUS_PARSE_ERROR);
        assert!(parse_error.artefacts.is_empty());
        assert!(parse_error.edges.is_empty());
    }

    #[test]
    fn decode_error_cache_payload_materializes_file_only_from_raw_bytes() {
        let decode_error = decode_error_file_only_to_cache_format(
            "src/bad.rs",
            "content-id",
            "rust",
            "fingerprint-v1",
            "parser-v1",
            "extractor-v1",
            &[0x2f, 0x2f, 0xff, 0x0a, 0x66, 0x6e, 0x20, 0x78, 0x0a],
        );

        assert_eq!(decode_error.parse_status, PARSE_STATUS_DECODE_ERROR);
        assert_eq!(decode_error.artefacts.len(), 1);
        assert!(decode_error.edges.is_empty());

        let file = &decode_error.artefacts[0];
        assert_eq!(file.canonical_kind.as_deref(), Some("file"));
        assert_eq!(file.language_kind, "file");
        assert_eq!(file.name, "src/bad.rs");
        assert_eq!(file.start_line, 1);
        assert_eq!(file.end_line, 2);
        assert_eq!(file.start_byte, 0);
        assert_eq!(file.end_byte, 9);
        assert!(file.signature.is_empty());
        assert!(file.modifiers.is_empty());
        assert!(file.docstring.is_none());
    }

    #[test]
    fn degraded_file_only_cache_payload_materializes_file_only_from_raw_bytes() {
        let degraded = degraded_file_only_to_cache_format(
            "scripts/E501_4.py",
            "content-id",
            "python",
            "fingerprint-v1",
            "parser-v1",
            "extractor-v1",
            b"hello\x00world\n",
        );

        assert_eq!(degraded.parse_status, PARSE_STATUS_DEGRADED_FILE_ONLY);
        assert_eq!(degraded.artefacts.len(), 1);
        assert!(degraded.edges.is_empty());

        let file = &degraded.artefacts[0];
        assert_eq!(file.canonical_kind.as_deref(), Some("file"));
        assert_eq!(file.language_kind, "file");
        assert_eq!(file.name, "scripts/E501_4.py");
        assert_eq!(file.start_line, 1);
        assert_eq!(file.end_line, 1);
        assert_eq!(file.start_byte, 0);
        assert_eq!(file.end_byte, 12);
        assert!(file.signature.is_empty());
        assert!(file.modifiers.is_empty());
        assert!(file.docstring.is_none());
    }
}
