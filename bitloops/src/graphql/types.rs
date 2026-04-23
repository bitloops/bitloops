pub mod artefact;
pub mod artefact_selection;
mod artefact_selection_schema;
pub mod chat;
pub mod checkpoint;
pub mod clone;
pub mod commit;
pub mod connection;
pub mod dependency_edge;
pub mod expand_hint;
pub mod file_context;
pub mod health;
pub mod ingestion;
pub mod interaction;
pub mod knowledge;
pub mod project;
pub mod repository;
pub mod scalars;
pub mod sync;
pub mod telemetry;
pub mod temporal_scope;
pub mod test_harness;

pub use artefact::LineRangeInput;
pub use artefact::{Artefact, ArtefactCopyLineage, ArtefactFilterInput, CanonicalKind};
pub use artefact_selection::{
    ArtefactSelection, ArtefactSelectorInput, DirectoryEntry, DirectoryEntryKind,
};
pub use chat::{ChatEntry, ChatRole};
pub use checkpoint::{Checkpoint, CheckpointFileRelation};
pub use clone::{CloneSummary, ClonesFilterInput, SemanticClone};
pub use commit::Commit;
pub use connection::{
    ArtefactConnection, ArtefactEdge, ChatEntryConnection, ChatEntryEdge, CheckpointConnection,
    CheckpointEdge, CloneConnection, CloneEdge, CommitConnection, CommitEdge, ConnectionPagination,
    DependencyConnectionEdge, DependencyEdgeConnection, InteractionEventConnection,
    InteractionEventEdge, InteractionSessionConnection, InteractionSessionEdge,
    InteractionTurnConnection, InteractionTurnEdge, KnowledgeItemConnection, KnowledgeItemEdge,
    KnowledgeRelationConnection, KnowledgeRelationEdge, KnowledgeVersionConnection,
    KnowledgeVersionEdge, TelemetryEventConnection, TelemetryEventEdge, paginate_items,
};
pub use dependency_edge::{
    DependencyEdge, DepsDirection, DepsFilterInput, DepsSummary, DepsSummaryFilterInput, EdgeKind,
};
pub use expand_hint::{ExpandHint, ExpandHintParameter, ExpandHintParameters};
pub use file_context::FileContext;
pub use health::{HealthBackendStatus, HealthStatus};
pub use ingestion::IngestionProgressEvent;
pub use interaction::{
    InteractionEventObject, InteractionFilterInput, InteractionSearchInputObject,
    InteractionSessionObject, InteractionSessionSearchHitObject, InteractionTurnObject,
    InteractionTurnSearchHitObject,
};
pub use knowledge::{
    KnowledgeItem, KnowledgeProvider, KnowledgeRelation, KnowledgeSourceKind, KnowledgeTargetType,
    KnowledgeVersion,
};
pub use project::Project;
pub use repository::{Branch, Repository};
pub use scalars::{DateTimeScalar, JsonScalar};
pub use sync::{
    TaskKind, TaskObject, TaskProgressEvent, TaskQueueControlResultObject, TaskQueueStatusObject,
    TaskStatus,
};
pub use telemetry::TelemetryEvent;
pub use temporal_scope::{AsOfInput, TemporalScope};
#[allow(unused_imports)]
pub use test_harness::{
    TestHarnessCommitSummary, TestHarnessCoverageResult, TestHarnessTestsResult,
};
