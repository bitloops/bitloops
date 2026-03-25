use super::*;

#[path = "pre_push_sync/commit_selection.rs"]
mod commit_selection;
#[path = "pre_push_sync/constants.rs"]
mod constants;
#[path = "pre_push_sync/current_state_replication.rs"]
mod current_state_replication;
#[path = "pre_push_sync/history_replication.rs"]
mod history_replication;
#[path = "pre_push_sync/parsing.rs"]
mod parsing;
#[path = "pre_push_sync/pruning.rs"]
mod pruning;
#[path = "pre_push_sync/runtime.rs"]
mod runtime;
#[path = "pre_push_sync/sql_helpers.rs"]
mod sql_helpers;
#[path = "pre_push_sync/sync_state.rs"]
mod sync_state;
#[path = "pre_push_sync/types.rs"]
mod types;

#[cfg(test)]
#[path = "pre_push_sync/tests.rs"]
mod tests;

pub(crate) use self::runtime::run_devql_pre_push_sync;
