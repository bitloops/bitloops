use std::time::Duration;

pub(in crate::daemon) const RUNTIME_STATE_FILE_NAME: &str = "runtime.json";
pub(in crate::daemon) const SERVICE_STATE_FILE_NAME: &str = "service.json";
pub(crate) const ENRICHMENT_STATE_FILE_NAME: &str = "enrichment.json";
pub(crate) const SYNC_STATE_FILE_NAME: &str = "sync.json";
pub(in crate::daemon) const INTERNAL_DAEMON_COMMAND_NAME: &str = "__daemon-process";
pub(in crate::daemon) const INTERNAL_SUPERVISOR_COMMAND_NAME: &str = "__daemon-supervisor";
pub(in crate::daemon) const GLOBAL_SUPERVISOR_SERVICE_NAME: &str = "com.bitloops.daemon";
pub(crate) const SUPERVISOR_RUNTIME_STATE_FILE_NAME: &str = "supervisor-runtime.json";
pub(in crate::daemon) const SUPERVISOR_SERVICE_STATE_FILE_NAME: &str = "supervisor-service.json";
pub(in crate::daemon) const READY_TIMEOUT: Duration = Duration::from_secs(20);
pub(in crate::daemon) const STOP_TIMEOUT: Duration = Duration::from_secs(20);
