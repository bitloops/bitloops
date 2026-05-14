use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_CONSUMER_ID,
};
use crate::capability_packs::navigation_context::types::{
    NAVIGATION_CONTEXT_CAPABILITY_ID, NAVIGATION_CONTEXT_CONSUMER_ID,
};
use crate::host::capability_host::{
    CurrentStateConsumerRequest, CurrentStateConsumerResult, DevqlCapabilityHost, ReconcileMode,
};

const CURRENT_STATE_WORKER_COMMAND_NAME: &str = "__current-state-worker";
const WORKER_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const WORKER_SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Args)]
pub struct CurrentStateWorkerArgs {
    #[arg(long)]
    pub config_path: PathBuf,

    #[arg(long)]
    pub capability_id: String,

    #[arg(long)]
    pub consumer_id: String,

    #[arg(long)]
    pub init_session_id: Option<String>,

    #[arg(long)]
    pub parent_pid: Option<u32>,
}

impl CurrentStateWorkerArgs {
    fn validate_supported_target(&self) -> Result<()> {
        if is_supported_current_state_worker_target(&self.capability_id, &self.consumer_id) {
            return Ok(());
        }

        bail!(
            "unsupported current-state worker target capability_id=`{}` consumer_id=`{}`",
            self.capability_id,
            self.consumer_id
        );
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentStateWorkerInvocation {
    pub(crate) config_path: PathBuf,
    pub(crate) capability_id: String,
    pub(crate) consumer_id: String,
    pub(crate) init_session_id: Option<String>,
    pub(crate) parent_pid: Option<u32>,
    pub(crate) request: CurrentStateConsumerRequest,
}

pub(crate) trait CurrentStateWorkerRunner: Send + Sync {
    fn spawn(
        &self,
        invocation: CurrentStateWorkerInvocation,
    ) -> Result<Box<dyn CurrentStateWorkerHandle>>;
}

pub(crate) trait CurrentStateWorkerHandle: Send {
    fn pid(&self) -> u32;
    fn wait<'a>(
        self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Result<CurrentStateConsumerResult>> + Send + 'a>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrentStateExecutionRoute {
    Inline,
    Subprocess { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentStateWorkerTarget {
    ArchitectureGraph,
    NavigationContext,
}

#[derive(Debug, Default)]
pub(crate) struct SubprocessCurrentStateWorkerRunner;

#[derive(Debug)]
struct SubprocessCurrentStateWorkerHandle {
    pid: u32,
    request_bytes: Vec<u8>,
    child: Option<Child>,
    completed: bool,
}

#[derive(Debug, Clone)]
struct CurrentStateWorkerProcessOutput {
    success: bool,
    status_summary: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct ScopedDaemonConfigPathOverride {
    previous: Option<OsString>,
}

impl ScopedDaemonConfigPathOverride {
    fn install(config_path: &std::path::Path) -> Self {
        let previous = std::env::var_os(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE);
        // SAFETY: the worker installs this override in a dedicated short-lived process and
        // restores the previous state before exit.
        unsafe {
            std::env::set_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE, config_path);
        }
        Self { previous }
    }
}

impl Drop for ScopedDaemonConfigPathOverride {
    fn drop(&mut self) {
        // SAFETY: paired with install() above in the same short-lived process.
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE, previous);
            } else {
                std::env::remove_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE);
            }
        }
    }
}

pub(crate) fn is_supported_current_state_worker_target(
    capability_id: &str,
    consumer_id: &str,
) -> bool {
    current_state_worker_target(capability_id, consumer_id).is_some()
}

pub(crate) fn current_state_execution_route(
    capability_id: &str,
    consumer_id: &str,
    reconcile_mode: ReconcileMode,
) -> CurrentStateExecutionRoute {
    if let Some(target) = current_state_worker_target(capability_id, consumer_id) {
        let reason = current_state_worker_route_reason(target, reconcile_mode);
        return CurrentStateExecutionRoute::Subprocess { reason };
    }

    CurrentStateExecutionRoute::Inline
}

fn current_state_worker_target(
    capability_id: &str,
    consumer_id: &str,
) -> Option<CurrentStateWorkerTarget> {
    match (capability_id, consumer_id) {
        (ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_CONSUMER_ID) => {
            Some(CurrentStateWorkerTarget::ArchitectureGraph)
        }
        (NAVIGATION_CONTEXT_CAPABILITY_ID, NAVIGATION_CONTEXT_CONSUMER_ID) => {
            Some(CurrentStateWorkerTarget::NavigationContext)
        }
        _ => None,
    }
}

fn current_state_worker_route_reason(
    target: CurrentStateWorkerTarget,
    reconcile_mode: ReconcileMode,
) -> &'static str {
    match (target, reconcile_mode) {
        (CurrentStateWorkerTarget::ArchitectureGraph, ReconcileMode::FullReconcile) => {
            "architecture_graph_full_reconcile"
        }
        (CurrentStateWorkerTarget::ArchitectureGraph, ReconcileMode::MergedDelta) => {
            "architecture_graph_merged_delta"
        }
        (CurrentStateWorkerTarget::NavigationContext, ReconcileMode::FullReconcile) => {
            "navigation_context_full_reconcile"
        }
        (CurrentStateWorkerTarget::NavigationContext, ReconcileMode::MergedDelta) => {
            "navigation_context_merged_delta"
        }
    }
}

pub(crate) fn should_use_current_state_worker(
    capability_id: &str,
    consumer_id: &str,
    reconcile_mode: ReconcileMode,
) -> bool {
    matches!(
        current_state_execution_route(capability_id, consumer_id, reconcile_mode),
        CurrentStateExecutionRoute::Subprocess { .. }
    )
}

pub async fn run_current_state_worker(args: CurrentStateWorkerArgs) -> Result<()> {
    args.validate_supported_target()?;

    let request = read_current_state_worker_request_from_stdin().await?;
    let result = execute_current_state_worker_request(&args, request).await?;

    ensure_parent_process_alive(args.parent_pid, "emitting current-state worker result")?;

    let payload =
        serde_json::to_vec(&result).context("serializing current-state worker success payload")?;
    tokio::io::stdout()
        .write_all(&payload)
        .await
        .context("writing current-state worker success payload")?;
    tokio::io::stdout()
        .flush()
        .await
        .context("flushing current-state worker success payload")?;

    Ok(())
}

pub(crate) fn terminate_current_state_worker_process(pid: u32) -> Result<()> {
    if !super::process_is_running(pid)? {
        let _ = super::reap_terminated_child_process(pid, Duration::ZERO);
        return Ok(());
    }

    super::terminate_process(pid)
        .with_context(|| format!("sending TERM to current-state worker process {pid}"))?;

    let deadline = Instant::now() + WORKER_SHUTDOWN_GRACE;
    while Instant::now() < deadline {
        if !super::process_is_running(pid)? {
            let _ = super::reap_terminated_child_process(pid, Duration::ZERO);
            return Ok(());
        }
        std::thread::sleep(WORKER_SHUTDOWN_POLL_INTERVAL);
    }

    super::force_kill_process(pid)
        .with_context(|| format!("force-killing current-state worker process {pid}"))?;
    let _ = super::reap_terminated_child_process(pid, WORKER_SHUTDOWN_GRACE);
    Ok(())
}

async fn read_current_state_worker_request_from_stdin() -> Result<CurrentStateConsumerRequest> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .read_to_end(&mut bytes)
        .await
        .context("reading current-state worker request from stdin")?;
    if bytes.is_empty() {
        bail!("current-state worker received an empty stdin payload");
    }

    serde_json::from_slice(&bytes).context("parsing current-state worker request JSON")
}

async fn execute_current_state_worker_request(
    args: &CurrentStateWorkerArgs,
    request: CurrentStateConsumerRequest,
) -> Result<CurrentStateConsumerResult> {
    if !should_use_current_state_worker(
        &args.capability_id,
        &args.consumer_id,
        request.reconcile_mode,
    ) {
        bail!(
            "unsupported current-state worker request capability_id=`{}` consumer_id=`{}` reconcile_mode=`{:?}`",
            args.capability_id,
            args.consumer_id,
            request.reconcile_mode
        );
    }

    ensure_parent_process_alive(args.parent_pid, "starting current-state worker")?;

    let daemon_config = super::resolve_daemon_config(Some(&args.config_path))
        .context("resolving current-state worker daemon config")?;
    let _config_override = ScopedDaemonConfigPathOverride::install(&daemon_config.config_path);

    let repo = crate::host::devql::resolve_repo_identity(&request.repo_root)
        .context("resolving repo identity for current-state worker")?;
    let host = DevqlCapabilityHost::builtin(request.repo_root.clone(), repo)
        .context("building capability host for current-state worker")?;

    let consumer = host
        .current_state_consumers()
        .iter()
        .find(|registration| {
            registration.capability_id == args.capability_id
                && registration.consumer_id == args.consumer_id
        })
        .map(|registration| Arc::clone(&registration.handler))
        .ok_or_else(|| {
            anyhow!(
                "current-state worker consumer `{}` for capability `{}` is not registered",
                args.consumer_id,
                args.capability_id
            )
        })?;

    let context = host
        .build_current_state_consumer_context_with_session_and_parent(
            &args.capability_id,
            args.init_session_id.clone(),
            args.parent_pid,
        )
        .context("building current-state worker consumer context")?;

    let result = consumer
        .reconcile(&request, &context)
        .await
        .context("running current-state worker consumer")?;
    super::capability_events::validate_consumer_result(&request, &result)
        .context("validating current-state worker result")?;

    Ok(result)
}

fn ensure_parent_process_alive(parent_pid: Option<u32>, stage: &str) -> Result<()> {
    let Some(parent_pid) = parent_pid else {
        return Ok(());
    };

    if super::process_is_running(parent_pid)
        .with_context(|| format!("checking parent process {parent_pid} liveness"))?
    {
        return Ok(());
    }

    bail!("current-state worker parent process {parent_pid} is not running while {stage}");
}

fn decode_current_state_worker_process_output(
    output: CurrentStateWorkerProcessOutput,
) -> Result<CurrentStateConsumerResult> {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.success {
        let stderr_suffix = if stderr.is_empty() {
            String::new()
        } else {
            format!(" stderr={stderr}")
        };
        bail!(
            "current-state worker exited unsuccessfully ({}){}",
            output.status_summary,
            stderr_suffix
        );
    }

    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        let stderr_suffix = if stderr.is_empty() {
            String::new()
        } else {
            format!(" stderr={stderr}")
        };
        bail!(
            "current-state worker produced empty stdout{}",
            stderr_suffix
        );
    }

    serde_json::from_slice(&output.stdout).with_context(|| {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stderr.is_empty() {
            format!("parsing current-state worker stdout as JSON: {stdout}")
        } else {
            format!("parsing current-state worker stdout as JSON: {stdout}; stderr={stderr}")
        }
    })
}

fn format_exit_status(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }

    "terminated without an exit code".to_string()
}

impl CurrentStateWorkerRunner for SubprocessCurrentStateWorkerRunner {
    fn spawn(
        &self,
        invocation: CurrentStateWorkerInvocation,
    ) -> Result<Box<dyn CurrentStateWorkerHandle>> {
        let executable =
            std::env::current_exe().context("resolving Bitloops executable for worker spawn")?;
        let request_bytes =
            serde_json::to_vec(&invocation.request).context("serializing worker request JSON")?;

        let mut command = Command::new(executable);
        command
            .arg(CURRENT_STATE_WORKER_COMMAND_NAME)
            .arg("--config-path")
            .arg(&invocation.config_path)
            .arg("--capability-id")
            .arg(&invocation.capability_id)
            .arg("--consumer-id")
            .arg(&invocation.consumer_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&invocation.request.repo_root);

        if let Some(init_session_id) = invocation.init_session_id.as_ref() {
            command.arg("--init-session-id").arg(init_session_id);
        }
        if let Some(parent_pid) = invocation.parent_pid {
            command.arg("--parent-pid").arg(parent_pid.to_string());
        }

        let child = command
            .spawn()
            .context("spawning current-state worker subprocess")?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow!("current-state worker subprocess did not report a pid"))?;

        Ok(Box::new(SubprocessCurrentStateWorkerHandle {
            pid,
            request_bytes,
            child: Some(child),
            completed: false,
        }))
    }
}

impl CurrentStateWorkerHandle for SubprocessCurrentStateWorkerHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn wait<'a>(
        mut self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Result<CurrentStateConsumerResult>> + Send + 'a>> {
        Box::pin(async move {
            let child = self
                .child
                .as_mut()
                .ok_or_else(|| anyhow!("current-state worker handle has no child process"))?;

            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("current-state worker child stdin was unavailable"))?;
            let mut stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow!("current-state worker child stdout was unavailable"))?;
            let mut stderr = child
                .stderr
                .take()
                .ok_or_else(|| anyhow!("current-state worker child stderr was unavailable"))?;
            let request_bytes = std::mem::take(&mut self.request_bytes);

            let stdin_task = tokio::spawn(async move {
                stdin
                    .write_all(&request_bytes)
                    .await
                    .context("writing current-state worker request to stdin")?;
                stdin
                    .shutdown()
                    .await
                    .context("closing current-state worker stdin")?;
                Result::<(), anyhow::Error>::Ok(())
            });
            let stdout_task = tokio::spawn(async move {
                let mut bytes = Vec::new();
                stdout
                    .read_to_end(&mut bytes)
                    .await
                    .context("reading current-state worker stdout")?;
                Result::<Vec<u8>, anyhow::Error>::Ok(bytes)
            });
            let stderr_task = tokio::spawn(async move {
                let mut bytes = Vec::new();
                stderr
                    .read_to_end(&mut bytes)
                    .await
                    .context("reading current-state worker stderr")?;
                Result::<Vec<u8>, anyhow::Error>::Ok(bytes)
            });

            let status = child
                .wait()
                .await
                .context("waiting for current-state worker subprocess")?;
            stdin_task
                .await
                .context("joining current-state worker stdin task")??;
            let stdout = stdout_task
                .await
                .context("joining current-state worker stdout task")??;
            let stderr = stderr_task
                .await
                .context("joining current-state worker stderr task")??;

            self.completed = true;

            decode_current_state_worker_process_output(CurrentStateWorkerProcessOutput {
                success: status.success(),
                status_summary: format_exit_status(status),
                stdout,
                stderr,
            })
        })
    }
}

impl Drop for SubprocessCurrentStateWorkerHandle {
    fn drop(&mut self) {
        if self.completed {
            return;
        }

        let _ = terminate_current_state_worker_process(self.pid);
        self.completed = true;
    }
}

#[cfg(test)]
mod tests {
    use crate::capability_packs::navigation_context::types::{
        NAVIGATION_CONTEXT_CAPABILITY_ID, NAVIGATION_CONTEXT_CONSUMER_ID,
    };

    use super::*;

    #[test]
    fn worker_target_selection_is_narrow() {
        assert!(is_supported_current_state_worker_target(
            ARCHITECTURE_GRAPH_CAPABILITY_ID,
            ARCHITECTURE_GRAPH_CONSUMER_ID
        ));
        assert_eq!(
            current_state_execution_route(
                ARCHITECTURE_GRAPH_CAPABILITY_ID,
                ARCHITECTURE_GRAPH_CONSUMER_ID,
                ReconcileMode::FullReconcile,
            ),
            CurrentStateExecutionRoute::Subprocess {
                reason: "architecture_graph_full_reconcile",
            }
        );
        assert_eq!(
            current_state_execution_route(
                ARCHITECTURE_GRAPH_CAPABILITY_ID,
                ARCHITECTURE_GRAPH_CONSUMER_ID,
                ReconcileMode::MergedDelta,
            ),
            CurrentStateExecutionRoute::Subprocess {
                reason: "architecture_graph_merged_delta",
            }
        );
        assert!(is_supported_current_state_worker_target(
            NAVIGATION_CONTEXT_CAPABILITY_ID,
            NAVIGATION_CONTEXT_CONSUMER_ID
        ));
        assert_eq!(
            current_state_execution_route(
                NAVIGATION_CONTEXT_CAPABILITY_ID,
                NAVIGATION_CONTEXT_CONSUMER_ID,
                ReconcileMode::FullReconcile,
            ),
            CurrentStateExecutionRoute::Subprocess {
                reason: "navigation_context_full_reconcile",
            }
        );
        assert_eq!(
            current_state_execution_route(
                NAVIGATION_CONTEXT_CAPABILITY_ID,
                NAVIGATION_CONTEXT_CONSUMER_ID,
                ReconcileMode::MergedDelta,
            ),
            CurrentStateExecutionRoute::Subprocess {
                reason: "navigation_context_merged_delta",
            }
        );
        assert!(!is_supported_current_state_worker_target(
            "semantic_clones",
            ARCHITECTURE_GRAPH_CONSUMER_ID
        ));
    }

    #[test]
    fn worker_args_reject_unsupported_targets() {
        let err = CurrentStateWorkerArgs {
            config_path: PathBuf::from("/tmp/config.toml"),
            capability_id: "semantic_clones".to_string(),
            consumer_id: "semantic_clones.snapshot".to_string(),
            init_session_id: None,
            parent_pid: None,
        }
        .validate_supported_target()
        .expect_err("unsupported target should fail");

        assert!(
            err.to_string()
                .contains("unsupported current-state worker target"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn decode_current_state_worker_process_output_rejects_non_zero_exit() {
        let err = decode_current_state_worker_process_output(CurrentStateWorkerProcessOutput {
            success: false,
            status_summary: "exit code 7".to_string(),
            stdout: br#"{"applied_to_generation_seq":1}"#.to_vec(),
            stderr: b"worker failed".to_vec(),
        })
        .expect_err("non-zero exit should fail");

        assert!(err.to_string().contains("exit code 7"));
        assert!(err.to_string().contains("worker failed"));
    }

    #[test]
    fn decode_current_state_worker_process_output_rejects_empty_stdout() {
        let err = decode_current_state_worker_process_output(CurrentStateWorkerProcessOutput {
            success: true,
            status_summary: "exit code 0".to_string(),
            stdout: b"   \n".to_vec(),
            stderr: Vec::new(),
        })
        .expect_err("empty stdout should fail");

        assert!(err.to_string().contains("empty stdout"));
    }

    #[test]
    fn decode_current_state_worker_process_output_rejects_malformed_stdout() {
        let err = decode_current_state_worker_process_output(CurrentStateWorkerProcessOutput {
            success: true,
            status_summary: "exit code 0".to_string(),
            stdout: b"not-json".to_vec(),
            stderr: b"stderr note".to_vec(),
        })
        .expect_err("malformed stdout should fail");

        assert!(err.to_string().contains("not-json"));
        assert!(err.to_string().contains("stderr note"));
    }
}
