//! Knowledge capability pack: ingestion, relational + document storage, and retrieval.
//!
//! ## Hygiene (store and config boundaries)
//!
//! - **SQLite / DuckDB / blob payloads** for this pack are implemented only under
//!   [`storage`] and opened by the DevQL host / extension wiring (`LocalCapabilityRuntime`,
//!   pack registration). Production pack logic should use [`crate::host::capability_host::gateways`]
//!   (`RelationalGateway`, `DocumentStoreGateway`, `BlobPayloadGateway`) via
//!   [`CapabilityIngestContext`](crate::host::capability_host::CapabilityIngestContext) /
//!   [`CapabilityExecutionContext`](crate::host::capability_host::CapabilityExecutionContext),
//!   not ad hoc connections to knowledge store files.
//! - **Repo config** for capability behaviour should be read through
//!   [`CapabilityConfigView`](crate::host::capability_host::CapabilityConfigView)
//!   (`config_view("knowledge")` on ingest/health contexts), not by re-parsing raw config in pack code.
//! - **Provenance** JSON on writes is built in [`provenance`] and includes pack identity from
//!   [`descriptor::KNOWLEDGE_DESCRIPTOR`] (`capability_version`, `api_version`) plus the ingest surface
//!   (`knowledge.add` vs `knowledge.refresh`, etc.). When runs go through [`DevqlCapabilityHost::invoke_ingester`](crate::host::capability_host::DevqlCapabilityHost::invoke_ingester),
//!   persisted JSON also includes **`invoking_capability_id`** and **`ingester_id`** from the ingest context.
//! - **External fetches** (GitHub / Jira / Confluence) use **`KnowledgeConnectorAdapter`** implementations
//!   under `engine::adapters::connectors`, selected via [`ConnectorRegistry`](crate::host::capability_host::gateways::ConnectorRegistry) on the ingest context — not a separate pack-local provider module.

pub mod cli;
pub mod descriptor;
pub mod discussion;
pub mod health;
pub mod ingesters;
pub mod migrations;
pub mod pack;
pub mod provenance;
pub mod query_examples;
mod reference_parse;
mod reference_resolve;
mod reference_types;
mod reference_validate;
pub mod references;
pub mod register;
pub mod schema;
pub mod services;
pub mod stages;
pub mod storage;
pub mod types;
pub mod url;

pub use cli::run_knowledge_versions_via_host;
pub use pack::KnowledgePack;
pub use references as refs;
pub use types::*;
