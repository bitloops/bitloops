use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::adapters::model_providers::embeddings::EmbeddingProvider;
use crate::capability_packs::semantic_clones::embeddings::{self, EmbeddingProviderConfig};
use crate::capability_packs::semantic_clones::features::{
    self as semantic, PreStageArtefactRow, PreStageDependencyRow, SemanticFeatureInput,
    SemanticSummaryProvider, SemanticSummaryProviderConfig,
};
use crate::capability_packs::semantic_clones::scoring;
use crate::host::extension_host::{
    CapabilityDescriptor, CapabilityIngesterContribution, CapabilityPackDescriptor,
    CapabilityQueryExampleContribution, CapabilitySchemaModuleContribution,
    CapabilityStageContribution, ExtensionCompatibility,
};

pub const SEMANTIC_CLONES_CAPABILITY_PACK_ID: &str = "semantic-clones-capability-pack";
pub const SEMANTIC_CLONES_CAPABILITY_PACK_ALIAS: &str = "semantic-clones-pack";
pub const SEMANTIC_CLONES_STAGE_ID: &str = "semantic-clones";
pub const SEMANTIC_CLONES_INGESTER_ID: &str = "semantic-clones-ingester";
pub const SEMANTIC_CLONES_SCHEMA_MODULE_ID: &str = "semantic-clones-schema";
pub const SEMANTIC_CLONES_QUERY_EXAMPLE_ID: &str = "semantic-clones-basic";
pub const SEMANTIC_CLONES_QUERY_EXAMPLE: &str = "repo(\"bitloops\")->semanticClones()->limit(10)";

const SEMANTIC_CLONES_CAPABILITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: SEMANTIC_CLONES_CAPABILITY_PACK_ID,
    display_name: "Semantic Clones Capability Pack",
    version: "1.0.0",
    api_version: 1,
    description: "Semantic clone detection and ranking capability",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
};

pub const fn semantic_clones_capability_pack_descriptor(
    required_host_features: &'static [&'static str],
) -> CapabilityPackDescriptor {
    let mut descriptor = SEMANTIC_CLONES_CAPABILITY_DESCRIPTOR;
    descriptor.required_host_features = required_host_features;
    CapabilityPackDescriptor {
        capability: descriptor,
        aliases: &[SEMANTIC_CLONES_CAPABILITY_PACK_ALIAS],
        stage_contributions: &[CapabilityStageContribution {
            id: SEMANTIC_CLONES_STAGE_ID,
        }],
        ingester_contributions: &[CapabilityIngesterContribution {
            id: SEMANTIC_CLONES_INGESTER_ID,
        }],
        schema_module_contributions: &[CapabilitySchemaModuleContribution {
            id: SEMANTIC_CLONES_SCHEMA_MODULE_ID,
        }],
        query_example_contributions: &[CapabilityQueryExampleContribution {
            id: SEMANTIC_CLONES_QUERY_EXAMPLE_ID,
            query: SEMANTIC_CLONES_QUERY_EXAMPLE,
        }],
        compatibility: ExtensionCompatibility::phase1_local_cli(required_host_features),
        migrations: &[],
    }
}

pub fn build_semantic_summary_provider(
    config: &SemanticSummaryProviderConfig,
) -> Result<Arc<dyn SemanticSummaryProvider>> {
    Ok(semantic::build_semantic_summary_provider(config)?.into())
}

pub fn build_symbol_embedding_provider(
    config: &EmbeddingProviderConfig,
    repo_root: Option<&Path>,
) -> Result<Option<Arc<dyn EmbeddingProvider>>> {
    Ok(
        embeddings::build_symbol_embedding_provider(config, repo_root)?
            .map(Arc::<dyn EmbeddingProvider>::from),
    )
}

pub fn build_semantic_feature_inputs(
    artefacts: &[PreStageArtefactRow],
    dependencies: &[PreStageDependencyRow],
    blob_content: &str,
) -> Vec<SemanticFeatureInput> {
    semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
        artefacts,
        dependencies,
        blob_content,
    )
}

pub fn build_symbol_clone_edges(
    inputs: &[scoring::SymbolCloneCandidateInput],
) -> scoring::SymbolCloneBuildResult {
    scoring::build_symbol_clone_edges(inputs)
}

pub fn build_symbol_clone_edges_with_options(
    inputs: &[scoring::SymbolCloneCandidateInput],
    options: scoring::CloneScoringOptions,
) -> scoring::SymbolCloneBuildResult {
    scoring::build_symbol_clone_edges_with_options(inputs, options)
}

pub fn build_symbol_clone_edges_for_source_with_options(
    inputs: &[scoring::SymbolCloneCandidateInput],
    source_symbol_id: &str,
    options: scoring::CloneScoringOptions,
) -> scoring::SymbolCloneBuildResult {
    scoring::build_symbol_clone_edges_for_source_with_options(inputs, source_symbol_id, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_clones_pack_descriptor_registers_expected_surfaces() {
        let descriptor = semantic_clones_capability_pack_descriptor(&[
            "capability-packs",
            "readiness",
            "diagnostics",
            "capability-migrations",
        ]);

        assert_eq!(descriptor.id(), SEMANTIC_CLONES_CAPABILITY_PACK_ID);
        assert_eq!(descriptor.aliases, &[SEMANTIC_CLONES_CAPABILITY_PACK_ALIAS]);
        assert_eq!(
            descriptor.stage_contributions[0].id,
            SEMANTIC_CLONES_STAGE_ID
        );
        assert_eq!(
            descriptor.ingester_contributions[0].id,
            SEMANTIC_CLONES_INGESTER_ID
        );
        assert_eq!(
            descriptor.schema_module_contributions[0].id,
            SEMANTIC_CLONES_SCHEMA_MODULE_ID
        );
        assert_eq!(
            descriptor.query_example_contributions[0].id,
            SEMANTIC_CLONES_QUERY_EXAMPLE_ID
        );
        assert_eq!(
            descriptor.query_example_contributions[0].query,
            SEMANTIC_CLONES_QUERY_EXAMPLE
        );
    }
}
