pub mod composition;
pub mod config_view;
pub mod contexts;
pub mod descriptor;
pub mod diagnostics;
pub mod gateways;
pub mod health;
pub mod host;
pub mod lifecycle;
pub mod migrations;
pub mod policy;
pub mod registrar;
pub mod runtime_contexts;

pub use composition::{
    DEFAULT_DEVQL_SUBQUERY_MAX_DEPTH, DevqlSubqueryOptions, execute_devql_subquery,
};
pub use config_view::CapabilityConfigView;
pub use contexts::{
    CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
    CapabilityMigrationContext, KnowledgeExecutionContext, KnowledgeIngestContext,
    KnowledgeMigrationContext,
};
pub use descriptor::{CapabilityDependency, CapabilityDescriptor};
pub use diagnostics::{
    HostRegistryReport, PackLifecycleReport, collect_health_outcomes,
    format_pack_lifecycle_report_human, format_registry_report_human,
};
pub use health::{CapabilityHealthCheck, CapabilityHealthResult};
pub use host::DevqlCapabilityHost;
pub use migrations::{CapabilityMigration, MigrationRunner};
pub use policy::{
    CrossPackAccessPolicy, CrossPackGrant, HostInvocationPolicy, PackTrustTier, with_timeout,
};
pub use registrar::{
    BoxFuture, CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration, KnowledgeIngesterHandler, KnowledgeIngesterRegistration,
    KnowledgeStageHandler, KnowledgeStageRegistration, QueryExample, SchemaModule, StageHandler,
    StageRegistration, StageRequest, StageResponse,
};
