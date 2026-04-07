mod file_discovery;
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

use crate::capability_packs::test_harness::mapping::file_discovery::discover_test_files;
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_enumerated_scenarios, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::{
    StructuralMappingOutput, StructuralMappingStats, TestDiscoveryBatch,
};
use crate::host::capability_host::gateways::LanguageServicesGateway;
use crate::host::language_adapter::{LanguageAdapterContext, LanguageTestSupport};
use crate::models::ProductionArtefact;

pub(crate) fn execute(
    repo_id: &str,
    repo_dir: &Path,
    commit_sha: &str,
    production: &[ProductionArtefact],
    languages: &dyn LanguageServicesGateway,
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
        let content_id = crate::host::devql::sync::content_identity::compute_blob_oid(&fs::read(
            &absolute_path,
        )?);
        content_ids.insert(candidate.relative_path.clone(), content_id);
        let provider = find_language_support(&supports, &candidate.language_id)?;
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

    Ok(StructuralMappingOutput {
        test_artefacts,
        test_edges,
        stats,
        enumeration_status,
        enumeration_notes,
        issues: discovery_batch.issues,
    })
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
