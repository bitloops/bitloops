// Enrichment execution surface for the daemon.
//
// This file is a slim facade. The actual implementation lives in cohesive
// submodules under `enrichment/execution/`. Only the names that the rest of
// the `enrichment` module (and the sibling `execution_tests.rs` test file)
// depend on are re-exported here, so the public surface stays unchanged.

#[path = "execution/follow_ups.rs"]
mod follow_ups;
#[path = "execution/helpers.rs"]
mod helpers;
#[path = "execution/loaders.rs"]
mod loaders;
#[path = "execution/mailbox_embedding.rs"]
mod mailbox_embedding;
#[path = "execution/mailbox_summary.rs"]
mod mailbox_summary;
#[path = "execution/workplane_job.rs"]
mod workplane_job;
#[path = "execution/workplane_plan.rs"]
mod workplane_plan;

#[cfg(test)]
#[path = "execution/enrichment_job.rs"]
mod enrichment_job;

#[cfg(test)]
#[path = "execution_tests.rs"]
mod execution_tests;

type SemanticFeatureInput =
    crate::capability_packs::semantic_clones::features::SemanticFeatureInput;

pub(crate) use mailbox_embedding::prepare_embedding_mailbox_batch;
pub(crate) use mailbox_summary::prepare_summary_mailbox_batch;
pub(crate) use workplane_job::execute_workplane_job;

#[cfg(test)]
pub(crate) use enrichment_job::execute_job;
#[cfg(test)]
pub(crate) use loaders::load_repo_backfill_inputs;
#[cfg(test)]
pub(crate) use workplane_plan::{
    WORKPLANE_EMBEDDING_REPO_BACKFILL_BATCH_SIZE, WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE,
    build_embedding_refresh_workplane_plan, build_summary_refresh_workplane_plan,
};

// The sibling `execution_tests.rs` reads these names via `use super::*;`. The
// `unused_imports` lint cannot trace those uses.
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use super::{EnrichmentJob, EnrichmentJobKind, FollowUpJob, JobExecutionOutcome};

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use std::collections::BTreeMap;

#[cfg(test)]
use crate::capability_packs::semantic_clones::clear_repo_active_embedding_setup;
#[cfg(test)]
use crate::capability_packs::semantic_clones::features as semantic_features;
#[cfg(test)]
use crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshScope;
#[cfg(test)]
use crate::config::resolve_store_backend_config_for_repo;
#[cfg(test)]
use crate::host::devql::DevqlConfig;
