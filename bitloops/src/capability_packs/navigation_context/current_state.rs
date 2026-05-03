use std::collections::BTreeMap;

use anyhow::Context;
use serde_json::{Value, json};

use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};
use crate::host::language_adapter::{LanguageEntryPointArtefact, LanguageEntryPointFile};
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

use super::storage::{
    NavigationEdgeFact, NavigationFacts, NavigationPrimitiveFact, edge_id, primitive_id,
    replace_navigation_context_current, stable_hash,
};
use super::types::{
    NAVIGATION_CONTEXT_CAPABILITY_ID, NAVIGATION_CONTEXT_CONSUMER_ID, NavigationPrimitiveKind,
};

pub struct NavigationContextCurrentStateConsumer;

impl CurrentStateConsumer for NavigationContextCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        NAVIGATION_CONTEXT_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        NAVIGATION_CONTEXT_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let files = context
                .relational
                .load_current_canonical_files(&request.repo_id)
                .context("loading current files for navigation context")?;
            let artefacts = context
                .relational
                .load_current_canonical_artefacts(&request.repo_id)
                .context("loading current artefacts for navigation context")?;
            let dependency_edges = context
                .relational
                .load_current_canonical_edges(&request.repo_id)
                .context("loading current dependency edges for navigation context")?;

            let mut builder = NavigationBuilder::new(
                &request.repo_id,
                request.to_generation_seq_inclusive,
                request
                    .run_id
                    .as_deref()
                    .unwrap_or(NAVIGATION_CONTEXT_CONSUMER_ID),
            );
            builder.add_files(&files);
            builder.add_packages(&files);
            builder.add_symbols(&artefacts);
            builder.add_dependency_edges(&dependency_edges);
            builder.add_entrypoints(context, &files, &artefacts);

            let facts = builder.finish();
            let metrics = json!({
                "primitives": facts.primitives.len(),
                "edges": facts.edges.len(),
                "files": files.len(),
                "artefacts": artefacts.len(),
                "dependency_edges": dependency_edges.len(),
                "reconcile_mode": format!("{:?}", request.reconcile_mode),
            });
            let warnings = Vec::new();
            replace_navigation_context_current(
                &context.storage,
                &request.repo_id,
                facts,
                request.to_generation_seq_inclusive,
                &warnings,
                metrics.clone(),
            )
            .await?;

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(metrics),
            })
        })
    }
}

struct NavigationBuilder {
    repo_id: String,
    generation: u64,
    run_id: String,
    primitives: BTreeMap<String, NavigationPrimitiveFact>,
    edges: BTreeMap<String, NavigationEdgeFact>,
    file_primitives_by_path: BTreeMap<String, String>,
    symbol_primitives_by_symbol_id: BTreeMap<String, String>,
    symbol_primitives_by_artefact_id: BTreeMap<String, String>,
}

impl NavigationBuilder {
    fn new(repo_id: &str, generation: u64, run_id: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            generation,
            run_id: run_id.to_string(),
            primitives: BTreeMap::new(),
            edges: BTreeMap::new(),
            file_primitives_by_path: BTreeMap::new(),
            symbol_primitives_by_symbol_id: BTreeMap::new(),
            symbol_primitives_by_artefact_id: BTreeMap::new(),
        }
    }

    fn finish(self) -> NavigationFacts {
        NavigationFacts {
            primitives: self.primitives.into_values().collect(),
            edges: self.edges.into_values().collect(),
        }
    }

    fn add_files(&mut self, files: &[CurrentCanonicalFileRecord]) {
        for file in files {
            let id = primitive_id(&self.repo_id, NavigationPrimitiveKind::FileBlob, &file.path);
            self.file_primitives_by_path
                .insert(file.path.clone(), id.clone());
            self.upsert_primitive(NavigationPrimitiveFact {
                repo_id: self.repo_id.clone(),
                primitive_id: id,
                primitive_kind: NavigationPrimitiveKind::FileBlob.as_str().to_string(),
                identity_key: format!("file:{}", file.path),
                label: file.path.clone(),
                path: Some(file.path.clone()),
                artefact_id: None,
                symbol_id: None,
                source_kind: "DEVQL_CURRENT_STATE".to_string(),
                confidence: 1.0,
                primitive_hash: stable_hash(&[
                    NavigationPrimitiveKind::FileBlob.as_str(),
                    &file.path,
                    &file.language,
                    &file.resolved_language,
                    &file.effective_content_id,
                    &file.parser_version,
                    &file.extractor_version,
                    bool_str(file.exists_in_head),
                    bool_str(file.exists_in_index),
                    bool_str(file.exists_in_worktree),
                ]),
                properties: json!({
                    "analysis_mode": &file.analysis_mode,
                    "file_role": &file.file_role,
                    "language": &file.language,
                    "resolved_language": &file.resolved_language,
                    "effective_content_id": &file.effective_content_id,
                    "parser_version": &file.parser_version,
                    "extractor_version": &file.extractor_version,
                    "exists_in_head": file.exists_in_head,
                    "exists_in_index": file.exists_in_index,
                    "exists_in_worktree": file.exists_in_worktree,
                }),
                provenance: self.provenance("current_file_state"),
                last_observed_generation: Some(self.generation),
            });
        }
    }

    fn add_packages(&mut self, files: &[CurrentCanonicalFileRecord]) {
        for file in files.iter().filter(|file| is_package_manifest(&file.path)) {
            let id = primitive_id(&self.repo_id, NavigationPrimitiveKind::Package, &file.path);
            self.upsert_primitive(NavigationPrimitiveFact {
                repo_id: self.repo_id.clone(),
                primitive_id: id.clone(),
                primitive_kind: NavigationPrimitiveKind::Package.as_str().to_string(),
                identity_key: format!("package-manifest:{}", file.path),
                label: package_label(&file.path),
                path: Some(file.path.clone()),
                artefact_id: None,
                symbol_id: None,
                source_kind: "MANIFEST_FILE".to_string(),
                confidence: 0.85,
                primitive_hash: stable_hash(&[
                    NavigationPrimitiveKind::Package.as_str(),
                    &file.path,
                    &file.effective_content_id,
                    &file.resolved_language,
                ]),
                properties: json!({
                    "manifest_path": &file.path,
                    "manifest_kind": manifest_kind(&file.path),
                    "effective_content_id": &file.effective_content_id,
                }),
                provenance: self.provenance("manifest_detection"),
                last_observed_generation: Some(self.generation),
            });
            if let Some(file_id) = self.file_primitives_by_path.get(&file.path).cloned() {
                self.upsert_edge("DECLARES", id, file_id, "MANIFEST_FILE", 0.80, json!({}));
            }
        }
    }

    fn add_symbols(&mut self, artefacts: &[CurrentCanonicalArtefactRecord]) {
        for artefact in artefacts {
            let id = primitive_id(
                &self.repo_id,
                NavigationPrimitiveKind::Symbol,
                &artefact.symbol_id,
            );
            self.symbol_primitives_by_symbol_id
                .insert(artefact.symbol_id.clone(), id.clone());
            self.symbol_primitives_by_artefact_id
                .insert(artefact.artefact_id.clone(), id.clone());
            self.upsert_primitive(NavigationPrimitiveFact {
                repo_id: self.repo_id.clone(),
                primitive_id: id.clone(),
                primitive_kind: NavigationPrimitiveKind::Symbol.as_str().to_string(),
                identity_key: format!("symbol:{}", artefact.symbol_id),
                label: symbol_label(artefact),
                path: Some(artefact.path.clone()),
                artefact_id: Some(artefact.artefact_id.clone()),
                symbol_id: Some(artefact.symbol_id.clone()),
                source_kind: "DEVQL_CURRENT_STATE".to_string(),
                confidence: 1.0,
                primitive_hash: stable_hash(&[
                    NavigationPrimitiveKind::Symbol.as_str(),
                    &artefact.path,
                    &artefact.content_id,
                    &artefact.symbol_id,
                    &artefact.artefact_id,
                    &artefact.language,
                    &artefact.extraction_fingerprint,
                    opt_str(artefact.canonical_kind.as_deref()),
                    opt_str(artefact.language_kind.as_deref()),
                    opt_str(artefact.symbol_fqn.as_deref()),
                    opt_str(artefact.parent_symbol_id.as_deref()),
                    opt_str(artefact.parent_artefact_id.as_deref()),
                    &artefact.start_line.to_string(),
                    &artefact.end_line.to_string(),
                    &artefact.start_byte.to_string(),
                    &artefact.end_byte.to_string(),
                    opt_str(artefact.signature.as_deref()),
                    &artefact.modifiers,
                    opt_str(artefact.docstring.as_deref()),
                ]),
                properties: json!({
                    "content_id": &artefact.content_id,
                    "language": &artefact.language,
                    "extraction_fingerprint": &artefact.extraction_fingerprint,
                    "canonical_kind": &artefact.canonical_kind,
                    "language_kind": &artefact.language_kind,
                    "symbol_fqn": &artefact.symbol_fqn,
                    "parent_symbol_id": &artefact.parent_symbol_id,
                    "parent_artefact_id": &artefact.parent_artefact_id,
                    "start_line": artefact.start_line,
                    "end_line": artefact.end_line,
                    "start_byte": artefact.start_byte,
                    "end_byte": artefact.end_byte,
                    "signature": &artefact.signature,
                    "modifiers": parse_modifiers(&artefact.modifiers),
                    "docstring": &artefact.docstring,
                }),
                provenance: self.provenance("artefacts_current"),
                last_observed_generation: Some(self.generation),
            });
            if let Some(file_id) = self.file_primitives_by_path.get(&artefact.path).cloned() {
                self.upsert_edge(
                    "CONTAINS",
                    file_id,
                    id,
                    "DEVQL_CURRENT_STATE",
                    1.0,
                    json!({}),
                );
            }
        }
    }

    fn add_dependency_edges(&mut self, dependency_edges: &[CurrentCanonicalEdgeRecord]) {
        for dependency in dependency_edges {
            let kind = if is_call_edge_kind(&dependency.edge_kind) {
                NavigationPrimitiveKind::CallEdge
            } else {
                NavigationPrimitiveKind::DependencyEdge
            };
            let id = primitive_id(&self.repo_id, kind, &dependency.edge_id);
            self.upsert_primitive(NavigationPrimitiveFact {
                repo_id: self.repo_id.clone(),
                primitive_id: id.clone(),
                primitive_kind: kind.as_str().to_string(),
                identity_key: format!("edge:{}", dependency.edge_id),
                label: format!("{} in {}", dependency.edge_kind, dependency.path),
                path: Some(dependency.path.clone()),
                artefact_id: Some(dependency.from_artefact_id.clone()),
                symbol_id: Some(dependency.from_symbol_id.clone()),
                source_kind: "DEVQL_CURRENT_STATE".to_string(),
                confidence: 0.95,
                primitive_hash: stable_hash(&[
                    kind.as_str(),
                    &dependency.edge_id,
                    &dependency.path,
                    &dependency.content_id,
                    &dependency.from_symbol_id,
                    &dependency.from_artefact_id,
                    opt_str(dependency.to_symbol_id.as_deref()),
                    opt_str(dependency.to_artefact_id.as_deref()),
                    opt_str(dependency.to_symbol_ref.as_deref()),
                    &dependency.edge_kind,
                    &dependency.language,
                    opt_i64(dependency.start_line).as_str(),
                    opt_i64(dependency.end_line).as_str(),
                    &dependency.metadata,
                ]),
                properties: json!({
                    "content_id": &dependency.content_id,
                    "from_symbol_id": &dependency.from_symbol_id,
                    "from_artefact_id": &dependency.from_artefact_id,
                    "to_symbol_id": &dependency.to_symbol_id,
                    "to_artefact_id": &dependency.to_artefact_id,
                    "to_symbol_ref": &dependency.to_symbol_ref,
                    "edge_kind": &dependency.edge_kind,
                    "language": &dependency.language,
                    "start_line": dependency.start_line,
                    "end_line": dependency.end_line,
                    "metadata": parse_json_or_string(&dependency.metadata),
                }),
                provenance: self.provenance("artefact_edges_current"),
                last_observed_generation: Some(self.generation),
            });

            let Some(from_id) = self
                .symbol_primitives_by_symbol_id
                .get(&dependency.from_symbol_id)
                .cloned()
                .or_else(|| {
                    self.symbol_primitives_by_artefact_id
                        .get(&dependency.from_artefact_id)
                        .cloned()
                })
            else {
                continue;
            };
            let to_id = dependency
                .to_symbol_id
                .as_ref()
                .and_then(|symbol_id| self.symbol_primitives_by_symbol_id.get(symbol_id).cloned())
                .or_else(|| {
                    dependency.to_artefact_id.as_ref().and_then(|artefact_id| {
                        self.symbol_primitives_by_artefact_id
                            .get(artefact_id)
                            .cloned()
                    })
                });
            if let Some(to_id) = to_id {
                let relation_kind = if kind == NavigationPrimitiveKind::CallEdge {
                    "CALLS"
                } else {
                    "DEPENDS_ON"
                };
                self.upsert_edge(
                    relation_kind,
                    from_id.clone(),
                    to_id,
                    "DEVQL_CURRENT_STATE",
                    0.95,
                    json!({ "source_edge_id": &dependency.edge_id }),
                );
            }
            self.upsert_edge(
                "REALISES",
                id,
                from_id,
                "DEVQL_CURRENT_STATE",
                0.90,
                json!({ "source_edge_id": &dependency.edge_id }),
            );
        }
    }

    fn add_entrypoints(
        &mut self,
        context: &CurrentStateConsumerContext,
        files: &[CurrentCanonicalFileRecord],
        artefacts: &[CurrentCanonicalArtefactRecord],
    ) {
        let artefacts_by_path = group_entry_point_artefacts_by_path(artefacts);
        for file in files {
            let entry_file = LanguageEntryPointFile {
                path: file.path.clone(),
                language: file.resolved_language.clone(),
                content_id: file.effective_content_id.clone(),
            };
            let file_artefacts = artefacts_by_path
                .get(&file.path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for candidate in context
                .language_services
                .entry_point_candidates_for_file(&entry_file, file_artefacts)
            {
                let identity = candidate.artefact_id.as_deref().unwrap_or(&candidate.path);
                let id = primitive_id(
                    &self.repo_id,
                    NavigationPrimitiveKind::Entrypoint,
                    &format!("{}:{identity}", candidate.entry_kind),
                );
                self.upsert_primitive(NavigationPrimitiveFact {
                    repo_id: self.repo_id.clone(),
                    primitive_id: id.clone(),
                    primitive_kind: NavigationPrimitiveKind::Entrypoint.as_str().to_string(),
                    identity_key: format!("entrypoint:{}:{identity}", candidate.entry_kind),
                    label: candidate.name.clone(),
                    path: Some(candidate.path.clone()),
                    artefact_id: candidate.artefact_id.clone(),
                    symbol_id: candidate.symbol_id.clone(),
                    source_kind: "LANGUAGE_ENTRY_POINT_SUPPORT".to_string(),
                    confidence: candidate.confidence,
                    primitive_hash: stable_hash(&[
                        NavigationPrimitiveKind::Entrypoint.as_str(),
                        &candidate.path,
                        opt_str(candidate.artefact_id.as_deref()),
                        opt_str(candidate.symbol_id.as_deref()),
                        &candidate.name,
                        &candidate.entry_kind,
                        &candidate.reason,
                        &candidate.evidence.join("\n"),
                    ]),
                    properties: json!({
                        "entry_kind": &candidate.entry_kind,
                        "reason": &candidate.reason,
                        "evidence": &candidate.evidence,
                        "language": &file.resolved_language,
                    }),
                    provenance: self.provenance("language_entry_point_support"),
                    last_observed_generation: Some(self.generation),
                });
                if let Some(symbol_id) = candidate
                    .symbol_id
                    .as_ref()
                    .and_then(|symbol_id| self.symbol_primitives_by_symbol_id.get(symbol_id))
                    .cloned()
                {
                    self.upsert_edge(
                        "EXPOSES",
                        id.clone(),
                        symbol_id,
                        "LANGUAGE_ENTRY_POINT_SUPPORT",
                        candidate.confidence,
                        json!({ "entry_kind": &candidate.entry_kind }),
                    );
                }
                if let Some(file_id) = self.file_primitives_by_path.get(&candidate.path).cloned() {
                    self.upsert_edge(
                        "DECLARES",
                        file_id,
                        id,
                        "LANGUAGE_ENTRY_POINT_SUPPORT",
                        candidate.confidence,
                        json!({ "entry_kind": &candidate.entry_kind }),
                    );
                }
            }
        }
    }

    fn upsert_primitive(&mut self, primitive: NavigationPrimitiveFact) {
        self.primitives
            .insert(primitive.primitive_id.clone(), primitive);
    }

    fn upsert_edge(
        &mut self,
        edge_kind: &str,
        from_primitive_id: String,
        to_primitive_id: String,
        source_kind: &str,
        confidence: f64,
        properties: Value,
    ) {
        let id = edge_id(
            &self.repo_id,
            edge_kind,
            &from_primitive_id,
            &to_primitive_id,
        );
        self.edges.insert(
            id.clone(),
            NavigationEdgeFact {
                repo_id: self.repo_id.clone(),
                edge_id: id,
                edge_kind: edge_kind.to_string(),
                from_primitive_id: from_primitive_id.clone(),
                to_primitive_id: to_primitive_id.clone(),
                source_kind: source_kind.to_string(),
                confidence,
                edge_hash: stable_hash(&[
                    edge_kind,
                    &from_primitive_id,
                    &to_primitive_id,
                    source_kind,
                    &properties.to_string(),
                ]),
                properties,
                provenance: self.provenance(source_kind),
                last_observed_generation: Some(self.generation),
            },
        );
    }

    fn provenance(&self, source: &str) -> Value {
        json!({
            "capability": NAVIGATION_CONTEXT_CAPABILITY_ID,
            "consumer": NAVIGATION_CONTEXT_CONSUMER_ID,
            "run_id": &self.run_id,
            "source": source,
        })
    }
}

fn group_entry_point_artefacts_by_path(
    artefacts: &[CurrentCanonicalArtefactRecord],
) -> BTreeMap<String, Vec<LanguageEntryPointArtefact>> {
    let mut grouped: BTreeMap<String, Vec<LanguageEntryPointArtefact>> = BTreeMap::new();
    for artefact in artefacts {
        grouped
            .entry(artefact.path.clone())
            .or_default()
            .push(LanguageEntryPointArtefact {
                artefact_id: artefact.artefact_id.clone(),
                symbol_id: artefact.symbol_id.clone(),
                path: artefact.path.clone(),
                name: symbol_label(artefact),
                canonical_kind: artefact.canonical_kind.clone(),
                language_kind: artefact.language_kind.clone(),
                symbol_fqn: artefact.symbol_fqn.clone(),
                signature: artefact.signature.clone(),
                modifiers: parse_modifiers(&artefact.modifiers),
                start_line: artefact.start_line,
                end_line: artefact.end_line,
            });
    }
    grouped
}

fn symbol_label(artefact: &CurrentCanonicalArtefactRecord) -> String {
    artefact
        .symbol_fqn
        .as_deref()
        .and_then(|fqn| fqn.rsplit("::").next())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| artefact.symbol_id.clone())
}

fn parse_modifiers(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn opt_str(value: Option<&str>) -> &str {
    value.unwrap_or("")
}

fn opt_i64(value: Option<i64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn is_call_edge_kind(edge_kind: &str) -> bool {
    edge_kind.eq_ignore_ascii_case("call") || edge_kind.eq_ignore_ascii_case("calls")
}

fn is_package_manifest(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "Cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "requirements.txt"
    )
}

fn manifest_kind(path: &str) -> &'static str {
    match path.rsplit('/').next().unwrap_or(path) {
        "Cargo.toml" => "cargo",
        "package.json" => "npm",
        "pyproject.toml" => "python",
        "go.mod" => "go",
        "pom.xml" => "maven",
        "build.gradle" | "build.gradle.kts" => "gradle",
        "requirements.txt" => "python_requirements",
        _ => "unknown",
    }
}

fn package_label(path: &str) -> String {
    let basename = path.rsplit('/').next().unwrap_or(path);
    if path == basename {
        basename.to_string()
    } else {
        format!("{} ({})", basename, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn file(path: &str, language: &str) -> CurrentCanonicalFileRecord {
        CurrentCanonicalFileRecord {
            repo_id: "repo".to_string(),
            path: path.to_string(),
            analysis_mode: "current".to_string(),
            file_role: "source".to_string(),
            language: language.to_string(),
            resolved_language: language.to_string(),
            effective_content_id: format!("blob-{path}"),
            parser_version: "parser-v1".to_string(),
            extractor_version: "extractor-v1".to_string(),
            exists_in_head: true,
            exists_in_index: false,
            exists_in_worktree: false,
        }
    }

    fn artefact(path: &str, symbol_id: &str, name: &str) -> CurrentCanonicalArtefactRecord {
        CurrentCanonicalArtefactRecord {
            repo_id: "repo".to_string(),
            path: path.to_string(),
            content_id: format!("blob-{path}"),
            symbol_id: symbol_id.to_string(),
            artefact_id: format!("artefact-{symbol_id}"),
            language: "rust".to_string(),
            extraction_fingerprint: "fingerprint".to_string(),
            canonical_kind: Some("function".to_string()),
            language_kind: Some("function_item".to_string()),
            symbol_fqn: Some(format!("crate::{name}")),
            parent_symbol_id: None,
            parent_artefact_id: None,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 20,
            signature: Some(format!("fn {name}()")),
            modifiers: "[]".to_string(),
            docstring: None,
        }
    }

    #[test]
    fn builder_materialises_files_packages_symbols_and_dependency_edges() {
        let files = vec![file("Cargo.toml", "toml"), file("src/main.rs", "rust")];
        let artefacts = vec![
            artefact("src/main.rs", "symbol-main", "main"),
            artefact("src/main.rs", "symbol-helper", "helper"),
        ];
        let edges = vec![CurrentCanonicalEdgeRecord {
            repo_id: "repo".to_string(),
            edge_id: "edge-main-helper".to_string(),
            path: "src/main.rs".to_string(),
            content_id: "blob-src/main.rs".to_string(),
            from_symbol_id: "symbol-main".to_string(),
            from_artefact_id: "artefact-symbol-main".to_string(),
            to_symbol_id: Some("symbol-helper".to_string()),
            to_artefact_id: Some("artefact-symbol-helper".to_string()),
            to_symbol_ref: None,
            edge_kind: "call".to_string(),
            language: "rust".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: "{}".to_string(),
        }];
        let mut builder = NavigationBuilder::new("repo", 1, "run");

        builder.add_files(&files);
        builder.add_packages(&files);
        builder.add_symbols(&artefacts);
        builder.add_dependency_edges(&edges);
        let facts = builder.finish();

        let primitive_kinds = facts
            .primitives
            .iter()
            .map(|primitive| primitive.primitive_kind.as_str())
            .collect::<BTreeSet<_>>();
        assert!(primitive_kinds.contains(NavigationPrimitiveKind::FileBlob.as_str()));
        assert!(primitive_kinds.contains(NavigationPrimitiveKind::Package.as_str()));
        assert!(primitive_kinds.contains(NavigationPrimitiveKind::Symbol.as_str()));
        assert!(primitive_kinds.contains(NavigationPrimitiveKind::CallEdge.as_str()));
        assert!(facts.edges.iter().any(|edge| edge.edge_kind == "CALLS"));
    }

    #[test]
    fn package_manifest_detection_covers_common_manifests() {
        for path in [
            "Cargo.toml",
            "frontend/package.json",
            "pyproject.toml",
            "service/go.mod",
            "pom.xml",
            "build.gradle.kts",
        ] {
            assert!(
                is_package_manifest(path),
                "{path} should be a package manifest"
            );
        }
    }
}
