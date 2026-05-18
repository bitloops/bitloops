use super::*;
#[path = "pre_push_sync/constants.rs"]
mod constants;
#[path = "pre_push_sync/parsing.rs"]
mod parsing;
#[path = "pre_push_sync/pruning.rs"]
mod pruning;
#[path = "pre_push_sync/runtime.rs"]
mod runtime;
#[path = "pre_push_sync/sync_state.rs"]
mod sync_state;
#[path = "pre_push_sync/types.rs"]
mod types;

#[cfg(test)]
#[path = "pre_push_sync/tests.rs"]
mod tests;

pub(crate) use self::runtime::{execute_devql_pre_push_sync, run_devql_pre_push_sync};
