use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use super::config::CodeCityConfig;
use crate::capability_packs::codecity::types::CodeCityDiagnostic;
use crate::host::capability_host::gateways::RelationalGateway;
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CodeCitySourceGraph {
    pub project_path: Option<String>,
    pub files: Vec<CodeCitySourceFile>,
    pub artefacts: Vec<CodeCitySourceArtefact>,
    pub edges: Vec<CodeCitySourceEdge>,
    pub external_dependency_hints: Vec<CodeCityExternalDependencyHint>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeCitySourceFile {
    pub path: String,
    pub language: String,
    pub effective_content_id: String,
    pub included: bool,
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeCitySourceArtefact {
    pub artefact_id: String,
    pub symbol_id: String,
    pub path: String,
    pub symbol_fqn: Option<String>,
    pub canonical_kind: Option<String>,
    pub language_kind: Option<String>,
    pub parent_artefact_id: Option<String>,
    pub parent_symbol_id: Option<String>,
    pub signature: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeCitySourceEdge {
    pub edge_id: String,
    pub from_path: String,
    pub to_path: String,
    pub from_symbol_id: String,
    pub from_artefact_id: String,
    pub to_symbol_id: Option<String>,
    pub to_artefact_id: Option<String>,
    pub to_symbol_ref: Option<String>,
    pub edge_kind: String,
    pub language: String,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeCityExternalDependencyHint {
    pub from_path: String,
    pub to_symbol_ref: Option<String>,
    pub edge_kind: String,
    pub metadata: String,
    pub reason: String,
}

pub fn load_current_source_graph(
    gateway: &dyn RelationalGateway,
    repo_id: &str,
    project_path: Option<&str>,
    config: &CodeCityConfig,
) -> Result<CodeCitySourceGraph> {
    let files = gateway.load_current_canonical_files(repo_id)?;
    let artefacts = gateway.load_current_canonical_artefacts(repo_id)?;
    let edges = gateway.load_current_canonical_edges(repo_id)?;
    Ok(build_source_graph_from_records(
        files,
        artefacts,
        edges,
        project_path,
        config,
    ))
}

pub(crate) fn build_source_graph_from_records(
    files_raw: Vec<CurrentCanonicalFileRecord>,
    artefacts_raw: Vec<CurrentCanonicalArtefactRecord>,
    edges_raw: Vec<CurrentCanonicalEdgeRecord>,
    project_path: Option<&str>,
    config: &CodeCityConfig,
) -> CodeCitySourceGraph {
    let mut files = Vec::with_capacity(files_raw.len());
    let mut included_paths = BTreeSet::new();

    for file in &files_raw {
        let exclusion_reason = file_exclusion_reason(file, project_path, config);
        let included = exclusion_reason.is_none();
        if included {
            included_paths.insert(file.path.clone());
        }
        files.push(CodeCitySourceFile {
            path: file.path.clone(),
            language: effective_language(file),
            effective_content_id: file.effective_content_id.clone(),
            included,
            exclusion_reason,
        });
    }

    let mut symbol_to_path = BTreeMap::new();
    let mut artefact_to_path = BTreeMap::new();
    let mut symbol_ref_to_path = BTreeMap::new();
    let mut artefacts = Vec::new();

    for artefact in artefacts_raw {
        symbol_to_path.insert(artefact.symbol_id.clone(), artefact.path.clone());
        artefact_to_path.insert(artefact.artefact_id.clone(), artefact.path.clone());
        if let Some(symbol_fqn) = artefact.symbol_fqn.as_ref() {
            symbol_ref_to_path.insert(symbol_fqn.clone(), artefact.path.clone());
        }

        if included_paths.contains(&artefact.path) {
            artefacts.push(CodeCitySourceArtefact {
                artefact_id: artefact.artefact_id,
                symbol_id: artefact.symbol_id,
                path: artefact.path,
                symbol_fqn: artefact.symbol_fqn,
                canonical_kind: artefact.canonical_kind,
                language_kind: artefact.language_kind,
                parent_artefact_id: artefact.parent_artefact_id,
                parent_symbol_id: artefact.parent_symbol_id,
                signature: artefact.signature,
                start_line: artefact.start_line,
                end_line: artefact.end_line,
            });
        }
    }

    let mut edges = Vec::new();
    let mut external_dependency_hints = Vec::new();
    let mut unresolved_targets = 0usize;
    let mut cross_scope_edges = 0usize;
    let mut self_edges = 0usize;

    for edge in edges_raw {
        if !included_paths.contains(&edge.path)
            || !is_codecity_dependency_edge_kind(&edge.edge_kind)
        {
            continue;
        }

        let target_path = edge
            .to_symbol_id
            .as_ref()
            .and_then(|symbol_id| symbol_to_path.get(symbol_id).cloned())
            .or_else(|| {
                edge.to_artefact_id
                    .as_ref()
                    .and_then(|artefact_id| artefact_to_path.get(artefact_id).cloned())
            })
            .or_else(|| {
                edge.to_symbol_ref
                    .as_ref()
                    .and_then(|symbol_ref| symbol_ref_to_path.get(symbol_ref).cloned())
            });

        let Some(target_path) = target_path else {
            external_dependency_hints.push(CodeCityExternalDependencyHint {
                from_path: edge.path.clone(),
                to_symbol_ref: edge.to_symbol_ref.clone(),
                edge_kind: normalise_edge_kind(&edge.edge_kind),
                metadata: edge.metadata.clone(),
                reason: "unresolved".to_string(),
            });
            unresolved_targets += 1;
            continue;
        };

        if edge.path == target_path {
            self_edges += 1;
            continue;
        }

        if !included_paths.contains(&target_path) {
            external_dependency_hints.push(CodeCityExternalDependencyHint {
                from_path: edge.path.clone(),
                to_symbol_ref: edge.to_symbol_ref.clone(),
                edge_kind: normalise_edge_kind(&edge.edge_kind),
                metadata: edge.metadata.clone(),
                reason: "out_of_scope".to_string(),
            });
            cross_scope_edges += 1;
            continue;
        }

        edges.push(CodeCitySourceEdge {
            edge_id: edge.edge_id,
            from_path: edge.path,
            to_path: target_path,
            from_symbol_id: edge.from_symbol_id,
            from_artefact_id: edge.from_artefact_id,
            to_symbol_id: edge.to_symbol_id,
            to_artefact_id: edge.to_artefact_id,
            to_symbol_ref: edge.to_symbol_ref,
            edge_kind: normalise_edge_kind(&edge.edge_kind),
            language: edge.language,
            start_line: edge.start_line,
            end_line: edge.end_line,
            metadata: edge.metadata,
        });
    }

    let mut diagnostics = Vec::new();
    if unresolved_targets > 0 {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.source.unresolved_targets".to_string(),
            severity: "info".to_string(),
            message: format!(
                "{unresolved_targets} dependency edge(s) could not be resolved to a target file and were excluded from CodeCity metrics."
            ),
            path: None,
            boundary_id: None,
        });
    }
    if cross_scope_edges > 0 {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.source.cross_scope_edges_ignored".to_string(),
            severity: "info".to_string(),
            message: format!(
                "{cross_scope_edges} dependency edge(s) crossed the active project scope and were excluded from CodeCity metrics."
            ),
            path: None,
            boundary_id: None,
        });
    }
    if self_edges > 0 {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.source.self_edges_ignored".to_string(),
            severity: "info".to_string(),
            message: format!(
                "{self_edges} same-file dependency edge(s) were ignored for CodeCity scoring."
            ),
            path: None,
            boundary_id: None,
        });
    }

    CodeCitySourceGraph {
        project_path: project_path.map(str::to_string),
        files,
        artefacts,
        edges,
        external_dependency_hints,
        diagnostics,
    }
}

pub fn build_dependency_arcs(
    edges: &[CodeCitySourceEdge],
) -> Vec<crate::capability_packs::codecity::types::CodeCityDependencyArc> {
    let mut counts = BTreeMap::new();
    for edge in edges {
        *counts
            .entry((edge.from_path.clone(), edge.to_path.clone()))
            .or_insert(0usize) += 1;
    }

    counts
        .into_iter()
        .map(|((from_path, to_path), edge_count)| {
            crate::capability_packs::codecity::types::CodeCityDependencyArc {
                from_path,
                to_path,
                edge_count,
                arc_kind: "dependency".to_string(),
                severity: None,
            }
        })
        .collect()
}

pub(crate) fn is_codecity_dependency_edge_kind(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "imports" | "calls" | "references" | "extends" | "inherits" | "implements" | "exports"
    )
}

fn normalise_edge_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "inherits" => "extends".to_string(),
        other => other.to_string(),
    }
}

fn effective_language(file: &CurrentCanonicalFileRecord) -> String {
    let resolved = file.resolved_language.trim();
    if resolved.is_empty() {
        file.language.clone()
    } else {
        resolved.to_string()
    }
}

fn file_exclusion_reason(
    file: &CurrentCanonicalFileRecord,
    project_path: Option<&str>,
    config: &CodeCityConfig,
) -> Option<String> {
    if !matches_project_scope(project_path, &file.path) {
        return Some("project_scope".to_string());
    }
    if !file.analysis_mode.eq_ignore_ascii_case("code") {
        return Some("analysis_mode".to_string());
    }
    if !file.file_role.eq_ignore_ascii_case("source_code") {
        return Some("file_role".to_string());
    }
    if !(file.exists_in_head || file.exists_in_index || file.exists_in_worktree) {
        return Some("not_present".to_string());
    }

    config
        .exclusions
        .iter()
        .find_map(|pattern| path_matches_exclusion(&file.path, pattern).then(|| pattern.clone()))
}

fn matches_project_scope(project_path: Option<&str>, path: &str) -> bool {
    let Some(project_path) = project_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return true;
    };
    if project_path == "." {
        return true;
    }

    path == project_path
        || path
            .strip_prefix(project_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn path_matches_exclusion(path: &str, pattern: &str) -> bool {
    match pattern {
        "vendor/**" => path == "vendor" || path.starts_with("vendor/") || path.contains("/vendor/"),
        "node_modules/**" => {
            path == "node_modules"
                || path.starts_with("node_modules/")
                || path.contains("/node_modules/")
        }
        "**/*.generated.*" => path.contains(".generated."),
        "**/*_test.*" => {
            path.contains("_test.")
                || path.ends_with("_test.rs")
                || path.ends_with("_test.go")
                || path.ends_with("_test.py")
                || path.ends_with("_test.ts")
                || path.ends_with("_test.js")
                || path.ends_with("_test.tsx")
                || path.ends_with("_test.jsx")
        }
        "**/*.spec.*" => path.contains(".spec."),
        other => path == other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        build_dependency_arcs, build_source_graph_from_records, is_codecity_dependency_edge_kind,
    };
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::models::{
        CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
    };

    fn file(path: &str) -> CurrentCanonicalFileRecord {
        CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: path.to_string(),
            analysis_mode: "code".to_string(),
            file_role: "source_code".to_string(),
            language: "typescript".to_string(),
            resolved_language: "typescript".to_string(),
            effective_content_id: format!("content::{path}"),
            parser_version: "parser-v1".to_string(),
            extractor_version: "extractor-v1".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }
    }

    fn artefact(
        path: &str,
        symbol_id: &str,
        artefact_id: &str,
        canonical_kind: &str,
    ) -> CurrentCanonicalArtefactRecord {
        CurrentCanonicalArtefactRecord {
            repo_id: "repo-1".to_string(),
            path: path.to_string(),
            content_id: format!("content::{path}"),
            symbol_id: symbol_id.to_string(),
            artefact_id: artefact_id.to_string(),
            language: "typescript".to_string(),
            extraction_fingerprint: "fingerprint".to_string(),
            canonical_kind: Some(canonical_kind.to_string()),
            language_kind: Some("function_declaration".to_string()),
            symbol_fqn: Some(format!("{path}::{symbol_id}")),
            parent_symbol_id: None,
            parent_artefact_id: None,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 30,
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
        }
    }

    fn edge(
        path: &str,
        edge_id: &str,
        from_symbol_id: &str,
        from_artefact_id: &str,
        target: (Option<&str>, Option<&str>, Option<&str>),
        edge_kind: &str,
    ) -> CurrentCanonicalEdgeRecord {
        let (to_symbol_id, to_artefact_id, to_symbol_ref) = target;
        CurrentCanonicalEdgeRecord {
            repo_id: "repo-1".to_string(),
            edge_id: edge_id.to_string(),
            path: path.to_string(),
            content_id: format!("content::{path}"),
            from_symbol_id: from_symbol_id.to_string(),
            from_artefact_id: from_artefact_id.to_string(),
            to_symbol_id: to_symbol_id.map(str::to_string),
            to_artefact_id: to_artefact_id.map(str::to_string),
            to_symbol_ref: to_symbol_ref.map(str::to_string),
            edge_kind: edge_kind.to_string(),
            language: "typescript".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: json!({"resolution": "fixture"}).to_string(),
        }
    }

    #[test]
    fn resolved_edges_map_to_target_paths_and_build_arcs() {
        let config = CodeCityConfig::default();
        let graph = build_source_graph_from_records(
            vec![
                file("packages/api/src/caller.ts"),
                file("packages/api/src/target.ts"),
            ],
            vec![
                artefact(
                    "packages/api/src/caller.ts",
                    "sym::caller",
                    "artefact::caller",
                    "function",
                ),
                artefact(
                    "packages/api/src/target.ts",
                    "sym::target",
                    "artefact::target",
                    "function",
                ),
            ],
            vec![edge(
                "packages/api/src/caller.ts",
                "edge-1",
                "sym::caller",
                "artefact::caller",
                (
                    Some("sym::target"),
                    Some("artefact::target"),
                    Some("packages/api/src/target.ts::sym::target"),
                ),
                "calls",
            )],
            Some("packages/api"),
            &config,
        );

        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from_path, "packages/api/src/caller.ts");
        assert_eq!(graph.edges[0].to_path, "packages/api/src/target.ts");

        let arcs = build_dependency_arcs(&graph.edges);
        assert_eq!(arcs.len(), 1);
        assert_eq!(arcs[0].edge_count, 1);
    }

    #[test]
    fn unresolved_targets_create_diagnostics_and_are_excluded() {
        let config = CodeCityConfig::default();
        let graph = build_source_graph_from_records(
            vec![file("src/caller.ts")],
            vec![artefact(
                "src/caller.ts",
                "sym::caller",
                "artefact::caller",
                "function",
            )],
            vec![edge(
                "src/caller.ts",
                "edge-1",
                "sym::caller",
                "artefact::caller",
                (None, None, Some("src/missing.ts::target")),
                "calls",
            )],
            Some("."),
            &config,
        );

        assert!(graph.edges.is_empty());
        assert!(
            graph
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "codecity.source.unresolved_targets")
        );
    }

    #[test]
    fn exclusions_and_project_scope_are_applied_before_metrics() {
        let config = CodeCityConfig::default();
        let graph = build_source_graph_from_records(
            vec![
                file("packages/api/src/caller.ts"),
                file("packages/web/src/page.ts"),
                file("vendor/lib.ts"),
            ],
            vec![
                artefact(
                    "packages/api/src/caller.ts",
                    "sym::caller",
                    "artefact::caller",
                    "function",
                ),
                artefact(
                    "packages/web/src/page.ts",
                    "sym::page",
                    "artefact::page",
                    "function",
                ),
                artefact(
                    "vendor/lib.ts",
                    "sym::vendor",
                    "artefact::vendor",
                    "function",
                ),
            ],
            vec![
                edge(
                    "packages/api/src/caller.ts",
                    "edge-local",
                    "sym::caller",
                    "artefact::caller",
                    (Some("sym::page"), Some("artefact::page"), None),
                    "calls",
                ),
                edge(
                    "packages/api/src/caller.ts",
                    "edge-self",
                    "sym::caller",
                    "artefact::caller",
                    (Some("sym::caller"), Some("artefact::caller"), None),
                    "calls",
                ),
            ],
            Some("packages/api"),
            &config,
        );

        assert_eq!(graph.files.iter().filter(|file| file.included).count(), 1);
        assert!(graph.edges.is_empty());
        assert!(
            graph
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "codecity.source.cross_scope_edges_ignored")
        );
        assert!(
            graph
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "codecity.source.self_edges_ignored")
        );
    }

    #[test]
    fn dependency_edge_kind_filter_accepts_phase_one_kinds() {
        for kind in [
            "imports",
            "calls",
            "references",
            "extends",
            "inherits",
            "implements",
            "exports",
        ] {
            assert!(
                is_codecity_dependency_edge_kind(kind),
                "{kind} should be accepted"
            );
        }
        assert!(!is_codecity_dependency_edge_kind("test_linkage"));
    }
}
