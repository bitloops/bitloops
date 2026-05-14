use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use crate::host::capability_host::gateways::RelationalGateway;
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
}

#[derive(Debug, Clone, Default)]
pub struct ArchitectureRoleFactExtractionResult {
    pub facts: Vec<ArchitectureArtefactFact>,
    pub refreshed_paths: Vec<String>,
    pub live_paths: BTreeSet<String>,
}

pub trait ArchitectureRoleCurrentStateSource: Send + Sync {
    fn visit_current_artefacts(
        &self,
        visitor: &mut dyn FnMut(CurrentCanonicalArtefactRecord) -> Result<()>,
    ) -> Result<()>;

    fn visit_current_dependency_edges(
        &self,
        visitor: &mut dyn FnMut(CurrentCanonicalEdgeRecord) -> Result<()>,
    ) -> Result<()>;
}

pub struct RelationalArchitectureRoleCurrentStateSource<'a> {
    repo_id: &'a str,
    relational: &'a dyn RelationalGateway,
}

impl<'a> RelationalArchitectureRoleCurrentStateSource<'a> {
    pub fn new(repo_id: &'a str, relational: &'a dyn RelationalGateway) -> Self {
        Self {
            repo_id,
            relational,
        }
    }
}

impl ArchitectureRoleCurrentStateSource for RelationalArchitectureRoleCurrentStateSource<'_> {
    fn visit_current_artefacts(
        &self,
        visitor: &mut dyn FnMut(CurrentCanonicalArtefactRecord) -> Result<()>,
    ) -> Result<()> {
        self.relational
            .visit_current_canonical_artefacts(self.repo_id, visitor)
    }

    fn visit_current_dependency_edges(
        &self,
        visitor: &mut dyn FnMut(CurrentCanonicalEdgeRecord) -> Result<()>,
    ) -> Result<()> {
        self.relational
            .visit_current_canonical_edges(self.repo_id, visitor)
    }
}

pub struct SliceArchitectureRoleCurrentStateSource<'a> {
    artefacts: &'a [CurrentCanonicalArtefactRecord],
    dependency_edges: &'a [CurrentCanonicalEdgeRecord],
}

impl<'a> SliceArchitectureRoleCurrentStateSource<'a> {
    pub fn new(
        artefacts: &'a [CurrentCanonicalArtefactRecord],
        dependency_edges: &'a [CurrentCanonicalEdgeRecord],
    ) -> Self {
        Self {
            artefacts,
            dependency_edges,
        }
    }
}

impl ArchitectureRoleCurrentStateSource for SliceArchitectureRoleCurrentStateSource<'_> {
    fn visit_current_artefacts(
        &self,
        visitor: &mut dyn FnMut(CurrentCanonicalArtefactRecord) -> Result<()>,
    ) -> Result<()> {
        for artefact in self.artefacts.iter().cloned() {
            visitor(artefact)?;
        }
        Ok(())
    }

    fn visit_current_dependency_edges(
        &self,
        visitor: &mut dyn FnMut(CurrentCanonicalEdgeRecord) -> Result<()>,
    ) -> Result<()> {
        for edge in self.dependency_edges.iter().cloned() {
            visitor(edge)?;
        }
        Ok(())
    }
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

#[derive(Debug, Default)]
struct ArtefactPathIndex {
    path_by_artefact_id: BTreeMap<String, String>,
    artefact_paths: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct DependencySummary {
    current_dependency_source_paths: BTreeSet<String>,
    destination_paths_for_affected_sources: BTreeSet<String>,
    outgoing_kinds_by_artefact_id: BTreeMap<String, BTreeSet<String>>,
    incoming_kinds_by_artefact_id: BTreeMap<String, BTreeSet<String>>,
    outgoing_counts_by_artefact_id: BTreeMap<String, usize>,
    incoming_counts_by_artefact_id: BTreeMap<String, usize>,
}

fn build_artefact_path_index(
    source: &dyn ArchitectureRoleCurrentStateSource,
) -> Result<ArtefactPathIndex> {
    let mut index = ArtefactPathIndex::default();
    source.visit_current_artefacts(&mut |artefact| {
        index
            .path_by_artefact_id
            .insert(artefact.artefact_id, artefact.path.clone());
        index.artefact_paths.insert(artefact.path);
        Ok(())
    })?;
    Ok(index)
}

fn summarize_dependency_edges(
    source: &dyn ArchitectureRoleCurrentStateSource,
    affected_paths: &BTreeSet<String>,
    artefact_index: &ArtefactPathIndex,
) -> Result<DependencySummary> {
    let mut summary = DependencySummary::default();
    source.visit_current_dependency_edges(&mut |edge| {
        let source_path = artefact_index
            .path_by_artefact_id
            .get(edge.from_artefact_id.as_str())
            .map(String::as_str);

        summary
            .current_dependency_source_paths
            .insert(edge.path.clone());
        if let Some(source_path) = source_path {
            summary
                .current_dependency_source_paths
                .insert(source_path.to_string());
        }

        let source_affected = affected_paths.contains(&edge.path)
            || source_path.is_some_and(|path| affected_paths.contains(path));
        if source_affected
            && let Some(destination_path) = edge
                .to_artefact_id
                .as_deref()
                .and_then(|artefact_id| artefact_index.path_by_artefact_id.get(artefact_id))
        {
            summary
                .destination_paths_for_affected_sources
                .insert(destination_path.clone());
        }

        summary
            .outgoing_kinds_by_artefact_id
            .entry(edge.from_artefact_id.clone())
            .or_default()
            .insert(edge.edge_kind.clone());
        *summary
            .outgoing_counts_by_artefact_id
            .entry(edge.from_artefact_id.clone())
            .or_default() += 1;

        if let Some(to_artefact_id) = edge.to_artefact_id.as_deref() {
            summary
                .incoming_kinds_by_artefact_id
                .entry(to_artefact_id.to_string())
                .or_default()
                .insert(edge.edge_kind.clone());
            *summary
                .incoming_counts_by_artefact_id
                .entry(to_artefact_id.to_string())
                .or_default() += 1;
        }

        Ok(())
    })?;
    Ok(summary)
}

pub fn extract_architecture_role_facts(
    input: ArchitectureRoleFactExtractionInput<'_>,
    source: &dyn ArchitectureRoleCurrentStateSource,
) -> Result<ArchitectureRoleFactExtractionResult> {
    let artefact_index = build_artefact_path_index(source)?;
    let dependency_summary =
        summarize_dependency_edges(source, input.affected_paths, &artefact_index)?;
    let refreshed_paths = paths_to_refresh(
        input.affected_paths,
        input.files,
        &artefact_index,
        &dependency_summary,
    );
    let refreshed_set = refreshed_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut facts = Vec::new();

    for file in input
        .files
        .iter()
        .filter(|file| refreshed_set.contains(&file.path))
    {
        add_file_facts(input.repo_id, input.generation_seq, file, &mut facts);
    }

    source.visit_current_artefacts(&mut |artefact| {
        if refreshed_set.contains(&artefact.path) {
            add_artefact_facts(input.repo_id, input.generation_seq, &artefact, &mut facts);
            add_dependency_summary_facts_for_artefact(
                input.repo_id,
                input.generation_seq,
                &artefact,
                &dependency_summary,
                &mut facts,
            );
        }
        Ok(())
    })?;

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

    let live_paths = input
        .files
        .iter()
        .map(|file| file.path.clone())
        .chain(artefact_index.artefact_paths)
        .collect::<BTreeSet<_>>();

    Ok(ArchitectureRoleFactExtractionResult {
        facts,
        refreshed_paths,
        live_paths,
    })
}

fn paths_to_refresh(
    affected_paths: &BTreeSet<String>,
    files: &[CurrentCanonicalFileRecord],
    artefact_index: &ArtefactPathIndex,
    dependency_summary: &DependencySummary,
) -> Vec<String> {
    let mut paths = BTreeSet::new();
    if affected_paths.is_empty() {
        paths.extend(files.iter().map(|file| file.path.clone()));
        paths.extend(artefact_index.artefact_paths.iter().cloned());
    } else {
        paths.extend(affected_paths.iter().cloned());
        paths.extend(
            dependency_summary
                .destination_paths_for_affected_sources
                .iter()
                .cloned(),
        );

        let file_paths = files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        let needs_dependency_reset = affected_paths.iter().any(|path| {
            let path = path.as_str();
            artefact_index.artefact_paths.contains(path)
                || (!file_paths.contains(path)
                    && !dependency_summary
                        .current_dependency_source_paths
                        .contains(path))
        });
        if needs_dependency_reset {
            paths.extend(artefact_index.artefact_paths.iter().cloned());
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

fn add_dependency_summary_facts_for_artefact(
    repo_id: &str,
    generation_seq: u64,
    artefact: &CurrentCanonicalArtefactRecord,
    dependency_summary: &DependencySummary,
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

    if let Some(kinds) = dependency_summary
        .outgoing_kinds_by_artefact_id
        .get(artefact.artefact_id.as_str())
    {
        for edge_kind in kinds {
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

    if let Some(count) = dependency_summary
        .outgoing_counts_by_artefact_id
        .get(artefact.artefact_id.as_str())
    {
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

    if let Some(kinds) = dependency_summary
        .incoming_kinds_by_artefact_id
        .get(artefact.artefact_id.as_str())
    {
        for edge_kind in kinds {
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

    if let Some(count) = dependency_summary
        .incoming_counts_by_artefact_id
        .get(artefact.artefact_id.as_str())
    {
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
mod tests;
