mod client;
mod documents;
mod progress;
mod subscription;
#[cfg(test)]
mod tests;
mod types;

#[cfg(test)]
pub(crate) use self::client::with_graphql_executor_hook;
#[cfg(test)]
pub(crate) use self::client::with_ingest_daemon_bootstrap_hook;
#[cfg(test)]
pub(crate) use self::client::with_task_daemon_bootstrap_hook as with_ingest_daemon_runtime_hook;
#[cfg(test)]
pub(crate) use self::client::with_schema_sdl_fetch_hook;
pub(crate) use self::client::{
    cancel_task_via_graphql, enqueue_ingest_task_via_graphql, enqueue_sync_task_via_graphql,
    execute_devql_graphql, fetch_global_schema_sdl_via_daemon, fetch_slim_schema_sdl_via_daemon,
    list_tasks_via_graphql, pause_task_queue_via_graphql, resume_task_queue_via_graphql,
    run_init_via_graphql, task_queue_status_via_graphql, watch_task_id_via_graphql,
    watch_task_via_graphql,
};
pub(crate) use self::types::{
    TaskGraphqlRecord, TaskQueueControlGraphqlRecord, TaskQueueGraphqlRecord,
};
