mod file_discovery;
pub(crate) mod languages;
pub(crate) mod linker;
pub(crate) mod materialize;
pub(crate) mod model;
mod registry;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::capability_packs::test_harness::mapping::file_discovery::discover_test_files;
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_enumerated_scenarios, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::{
    StructuralMappingOutput, StructuralMappingStats, TestDiscoveryBatch,
};
use crate::capability_packs::test_harness::mapping::registry::StructuralMappingRegistry;
use crate::models::ProductionArtefact;

pub(crate) fn execute(
    repo_id: &str,
    repo_dir: &Path,
    commit_sha: &str,
    production: &[ProductionArtefact],
) -> Result<StructuralMappingOutput> {
    let production_index = build_production_index(production);
    let mut registry = StructuralMappingRegistry::new()?;
    let enumeration_results: HashMap<&'static str, _> =
        registry.enumerate_all(repo_dir).into_iter().collect();

    let candidates = discover_test_files(repo_dir, registry.providers())?;
    let mut discovery_batch = TestDiscoveryBatch::default();

    for candidate in candidates {
        let absolute_path = repo_dir.join(&candidate.relative_path);
        let provider = registry.provider_mut(candidate.provider_index);
        discovery_batch
            .files
            .push(provider.discover_tests(&absolute_path, &candidate.relative_path)?);
    }

    let mut stats = StructuralMappingStats::default();
    let mut test_artefacts = Vec::new();
    let mut test_edges = Vec::new();
    let mut link_keys = std::collections::HashSet::new();

    let mut materialization = MaterializationContext {
        repo_id,
        commit_sha,
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

        let provider_index = registry
            .providers()
            .iter()
            .position(|provider| provider.language_id() == language_id)
            .expect("provider should exist for enumeration result");
        let provider = registry.provider_mut(provider_index);
        let reconciled = provider.reconcile(&discovery_batch.files, enumeration);

        materialize_enumerated_scenarios(&mut materialization, &reconciled.enumerated_scenarios);
    }

    Ok(StructuralMappingOutput {
        test_artefacts,
        test_edges,
        stats,
        enumeration_status,
        enumeration_notes,
        issues: discovery_batch.issues,
    })
}
