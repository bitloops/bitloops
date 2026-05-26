#[path = "worker/activation.rs"]
mod activation;
#[path = "worker/execution.rs"]
mod execution;
#[path = "worker/lifecycle_spool.rs"]
pub(crate) mod lifecycle_spool;
#[path = "worker/reconcile.rs"]
mod reconcile;
#[path = "worker/spool.rs"]
mod spool;

#[cfg(test)]
pub(crate) use lifecycle_spool::process_lifecycle_stop_spool_once_for_tests;

#[cfg(test)]
#[path = "worker/tests.rs"]
mod tests;
