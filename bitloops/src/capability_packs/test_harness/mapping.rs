mod file_discovery;
pub(crate) mod identity_resolution;
pub(crate) mod linker;
pub(crate) mod materialize;
pub(crate) mod model;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::capability_packs::test_harness::event_handlers::ExistingTestArtefactIdentityRow;
use crate::capability_packs::test_harness::mapping::file_discovery::discover_test_files;
use crate::capability_packs::test_harness::mapping::identity_resolution::{
    DraftTestArtefact, DraftTestArtefactId, DraftTestEdge, ResolvedTestIdentityOutput,
    resolve_test_identities,
};
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_enumerated_scenarios, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveryIssue, StructuralMappingOutput, StructuralMappingStats, TestDiscoveryBatch,
};
use crate::host::capability_host::gateways::LanguageServicesGateway;
use crate::host::language_adapter::{LanguageAdapterContext, LanguageTestSupport};
use crate::models::{ProductionArtefact, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

pub(crate) fn execute(
    repo_id: &str,
    repo_dir: &Path,
    commit_sha: &str,
    production: &[ProductionArtefact],
    languages: &dyn LanguageServicesGateway,
) -> Result<StructuralMappingOutput> {
    execute_with_existing(repo_id, repo_dir, commit_sha, production, languages, &[])
}

pub(crate) fn execute_with_existing(
    repo_id: &str,
    repo_dir: &Path,
    commit_sha: &str,
    production: &[ProductionArtefact],
    languages: &dyn LanguageServicesGateway,
    existing_test_artefacts: &[ExistingTestArtefactIdentityRow],
) -> Result<StructuralMappingOutput> {
    let production_index = build_production_index(production);
    let supports = languages.test_supports();
    let language_context = LanguageAdapterContext::new(
        repo_dir.to_path_buf(),
        repo_id.to_string(),
        Some(commit_sha.to_string()),
    );
    let enumeration_results: HashMap<String, _> = supports
        .iter()
        .map(|support| {
            (
                support.language_id().to_string(),
                support.enumerate_tests(&language_context),
            )
        })
        .collect();

    let candidates = discover_test_files(repo_dir, &supports)?;
    let mut discovery_batch = TestDiscoveryBatch::default();
    let mut content_ids = HashMap::new();

    for candidate in candidates {
        let absolute_path = repo_dir.join(&candidate.relative_path);
        let content_id = match fs::read(&absolute_path) {
            Ok(bytes) => crate::host::devql::sync::content_identity::compute_blob_oid(&bytes),
            Err(err) => {
                record_discovery_issue(
                    &mut discovery_batch,
                    &candidate.relative_path,
                    format!("failed reading test file: {err}"),
                );
                continue;
            }
        };
        let provider = find_language_support(&supports, &candidate.language_id)?;
        match provider.discover_tests(&absolute_path, &candidate.relative_path) {
            Ok(discovered) => {
                content_ids.insert(candidate.relative_path.clone(), content_id);
                discovery_batch.files.push(discovered);
            }
            Err(err) => {
                record_discovery_issue(
                    &mut discovery_batch,
                    &candidate.relative_path,
                    format!("{err:#}"),
                );
            }
        }
    }

    let mut stats = StructuralMappingStats::default();
    let mut test_artefacts = Vec::new();
    let mut test_edges = Vec::new();
    let mut link_keys = std::collections::HashSet::new();

    let mut materialization = MaterializationContext {
        repo_id,
        content_ids: &content_ids,
        production,
        production_index: &production_index,
        test_artefacts: &mut test_artefacts,
        test_edges: &mut test_edges,
        link_keys: &mut link_keys,
        stats: &mut stats,
    };
    materialize_source_discovery(&mut materialization, &discovery_batch.files);

    let mut enumeration_status = "source-only".to_string();
    let mut enumeration_notes = Vec::new();

    for (language_id, enumeration) in enumeration_results {
        if language_id == "rust"
            || enumeration.status_label() != "source-only"
            || !enumeration_notes.is_empty()
        {
            enumeration_status = enumeration.status_label().to_string();
        }
        enumeration_notes.extend(enumeration.notes.clone());

        let provider = find_language_support(&supports, &language_id)?;
        let reconciled = provider.reconcile(&discovery_batch.files, enumeration);

        materialize_enumerated_scenarios(&mut materialization, &reconciled.enumerated_scenarios);
    }

    let resolved = resolve_materialized_output(
        repo_id,
        existing_test_artefacts,
        production,
        test_artefacts,
        test_edges,
    );
    stats.test_artefacts = resolved.test_artefacts.len();
    stats.test_edges = resolved.test_edges.len();

    Ok(StructuralMappingOutput {
        test_artefacts: resolved.test_artefacts,
        test_edges: resolved.test_edges,
        stats,
        enumeration_status,
        enumeration_notes,
        issues: discovery_batch.issues,
    })
}

pub(crate) fn resolve_materialized_output(
    repo_id: &str,
    existing_test_artefacts: &[ExistingTestArtefactIdentityRow],
    production: &[ProductionArtefact],
    test_artefacts: Vec<TestArtefactCurrentRecord>,
    test_edges: Vec<TestArtefactEdgeCurrentRecord>,
) -> ResolvedTestIdentityOutput {
    resolve_test_identities(
        repo_id,
        existing_test_artefacts,
        materialized_drafts(&test_artefacts),
        materialized_draft_edges(&test_artefacts, &test_edges, production),
    )
}

fn materialized_drafts(test_artefacts: &[TestArtefactCurrentRecord]) -> Vec<DraftTestArtefact> {
    let parent_by_symbol_id = test_artefacts
        .iter()
        .enumerate()
        .filter(|(_, artefact)| artefact.canonical_kind == "test_suite")
        .map(|(index, artefact)| (artefact.symbol_id.as_str(), DraftTestArtefactId(index)))
        .collect::<HashMap<_, _>>();

    test_artefacts
        .iter()
        .enumerate()
        .map(|(index, artefact)| DraftTestArtefact {
            draft_id: DraftTestArtefactId(index),
            parent_draft_id: artefact
                .parent_symbol_id
                .as_deref()
                .and_then(|parent_symbol_id| parent_by_symbol_id.get(parent_symbol_id))
                .copied(),
            record: artefact.clone(),
        })
        .collect()
}

fn materialized_draft_edges(
    test_artefacts: &[TestArtefactCurrentRecord],
    test_edges: &[TestArtefactEdgeCurrentRecord],
    production: &[ProductionArtefact],
) -> Vec<DraftTestEdge> {
    let draft_id_by_edge_key = test_artefacts
        .iter()
        .enumerate()
        .map(|(index, artefact)| {
            (
                (
                    artefact.path.as_str(),
                    artefact.symbol_id.as_str(),
                    artefact.start_line,
                    artefact.end_line,
                ),
                DraftTestArtefactId(index),
            )
        })
        .collect::<HashMap<_, _>>();
    let production_by_symbol_id = production
        .iter()
        .map(|artefact| (artefact.symbol_id.as_str(), artefact))
        .collect::<HashMap<_, _>>();

    test_edges
        .iter()
        .filter_map(|edge| {
            let start_line = edge.start_line?;
            let end_line = edge.end_line?;
            let from_symbol_id = edge.from_symbol_id.as_str();
            let from_draft_id = draft_id_by_edge_key.get(&(
                edge.path.as_str(),
                from_symbol_id,
                start_line,
                end_line,
            ))?;
            let to_symbol_id = edge.to_symbol_id.as_deref()?;
            let production = production_by_symbol_id.get(to_symbol_id)?;

            Some(DraftTestEdge {
                from_draft_id: *from_draft_id,
                production: (*production).clone(),
                path: edge.path.clone(),
                content_id: edge.content_id.clone(),
                language: edge.language.clone(),
            })
        })
        .collect()
}

fn find_language_support<'a>(
    supports: &'a [Arc<dyn LanguageTestSupport>],
    language_id: &str,
) -> Result<&'a dyn LanguageTestSupport> {
    supports
        .iter()
        .find(|support| support.language_id() == language_id)
        .map(Arc::as_ref)
        .ok_or_else(|| anyhow!("language test support `{language_id}` is not registered"))
}

fn record_discovery_issue(batch: &mut TestDiscoveryBatch, path: &str, message: String) {
    log::warn!("test_harness mapping: failed discovering tests for {path}: {message}");
    batch.issues.push(DiscoveryIssue {
        path: path.to_string(),
        message,
    });
}
