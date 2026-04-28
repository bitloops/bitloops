use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_enumerated_scenarios, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::StructuralMappingStats;
use crate::host::capability_host::{CurrentStateConsumerContext, CurrentStateConsumerRequest};
use crate::host::language_adapter::{
    DiscoveredTestFile, EnumeratedTestScenario, LanguageAdapterContext,
};

use super::full::reconcile_full;
use super::persistence::{delete_edges_to_removed_symbols, delete_paths, persist_discovered_files};

pub(super) async fn reconcile_delta(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> Result<()> {
    if requires_full_reconcile_for_delta(request) {
        return reconcile_full(request, context).await;
    }

    let mut discovered_files = Vec::new();
    let mut content_ids: HashMap<String, String> = HashMap::new();
    let mut processed_paths: HashSet<String> = HashSet::new();

    let supports = context.language_services.test_supports();
    for file in &request.file_upserts {
        let absolute_path = request.repo_root.join(&file.path);
        let support = context
            .language_services
            .resolve_test_support_for_path(&file.path)
            .or_else(|| {
                supports
                    .iter()
                    .find(|support| support.supports_path(&absolute_path, &file.path))
                    .cloned()
            });
        let Some(support) = support else {
            continue;
        };

        match support.discover_tests(&absolute_path, &file.path) {
            Ok(discovered) => {
                content_ids.insert(file.path.clone(), file.content_id.clone());
                processed_paths.insert(file.path.clone());
                discovered_files.push(discovered);
            }
            Err(err) => {
                log::warn!(
                    "test_harness current-state reconcile: failed discovering tests for {}: {err}",
                    file.path
                );
            }
        }
    }

    if !discovered_files.is_empty() || !processed_paths.is_empty() {
        let enumerated_scenarios = match enumerate_delta_scenarios(
            request,
            context,
            &discovered_files,
            &processed_paths,
        ) {
            DeltaEnumerationDecision::Incremental(scenarios) => scenarios,
            DeltaEnumerationDecision::RequiresFullReconcile => {
                return reconcile_full(request, context).await;
            }
        };
        let production = context
            .relational
            .load_current_production_artefacts(&request.repo_id)?;
        let production_index = build_production_index(&production);
        let mut test_artefacts = Vec::new();
        let mut test_edges = Vec::new();
        let mut link_keys = HashSet::new();
        let mut stats = StructuralMappingStats::default();

        let mut materialization = MaterializationContext {
            repo_id: &request.repo_id,
            content_ids: &content_ids,
            production: &production,
            production_index: &production_index,
            test_artefacts: &mut test_artefacts,
            test_edges: &mut test_edges,
            link_keys: &mut link_keys,
            stats: &mut stats,
        };
        materialize_source_discovery(&mut materialization, &discovered_files);
        materialize_enumerated_scenarios(&mut materialization, &enumerated_scenarios);

        persist_discovered_files(
            &context.storage,
            &request.repo_id,
            &processed_paths,
            &test_artefacts,
            &test_edges,
        )
        .await?;
    }

    if !request.file_removals.is_empty() {
        let removed_paths = request
            .file_removals
            .iter()
            .map(|file| file.path.clone())
            .collect::<HashSet<_>>();
        delete_paths(&context.storage, &request.repo_id, &removed_paths).await?;
    }

    if !request.artefact_removals.is_empty() {
        let removed_symbol_ids = request
            .artefact_removals
            .iter()
            .map(|artefact| artefact.symbol_id.clone())
            .collect::<Vec<_>>();
        delete_edges_to_removed_symbols(&context.storage, &request.repo_id, &removed_symbol_ids)
            .await?;
    }

    Ok(())
}

fn requires_full_reconcile_for_delta(request: &CurrentStateConsumerRequest) -> bool {
    !request.artefact_upserts.is_empty() || !request.artefact_removals.is_empty()
}

enum DeltaEnumerationDecision {
    Incremental(Vec<EnumeratedTestScenario>),
    RequiresFullReconcile,
}

fn enumerate_delta_scenarios(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
    discovered_files: &[DiscoveredTestFile],
    processed_paths: &HashSet<String>,
) -> DeltaEnumerationDecision {
    if discovered_files.is_empty() || processed_paths.is_empty() {
        return DeltaEnumerationDecision::Incremental(Vec::new());
    }

    let language_context = LanguageAdapterContext::new(
        request.repo_root.clone(),
        request.repo_id.clone(),
        request.head_commit_sha.clone(),
    );
    let mut enumerated = Vec::new();

    for support in context.language_services.test_supports() {
        let source_files = discovered_files
            .iter()
            .filter(|file| file.language == support.language_id())
            .cloned()
            .collect::<Vec<_>>();
        if source_files.is_empty() {
            continue;
        }

        let enumeration = support.enumerate_tests(&language_context);
        let reconciled = support.reconcile(&source_files, enumeration);
        if reconciled
            .enumerated_scenarios
            .iter()
            .any(|scenario| scenario.relative_path.starts_with("__synthetic_tests__/"))
        {
            return DeltaEnumerationDecision::RequiresFullReconcile;
        }
        enumerated.extend(
            reconciled
                .enumerated_scenarios
                .into_iter()
                .filter(|scenario| processed_paths.contains(&scenario.relative_path)),
        );
    }

    DeltaEnumerationDecision::Incremental(enumerated)
}
