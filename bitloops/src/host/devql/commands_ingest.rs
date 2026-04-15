#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use crate::capability_packs::semantic_clones::embeddings;
#[allow(unused_imports)]
use crate::capability_packs::semantic_clones::features as semantic;
#[allow(unused_imports)]
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
#[allow(unused_imports)]
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, SummaryProviderMode, embeddings_enabled, resolve_embedding_provider,
    resolve_semantic_clones_config, resolve_summary_provider,
};
#[allow(unused_imports)]
use crate::capability_packs::semantic_clones::workplane::resolve_effective_mailbox_intent;
#[allow(unused_imports)]
use crate::capability_packs::semantic_clones::{
    RepoEmbeddingSyncAction, clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
    determine_repo_embedding_sync_action, load_active_embedding_setup,
    load_current_repo_embedding_states, load_semantic_feature_inputs_for_current_repo,
    persist_active_embedding_setup,
};

#[path = "commands_ingest/orchestrator.rs"]
mod orchestrator;
#[path = "commands_ingest/progress.rs"]
mod progress;
#[allow(dead_code)]
#[path = "commands_ingest/semantic_refresh.rs"]
mod semantic_refresh;
#[path = "commands_ingest/shared.rs"]
mod shared;

pub use self::orchestrator::run_ingest;
pub(crate) use self::orchestrator::{
    execute_ingest_with_backfill_window, execute_ingest_with_observer,
};
