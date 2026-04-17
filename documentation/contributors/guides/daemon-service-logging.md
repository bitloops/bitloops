---
title: Daemon and Service Logging
description: Logging rules for daemon-owned, background, and long-lived backend flows in Bitloops.
---

# Daemon and Service Logging

This guide defines the logging contract for Bitloops daemon-owned and service/backend flows.

It applies to:

- daemon lifecycle code
- supervisor flows
- background workers
- server and API handlers
- other long-lived backend flows that do not behave like a one-shot foreground CLI command

If code runs in one of those environments, it should use the shared main logger.

## Use the main logger

Do not introduce ad hoc log files, `println!`, or `eprintln!` as the primary diagnostic path for daemon or service code.

The expected sink is the shared daemon logger used by:

- `bitloops daemon logs`
- `bitloops daemon logs --level warn`
- `bitloops daemon logs --level error`

If a background flow fails, a user or engineer should be able to inspect that failure through the main logger.

## Which logger to use

There are two different structured logging paths in the codebase.

### Daemon and service/backend flows

For:

- daemon process code
- daemon supervisor code
- watcher code
- background workers
- server and API handlers
- other long-lived backend flows

use the standard Rust `log` facade:

- `log::error!`
- `log::warn!`
- `log::info!`
- `log::debug!`

These records go to the shared daemon logger when the process entrypoint initializes it.
That is the logging path behind `bitloops daemon logs`.

### Capture and hook flows

For agent-hook and git-hook capture flows, use the hook/session logger under `crate::telemetry::logging`.

Examples:

- `logging::error(...)`
- `logging::warn(...)`
- `logging::info(...)`

Those hooks do not use the shared daemon log file as their primary sink.
They have their own structured logging path.

### Practical rule

If the code runs as part of the daemon runtime or another long-lived backend/service process, use `log::*`.

If the code runs in the capture-hook/session-hook pipeline, use `crate::telemetry::logging::*`.

## Main rule

Major daemon-owned and service/backend flows must not fail silently.

That means:

- terminal failures must be logged
- retryable and degraded states should be logged
- important startup, shutdown, and recovery failures should be logged
- fallback paths that skip work or reduce guarantees should be logged

## Severity contract

Use these levels consistently:

- `ERROR`: terminal failure, failed startup/activation, failed handoff, failed background operation that will not complete successfully without intervention
- `WARN`: retryable failure, degraded mode, recovery path, best-effort fallback, cleanup failure, or skipped work that matters but is not terminal
- `INFO`: workflow boundary events such as start, ready, stop, restart, and successful completion when those events are operationally useful
- `DEBUG`: noisy inner-loop detail that is useful during deep debugging but should not dominate normal daemon logs

If a flow cannot start because a required runtime, dependency, or handoff is missing, prefer `ERROR`.

## Log at owner boundaries

Log where the flow is owned, not in every helper.

Preferred places:

- lifecycle entry points
- worker activation boundaries
- supervisor/API boundaries
- queue/task/job completion handlers
- startup and recovery boundaries

Avoid duplicate stack spam from every internal helper in the same failure path.
Helpers should usually return contextual errors upward; the owner of the background workflow should emit the final operational log.

## Foreground CLI vs background flows

Foreground CLI output and daemon/service logging are not the same thing.

For one-shot interactive commands, direct CLI output may be enough.
For daemon, supervisor, server/API, and background worker paths, CLI output is not a substitute for logging.

If a user might later debug the issue through `bitloops daemon logs`, the flow should log it.

## Common cases that must be logged

Examples of events that should normally produce a log entry:

- daemon start, ready, restart, stop, shutdown
- supervisor start/stop/restart handoff failures
- worker activation failures
- background task or job terminal failures
- retry scheduling after failure
- startup recovery problems
- repository resolution or registration failures in backend request paths
- cleanup or state-persistence failures that change reliability guarantees

## Common mistakes

Avoid these patterns in daemon/service code unless the fallback is truly harmless:

- swallowing `Result` values with `let _ = ...`
- using `unwrap_or(false)` or `unwrap_or_default()` on operational checks without logging the failure that triggered the fallback
- returning an API error to the caller without logging the internal backend failure
- logging only in low-level helpers and never at the workflow boundary

## Review checklist

When adding or reviewing a daemon/service/backend flow, check:

1. Does the flow use the shared main logger?
2. Are terminal failures logged at `ERROR`?
3. Are retryable or degraded states logged at `WARN`?
4. Are startup, activation, shutdown, and recovery failures visible?
5. Can a user diagnose the failure through `bitloops daemon logs`?
6. Is logging emitted once at the right owner boundary rather than duplicated everywhere?

If the answer to any of those is no, the implementation is incomplete.
