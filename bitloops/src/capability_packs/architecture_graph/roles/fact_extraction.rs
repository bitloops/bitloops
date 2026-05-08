use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

use super::taxonomy::{ArchitectureArtefactFact, RoleTarget, fact_id};

#[derive(Debug, Clone)]
pub struct ArchitectureRoleFactExtractionInput<'a> {
    pub repo_id: &'a str,
    pub generation_seq: u64,
    pub affected_paths: &'a BTreeSet<String>,
    pub files: &'a [CurrentCanonicalFileRecord],
    pub artefacts: &'a [CurrentCanonicalArtefactRecord],
    pub dependency_edges: &'a [CurrentCanonicalEdgeRecord],
}

#[derive(Debug, Clone, Default)]
pub struct ArchitectureRoleFactExtractionResult {
    pub facts: Vec<ArchitectureArtefactFact>,
    pub refreshed_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct FactContext<'a> {
    repo_id: &'a str,
    generation_seq: u64,
    target: &'a RoleTarget,
    language: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
struct FactSeed<'a> {
    kind: &'a str,
    key: &'a str,
    value: &'a str,
    source: &'a str,
    confidence: f64,
}

pub fn extract_architecture_role_facts(
    input: ArchitectureRoleFactExtractionInput<'_>,
) -> ArchitectureRoleFactExtractionResult {
    let mut facts = Vec::new();
    let refreshed_paths = paths_to_refresh(
        input.affected_paths,
        input.files,
        input.artefacts,
        input.dependency_edges,
    );
    let refreshed_set = refreshed_paths.iter().cloned().collect::<BTreeSet<_>>();

    for file in input
        .files
        .iter()
        .filter(|file| refreshed_set.contains(&file.path))
    {
        add_file_facts(input.repo_id, input.generation_seq, file, &mut facts);
    }

    for artefact in input
        .artefacts
        .iter()
        .filter(|artefact| refreshed_set.contains(&artefact.path))
    {
        add_artefact_facts(input.repo_id, input.generation_seq, artefact, &mut facts);
    }

    add_dependency_summary_facts(
        input.repo_id,
        input.generation_seq,
        &refreshed_set,
        input.artefacts,
        input.dependency_edges,
        &mut facts,
    );
    facts.sort_by(|left, right| {
        (
            &left.target,
            &left.fact_kind,
            &left.fact_key,
            &left.fact_value,
            &left.fact_id,
        )
            .cmp(&(
                &right.target,
                &right.fact_kind,
                &right.fact_key,
                &right.fact_value,
                &right.fact_id,
            ))
    });

    ArchitectureRoleFactExtractionResult {
        facts,
        refreshed_paths,
    }
}

fn paths_to_refresh(
    affected_paths: &BTreeSet<String>,
    files: &[CurrentCanonicalFileRecord],
    artefacts: &[CurrentCanonicalArtefactRecord],
    dependency_edges: &[CurrentCanonicalEdgeRecord],
) -> Vec<String> {
    let mut paths = BTreeSet::new();
    if affected_paths.is_empty() {
        paths.extend(files.iter().map(|file| file.path.clone()));
        paths.extend(artefacts.iter().map(|artefact| artefact.path.clone()));
    } else {
        paths.extend(affected_paths.iter().cloned());

        let artefact_by_id = artefacts
            .iter()
            .map(|artefact| (artefact.artefact_id.as_str(), artefact))
            .collect::<BTreeMap<_, _>>();
        let artefact_paths = artefacts
            .iter()
            .map(|artefact| artefact.path.as_str())
            .collect::<BTreeSet<_>>();
        let file_paths = files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        let mut current_dependency_source_paths = BTreeSet::new();

        for edge in dependency_edges {
            let source_path = artefact_by_id
                .get(edge.from_artefact_id.as_str())
                .map(|artefact| artefact.path.as_str());
            current_dependency_source_paths.insert(edge.path.as_str());
            if let Some(source_path) = source_path {
                current_dependency_source_paths.insert(source_path);
            }
            let source_affected = affected_paths.contains(&edge.path)
                || source_path.is_some_and(|path| affected_paths.contains(path));

            if source_affected
                && let Some(destination) = edge
                    .to_artefact_id
                    .as_deref()
                    .and_then(|artefact_id| artefact_by_id.get(artefact_id))
            {
                paths.insert(destination.path.clone());
            }
        }

        let needs_dependency_reset = affected_paths.iter().any(|path| {
            let path = path.as_str();
            artefact_paths.contains(path)
                || (!file_paths.contains(path) && !current_dependency_source_paths.contains(path))
        });
        if needs_dependency_reset {
            paths.extend(artefacts.iter().map(|artefact| artefact.path.clone()));
        }
    }
    paths.into_iter().collect()
}

fn add_path_facts(
    context: FactContext<'_>,
    path: &str,
    source: &'static str,
    out: &mut Vec<ArchitectureArtefactFact>,
) {
    push_fact(context, fact_seed("path", "full", path, source, 1.0), out);

    if let Some(extension) = path.rsplit('.').next().filter(|part| *part != path) {
        push_fact(
            context,
            fact_seed("path", "extension", extension, source, 1.0),
            out,
        );
    }

    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        push_fact(
            context,
            fact_seed("path", "segment", segment, source, 0.95),
            out,
        );
    }
}

fn add_language_facts(
    context: FactContext<'_>,
    declared_language: &str,
    resolved_language: &str,
    source: &'static str,
    out: &mut Vec<ArchitectureArtefactFact>,
) {
    push_fact(
        context,
        fact_seed("language", "declared", declared_language, source, 1.0),
        out,
    );
    push_fact(
        context,
        fact_seed("language", "resolved", resolved_language, source, 1.0),
        out,
    );
}

fn add_file_facts(
    repo_id: &str,
    generation_seq: u64,
    file: &CurrentCanonicalFileRecord,
    out: &mut Vec<ArchitectureArtefactFact>,
) {
    let target = RoleTarget::file(file.path.clone());
    let language = Some(file.resolved_language.as_str());
    let context = FactContext {
        repo_id,
        generation_seq,
        target: &target,
        language,
    };

    add_path_facts(context, &file.path, "canonical_file", out);
    add_language_facts(
        context,
        &file.language,
        &file.resolved_language,
        "canonical_file",
        out,
    );
    push_fact(
        context,
        fact_seed(
            "file",
            "analysis_mode",
            &file.analysis_mode,
            "canonical_file",
            1.0,
        ),
        out,
    );
    push_fact(
        context,
        fact_seed("file", "role", &file.file_role, "canonical_file", 1.0),
        out,
    );
}

fn add_artefact_facts(
    repo_id: &str,
    generation_seq: u64,
    artefact: &CurrentCanonicalArtefactRecord,
    out: &mut Vec<ArchitectureArtefactFact>,
) {
    let target = RoleTarget::artefact(
        artefact.artefact_id.clone(),
        artefact.symbol_id.clone(),
        artefact.path.clone(),
    );
    let language = Some(artefact.language.as_str());
    let context = FactContext {
        repo_id,
        generation_seq,
        target: &target,
        language,
    };

    add_path_facts(context, &artefact.path, "canonical_artefact", out);
    add_language_facts(
        context,
        &artefact.language,
        &artefact.language,
        "canonical_artefact",
        out,
    );

    if let Some(kind) = artefact.canonical_kind.as_deref() {
        push_fact(
            context,
            fact_seed(
                "artefact",
                "canonical_kind",
                kind,
                "canonical_artefact",
                1.0,
            ),
            out,
        );
    }

    if let Some(kind) = artefact.language_kind.as_deref() {
        push_fact(
            context,
            fact_seed("artefact", "language_kind", kind, "canonical_artefact", 1.0),
            out,
        );
    }

    if let Some(symbol_fqn) = artefact.symbol_fqn.as_deref() {
        push_fact(
            context,
            fact_seed("symbol", "fqn", symbol_fqn, "canonical_artefact", 1.0),
            out,
        );

        if let Some(name) = symbol_name_from_fqn(symbol_fqn) {
            push_fact(
                context,
                fact_seed("symbol", "name", name, "canonical_artefact", 1.0),
                out,
            );

            for suffix in [
                "Command",
                "Handler",
                "Store",
                "Repository",
                "Controller",
                "Consumer",
            ] {
                if name.ends_with(suffix) {
                    push_fact(
                        context,
                        fact_seed("symbol", "name_suffix", suffix, "canonical_artefact", 0.90),
                        out,
                    );
                }
            }
        }
    }

    if let Some(signature) = artefact.signature.as_deref() {
        push_fact(
            context,
            fact_seed("symbol", "has_signature", "true", "canonical_artefact", 1.0),
            out,
        );

        for token in ["async", "Result", "Command", "Context"] {
            if signature.contains(token) {
                push_fact(
                    context,
                    fact_seed("signature", "contains", token, "canonical_artefact", 0.75),
                    out,
                );
            }
        }
    }

    if let Some(parent) = artefact.parent_artefact_id.as_deref() {
        push_fact(
            context,
            fact_seed(
                "artefact",
                "has_parent_artefact",
                parent,
                "canonical_artefact",
                1.0,
            ),
            out,
        );
    }
}

fn symbol_name_from_fqn(symbol_fqn: &str) -> Option<&str> {
    symbol_fqn
        .rsplit("::")
        .next()
        .and_then(|part| part.rsplit('.').next())
        .filter(|part| !part.trim().is_empty())
}

fn add_dependency_summary_facts(
    repo_id: &str,
    generation_seq: u64,
    refreshed_paths: &BTreeSet<String>,
    artefacts: &[CurrentCanonicalArtefactRecord],
    edges: &[CurrentCanonicalEdgeRecord],
    out: &mut Vec<ArchitectureArtefactFact>,
) {
    let artefact_by_id = artefacts
        .iter()
        .map(|artefact| (artefact.artefact_id.as_str(), artefact))
        .collect::<BTreeMap<_, _>>();
    let mut outgoing_kinds: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut incoming_kinds: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut outgoing_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut incoming_counts: BTreeMap<&str, usize> = BTreeMap::new();

    for edge in edges {
        outgoing_kinds.insert((edge.from_artefact_id.as_str(), edge.edge_kind.as_str()));
        *outgoing_counts
            .entry(edge.from_artefact_id.as_str())
            .or_default() += 1;

        if let Some(to_artefact_id) = edge.to_artefact_id.as_deref() {
            incoming_kinds.insert((to_artefact_id, edge.edge_kind.as_str()));
            *incoming_counts.entry(to_artefact_id).or_default() += 1;
        }
    }

    for (artefact_id, edge_kind) in outgoing_kinds {
        if let Some(artefact) = artefact_by_id
            .get(artefact_id)
            .filter(|artefact| refreshed_paths.contains(&artefact.path))
        {
            let target = RoleTarget::artefact(
                artefact.artefact_id.clone(),
                artefact.symbol_id.clone(),
                artefact.path.clone(),
            );
            let language = Some(artefact.language.as_str());
            let context = FactContext {
                repo_id,
                generation_seq,
                target: &target,
                language,
            };

            push_fact(
                context,
                fact_seed(
                    "dependency",
                    "outgoing_kind",
                    edge_kind,
                    "canonical_edge",
                    0.95,
                ),
                out,
            );
        }
    }

    for (artefact_id, count) in outgoing_counts {
        if let Some(artefact) = artefact_by_id
            .get(artefact_id)
            .filter(|artefact| refreshed_paths.contains(&artefact.path))
        {
            let target = RoleTarget::artefact(
                artefact.artefact_id.clone(),
                artefact.symbol_id.clone(),
                artefact.path.clone(),
            );
            let language = Some(artefact.language.as_str());
            let context = FactContext {
                repo_id,
                generation_seq,
                target: &target,
                language,
            };

            push_fact(
                context,
                fact_seed(
                    "dependency",
                    "outgoing_count",
                    &count.to_string(),
                    "canonical_edge",
                    0.90,
                ),
                out,
            );
        }
    }

    for (artefact_id, edge_kind) in incoming_kinds {
        if let Some(artefact) = artefact_by_id
            .get(artefact_id)
            .filter(|artefact| refreshed_paths.contains(&artefact.path))
        {
            let target = RoleTarget::artefact(
                artefact.artefact_id.clone(),
                artefact.symbol_id.clone(),
                artefact.path.clone(),
            );
            let language = Some(artefact.language.as_str());
            let context = FactContext {
                repo_id,
                generation_seq,
                target: &target,
                language,
            };

            push_fact(
                context,
                fact_seed(
                    "dependency",
                    "incoming_kind",
                    edge_kind,
                    "canonical_edge",
                    0.95,
                ),
                out,
            );
        }
    }

    for (artefact_id, count) in incoming_counts {
        if let Some(artefact) = artefact_by_id
            .get(artefact_id)
            .filter(|artefact| refreshed_paths.contains(&artefact.path))
        {
            let target = RoleTarget::artefact(
                artefact.artefact_id.clone(),
                artefact.symbol_id.clone(),
                artefact.path.clone(),
            );
            let language = Some(artefact.language.as_str());
            let context = FactContext {
                repo_id,
                generation_seq,
                target: &target,
                language,
            };

            push_fact(
                context,
                fact_seed(
                    "dependency",
                    "incoming_count",
                    &count.to_string(),
                    "canonical_edge",
                    0.90,
                ),
                out,
            );
        }
    }
}

fn fact_seed<'a>(
    kind: &'a str,
    key: &'a str,
    value: &'a str,
    source: &'a str,
    confidence: f64,
) -> FactSeed<'a> {
    FactSeed {
        kind,
        key,
        value,
        source,
        confidence,
    }
}

fn push_fact(
    context: FactContext<'_>,
    fact: FactSeed<'_>,
    out: &mut Vec<ArchitectureArtefactFact>,
) {
    if fact.value.trim().is_empty() {
        return;
    }

    out.push(ArchitectureArtefactFact {
        repo_id: context.repo_id.to_string(),
        fact_id: fact_id(
            context.repo_id,
            context.target,
            fact.kind,
            fact.key,
            fact.value,
        ),
        target: context.target.clone(),
        language: context.language.map(str::to_string),
        fact_kind: fact.kind.to_string(),
        fact_key: fact.key.to_string(),
        fact_value: fact.value.to_string(),
        source: fact.source.to_string(),
        confidence: fact.confidence,
        evidence: json!([{ "target": context.target, "kind": fact.kind, "key": fact.key, "value": fact.value }]),
        generation_seq: context.generation_seq,
    });
}

#[cfg(test)]
mod tests {
    use crate::models::{
        CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
    };

    use super::*;

    fn file_fixture(path: &str) -> CurrentCanonicalFileRecord {
        CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: path.to_string(),
            analysis_mode: "code".to_string(),
            file_role: "source".to_string(),
            language: "rust".to_string(),
            resolved_language: "rust".to_string(),
            effective_content_id: format!("content:{path}"),
            parser_version: "parser".to_string(),
            extractor_version: "extractor".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }
    }

    fn artefact_fixture(path: &str, name: &str) -> CurrentCanonicalArtefactRecord {
        CurrentCanonicalArtefactRecord {
            repo_id: "repo-1".to_string(),
            path: path.to_string(),
            content_id: format!("content:{path}"),
            symbol_id: format!("symbol-{name}"),
            artefact_id: format!("artefact-{name}"),
            language: "rust".to_string(),
            extraction_fingerprint: format!("fingerprint-{name}"),
            canonical_kind: Some("function".to_string()),
            language_kind: Some("function".to_string()),
            symbol_fqn: Some(format!("crate::{name}")),
            parent_symbol_id: None,
            parent_artefact_id: None,
            start_line: 1,
            end_line: 5,
            start_byte: 0,
            end_byte: 40,
            signature: None,
            modifiers: String::new(),
            docstring: None,
        }
    }

    #[test]
    fn extracts_file_and_path_facts_for_affected_path() {
        let files = vec![file_fixture("bitloops/src/cli/main.rs")];
        let affected_paths = BTreeSet::from(["bitloops/src/cli/main.rs".to_string()]);

        let result = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
            artefacts: &[],
            dependency_edges: &[],
        });

        assert!(result.facts.iter().any(|fact| {
            fact.fact_kind == "path" && fact.fact_key == "segment" && fact.fact_value == "cli"
        }));
        assert!(result.facts.iter().any(|fact| {
            fact.fact_kind == "language" && fact.fact_key == "resolved" && fact.fact_value == "rust"
        }));
        assert_eq!(result.refreshed_paths, vec!["bitloops/src/cli/main.rs"]);
    }

    #[test]
    fn extracts_path_and_language_facts_for_artefact_targets() {
        let artefacts = vec![artefact_fixture(
            "src/application/create_user.rs",
            "create_user_use_case",
        )];
        let affected_paths = BTreeSet::from(["src/application/create_user.rs".to_string()]);

        let result = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &[],
            artefacts: &artefacts,
            dependency_edges: &[],
        });

        let artefact_target = RoleTarget::artefact(
            "artefact-create_user_use_case".to_string(),
            "symbol-create_user_use_case".to_string(),
            "src/application/create_user.rs".to_string(),
        );
        let target_facts = result
            .facts
            .iter()
            .filter(|fact| fact.target == artefact_target)
            .collect::<Vec<_>>();

        assert!(target_facts.iter().any(|fact| {
            fact.fact_kind == "path"
                && fact.fact_key == "full"
                && fact.fact_value == "src/application/create_user.rs"
        }));
        assert!(target_facts.iter().any(|fact| {
            fact.fact_kind == "path"
                && fact.fact_key == "segment"
                && fact.fact_value == "application"
        }));
        assert!(target_facts.iter().any(|fact| {
            fact.fact_kind == "path" && fact.fact_key == "extension" && fact.fact_value == "rs"
        }));
        assert!(target_facts.iter().any(|fact| {
            fact.fact_kind == "language" && fact.fact_key == "declared" && fact.fact_value == "rust"
        }));
        assert!(target_facts.iter().any(|fact| {
            fact.fact_kind == "language" && fact.fact_key == "resolved" && fact.fact_value == "rust"
        }));
        assert!(target_facts.iter().any(|fact| {
            fact.fact_kind == "symbol"
                && fact.fact_key == "fqn"
                && fact.fact_value == "crate::create_user_use_case"
        }));
    }

    #[test]
    fn refreshed_paths_include_dependency_destinations_for_affected_source_path() {
        let artefacts = vec![
            artefact_fixture("src/a.rs", "a"),
            artefact_fixture("src/b.rs", "b"),
        ];
        let dependency_edges = vec![CurrentCanonicalEdgeRecord {
            repo_id: "repo-1".to_string(),
            edge_id: "edge-1".to_string(),
            path: "src/a.rs".to_string(),
            content_id: "content-a".to_string(),
            from_symbol_id: "symbol-a".to_string(),
            from_artefact_id: "artefact-a".to_string(),
            to_symbol_id: Some("symbol-b".to_string()),
            to_artefact_id: Some("artefact-b".to_string()),
            to_symbol_ref: Some("crate::b::destination".to_string()),
            edge_kind: "calls".to_string(),
            language: "rust".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: "{}".to_string(),
        }];
        let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

        let result = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &[],
            artefacts: &artefacts,
            dependency_edges: &dependency_edges,
        });

        assert_eq!(result.refreshed_paths, vec!["src/a.rs", "src/b.rs"]);
        assert!(result.facts.iter().any(|fact| {
            fact.target.path == "src/b.rs"
                && fact.fact_kind == "dependency"
                && fact.fact_key == "incoming_kind"
                && fact.fact_value == "calls"
        }));
    }

    #[test]
    fn affected_artefact_path_refreshes_all_artefact_paths_for_removed_edges() {
        let artefacts = vec![
            artefact_fixture("src/a.rs", "a"),
            artefact_fixture("src/b.rs", "b"),
        ];
        let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

        let result = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &[],
            artefacts: &artefacts,
            dependency_edges: &[],
        });

        assert_eq!(result.refreshed_paths, vec!["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn affected_artefact_path_refreshes_all_artefact_paths_for_partial_edge_removals() {
        let artefacts = vec![
            artefact_fixture("src/a.rs", "a"),
            artefact_fixture("src/b.rs", "b"),
            artefact_fixture("src/c.rs", "c"),
        ];
        let dependency_edges = vec![CurrentCanonicalEdgeRecord {
            repo_id: "repo-1".to_string(),
            edge_id: "edge-1".to_string(),
            path: "src/a.rs".to_string(),
            content_id: "content-a".to_string(),
            from_symbol_id: "symbol-a".to_string(),
            from_artefact_id: "artefact-a".to_string(),
            to_symbol_id: Some("symbol-b".to_string()),
            to_artefact_id: Some("artefact-b".to_string()),
            to_symbol_ref: Some("crate::b".to_string()),
            edge_kind: "calls".to_string(),
            language: "rust".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: "{}".to_string(),
        }];
        let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

        let result = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &[],
            artefacts: &artefacts,
            dependency_edges: &dependency_edges,
        });

        assert_eq!(
            result.refreshed_paths,
            vec!["src/a.rs", "src/b.rs", "src/c.rs"]
        );
    }

    #[test]
    fn extracts_facts_in_stable_order_independent_of_input_order() {
        let first_files = vec![file_fixture("src/a.rs"), file_fixture("src/b.rs")];
        let second_files = vec![file_fixture("src/b.rs"), file_fixture("src/a.rs")];
        let affected_paths = BTreeSet::new();

        let first = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &first_files,
            artefacts: &[],
            dependency_edges: &[],
        });
        let second = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &second_files,
            artefacts: &[],
            dependency_edges: &[],
        });

        assert_eq!(first.facts, second.facts);
    }

    #[test]
    fn dependency_counts_are_totalled_once_per_target() {
        let artefacts = vec![
            artefact_fixture("src/a.rs", "a"),
            artefact_fixture("src/b.rs", "b"),
            artefact_fixture("src/c.rs", "c"),
        ];
        let dependency_edges = vec![
            CurrentCanonicalEdgeRecord {
                repo_id: "repo-1".to_string(),
                edge_id: "edge-1".to_string(),
                path: "src/a.rs".to_string(),
                content_id: "content-a".to_string(),
                from_symbol_id: "symbol-a".to_string(),
                from_artefact_id: "artefact-a".to_string(),
                to_symbol_id: Some("symbol-b".to_string()),
                to_artefact_id: Some("artefact-b".to_string()),
                to_symbol_ref: Some("crate::b".to_string()),
                edge_kind: "calls".to_string(),
                language: "rust".to_string(),
                start_line: Some(2),
                end_line: Some(2),
                metadata: "{}".to_string(),
            },
            CurrentCanonicalEdgeRecord {
                repo_id: "repo-1".to_string(),
                edge_id: "edge-2".to_string(),
                path: "src/a.rs".to_string(),
                content_id: "content-a".to_string(),
                from_symbol_id: "symbol-a".to_string(),
                from_artefact_id: "artefact-a".to_string(),
                to_symbol_id: Some("symbol-c".to_string()),
                to_artefact_id: Some("artefact-c".to_string()),
                to_symbol_ref: Some("crate::c".to_string()),
                edge_kind: "imports".to_string(),
                language: "rust".to_string(),
                start_line: Some(3),
                end_line: Some(3),
                metadata: "{}".to_string(),
            },
        ];
        let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

        let result = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &[],
            artefacts: &artefacts,
            dependency_edges: &dependency_edges,
        });
        let outgoing_counts = result
            .facts
            .iter()
            .filter(|fact| {
                fact.target.path == "src/a.rs"
                    && fact.fact_kind == "dependency"
                    && fact.fact_key == "outgoing_count"
            })
            .collect::<Vec<_>>();

        assert_eq!(outgoing_counts.len(), 1);
        assert_eq!(outgoing_counts[0].fact_value, "2");
        assert!(result.facts.iter().any(|fact| {
            fact.target.path == "src/a.rs"
                && fact.fact_kind == "dependency"
                && fact.fact_key == "outgoing_kind"
                && fact.fact_value == "calls"
        }));
        assert!(result.facts.iter().any(|fact| {
            fact.target.path == "src/a.rs"
                && fact.fact_kind == "dependency"
                && fact.fact_key == "outgoing_kind"
                && fact.fact_value == "imports"
        }));
    }
}
