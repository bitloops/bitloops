// Query execution is split into domain-focused fragments; all are `include!`d into `devql` and
// share the parent module's scope (imports, `RelationalStorage`, parser types, etc.).
use super::*;

#[path = "executor/chat_history.rs"]
mod chat_history;
#[path = "executor/events_pipelines.rs"]
mod events_pipelines;
#[path = "executor/registered_stages.rs"]
mod registered_stages;
#[path = "executor/relational.rs"]
mod relational;
#[path = "executor/row_normalise.rs"]
mod row_normalise;
#[path = "executor/validation.rs"]
mod validation;

pub(super) use self::chat_history::*;
pub(super) use self::events_pipelines::*;
pub(super) use self::registered_stages::*;
pub(super) use self::relational::*;
pub(super) use self::row_normalise::*;
pub(super) use self::validation::*;
