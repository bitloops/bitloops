// Query execution is split into domain-focused fragments; all are `include!`d into `devql` and
// share the parent module's scope (imports, `RelationalStorage`, parser types, etc.).
include!("executor/validation.rs");
include!("executor/events_pipelines.rs");
include!("executor/row_normalise.rs");
include!("executor/relational.rs");
include!("executor/registered_stages.rs");
include!("executor/chat_history.rs");
