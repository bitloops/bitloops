pub mod composition;
pub mod config_view;
pub mod contexts;
pub mod descriptor;
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
};
pub use descriptor::{CapabilityDependency, CapabilityDescriptor};
pub use health::{CapabilityHealthCheck, CapabilityHealthResult};
pub use host::DevqlCapabilityHost;
pub use migrations::CapabilityMigration;
pub use policy::{
    CrossPackAccessPolicy, CrossPackGrant, HostInvocationPolicy, PackTrustTier, with_timeout,
};
pub use registrar::{
    BoxFuture, CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration, KnowledgeIngester, KnowledgeIngesterRegistration, KnowledgeStage,
    KnowledgeStageRegistration, QueryExample, SchemaModule, StageHandler, StageRegistration,
    StageRequest, StageResponse,
};
