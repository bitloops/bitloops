mod file_discovery;
pub(crate) mod languages;
mod linker;
mod materialize;
mod model;
mod registry;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::app::test_mapping::file_discovery::discover_test_files;
use crate::app::test_mapping::linker::build_production_index;
use crate::app::test_mapping::materialize::{
    materialize_enumerated_scenarios, materialize_source_discovery,
};
use crate::app::test_mapping::model::{
    StructuralMappingOutput, StructuralMappingStats, TestDiscoveryBatch,
};
use crate::app::test_mapping::registry::StructuralMappingRegistry;
use crate::domain::ProductionArtefact;

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
        discovery_batch.files.push(
            provider.discover_tests(&absolute_path, &candidate.relative_path)?,
        );
    }

    let mut stats = StructuralMappingStats::default();
    let mut artefacts = Vec::new();
    let mut links = Vec::new();
    let mut link_keys = std::collections::HashSet::new();

    materialize_source_discovery(
        repo_id,
        commit_sha,
        production,
        &production_index,
        &discovery_batch.files,
        &mut artefacts,
        &mut links,
        &mut link_keys,
        &mut stats,
    );

    let mut enumeration_status = "source-only".to_string();
    let mut enumeration_notes = Vec::new();

    for (language_id, enumeration) in enumeration_results {
        if enumeration.status_label() != "source-only" || !enumeration_notes.is_empty() {
            enumeration_status = enumeration.status_label().to_string();
        } else if language_id == "rust" {
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

        materialize_enumerated_scenarios(
            repo_id,
            commit_sha,
            production,
            &production_index,
            &reconciled.enumerated_scenarios,
            &mut artefacts,
            &mut links,
            &mut link_keys,
            &mut stats,
        );
    }

    Ok(StructuralMappingOutput {
        artefacts,
        links,
        stats,
        enumeration_status,
        enumeration_notes,
        issues: discovery_batch.issues,
    })
}
