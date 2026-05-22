pub(crate) use crate::host::runtime_store::{
    LifecycleStopHookEnqueueResult, LifecycleStopJobInsert, LifecycleStopJobRecord,
    claim_next_lifecycle_stop_jobs, delete_lifecycle_stop_job,
    enqueue_lifecycle_stop_job_hook_safe_at, recover_running_lifecycle_stop_jobs,
    requeue_lifecycle_stop_job, unix_timestamp_now,
};

#[cfg(test)]
pub(crate) use crate::host::runtime_store::{
    enqueue_lifecycle_stop_job_sqlite, list_lifecycle_stop_jobs_for_tests,
};
