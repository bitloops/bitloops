# Daemon Logging MVP — Design Specification

**Feature name:** Daemon Logging MVP  
**Module(s):** Daemon Logger, Daemon CLI, Supervisor, Background Workflows  
**Status:** Draft  
**Priority:** P1  
**Target release:** MVP

---

## Executive Summary

Bitloops currently writes daemon-related logs into one global JSON-lines file, `daemon.log`, under the Bitloops state directory. `bitloops daemon logs` is a thin viewer over that file with `--tail`, `--follow`, and `--path`. The basic workflow is workable, but the current design has four gaps that make background daemon debugging brittle:

1. The log file grows forever and has no retention policy.
2. The viewer cannot filter by severity.
3. Timestamps are stored as stringified Unix-millis values, which are poor for direct human inspection.
4. The system does not yet define a consistent contract for which daemon-owned workflows must emit terminal `ERROR` logs, making silent failures too easy.

This spec defines a file-only MVP that keeps the current single shared `daemon.log` model and adds:

- Bitloops-managed internal size-based rotation and retention
- Repeatable `--level` filtering on `bitloops daemon logs`
- RFC3339 UTC timestamps in the log entry itself
- A small logging contract that requires start/success/failure logging for major daemon-owned workflows
- Explicit failure handling for logger initialization in background modes

The MVP stays intentionally narrow: no OS-native logging sinks, no compression, no rotated-file reading in the CLI, and no additional viewer filters beyond `--level`.

---

## Problem Statement

Bitloops runs important work in daemon-owned background flows: server startup, supervisor orchestration, DevQL task execution, enrichment, embeddings bootstrap, and external runtime management. When these flows fail, the log file is the main durable debugging surface, especially for detached and service-backed execution.

Today the log surface has the right foundation but not the right operational guarantees:

- The log sink is append-only and unbounded.
- Viewer-side inspection is all-or-nothing unless users manually pipe to other tools.
- The raw file is structured but not especially readable.
- Background-mode fallback to stderr is not safe when stdout/stderr are redirected to `/dev/null`.
- Severity usage is not yet specified strongly enough to guarantee that important background failures are always visible.

For MVP, Bitloops does not need a full logging platform. It needs a simple and reliable file-based contract that supports day-to-day diagnosis of warnings and errors.

### Design Principles

1. **Stay file-first** — use one shared file as the canonical sink for daemon-related logs.
2. **Own rotation internally** — do not depend on external `logrotate`, `journald`, or platform-native sinks for MVP correctness.
3. **Optimize for errors first** — the main operator value is reliable visibility into warnings and terminal failures.
4. **Keep the viewer simple** — `bitloops daemon logs` remains a file viewer, not a query engine.
5. **Avoid silent background loss** — background modes must not “log” into discarded stderr.
6. **Preserve structured logs** — keep raw JSONL as the wire format for both humans and tooling.

---

## Scope

### In Scope

- Single shared daemon log file, `daemon.log`
- Bitloops-managed rotation and retention for `daemon.log`
- `bitloops daemon logs --level <level>` exact-match filtering
- Repeatable `--level` flags for multi-level matching
- RFC3339 UTC timestamps with millisecond precision in log entries
- Logging contract for major daemon-owned workflows
- Background logger initialization failure policy
- Focused test coverage for rotation, filtering, timestamp formatting, and major failure paths

### Out of Scope

- OS-native sinks such as `journald`, `syslog`, `launchd` log routing, or Windows Event Log
- Compression of rotated log files
- Reading rotated log archives from `bitloops daemon logs`
- Additional viewer filters such as `--process`, `--mode`, or time-range filtering
- Threshold-style viewer flags such as `--min-level`
- A larger event taxonomy or a separate structured diagnostics subsystem

---

## Current State

The current log file path is resolved in `bitloops/src/daemon/logger.rs` by `daemon_log_file_path()`, which returns:

- `<Bitloops state dir>/logs/daemon.log`

The viewer implementation in `bitloops/src/cli/daemon.rs` does the following:

- `--path`: print the resolved path and exit
- otherwise: require the file to exist, read the last `N` lines, and print them
- `--follow`: after the initial tail, poll for appended lines and stream them until shutdown

The log entry is JSON built from:

- `time`
- `level`
- `msg`
- `target`
- `module`
- `file`
- `line`
- `pid`
- `process`
- `mode`
- `config_path`
- `service_name`

The shared file can currently contain entries from:

- daemon processes
- the supervisor
- daemon-related CLI control paths such as start/stop/restart

This shared-file design is worth preserving for MVP because it is already cross-platform, already structured, and already sufficient for the main debugging path.

## Storage Model

Bitloops will continue using one shared JSONL file as the canonical daemon log sink:

- active file: `daemon.log`
- rotated files: `daemon.log.1` through `daemon.log.5`

The path remains under the global Bitloops state directory. The file stays global rather than per-repo or per-config for MVP.

### Why One Shared File

One shared file is the simplest workable model because:

- it matches the current design
- the log entry already carries `process`, `mode`, and config metadata
- rotation remains straightforward
- user mental overhead stays low

Splitting logs by producer can be revisited later if operator workflows show that the shared file becomes difficult to inspect.

---

## Rotation And Retention

Bitloops will manage rotation internally rather than relying on external tools.

### Rotation Policy

- rotate when `daemon.log` exceeds `10 MiB`
- retain `5` rotated files
- delete the oldest archive when creating a new one beyond the retention limit
- do not compress rotated files in MVP

### Rotation Naming

- current file: `daemon.log`
- newest archive: `daemon.log.1`
- oldest retained archive: `daemon.log.5`

Rotation shifts files upward:

1. delete `daemon.log.5` if present
2. rename `daemon.log.4` to `daemon.log.5`
3. rename `daemon.log.3` to `daemon.log.4`
4. rename `daemon.log.2` to `daemon.log.3`
5. rename `daemon.log.1` to `daemon.log.2`
6. rename `daemon.log` to `daemon.log.1`
7. create a fresh `daemon.log`

### Why Internal Rotation

The current logger opens the file in append mode and keeps that file handle alive. External rename-based rotation would not be reliable because the daemon process could keep writing to the old inode after the path has moved. Internal rotation avoids that ambiguity and keeps the implementation self-contained and cross-platform.

### MVP Constraint

`bitloops daemon logs` reads only the active `daemon.log` in MVP. Rotated archives remain available for manual inspection, but the CLI does not attempt to merge or traverse them yet.

---

## Timestamp Format

The log entry timestamp will change from a stringified Unix-millis value to an RFC3339 UTC timestamp with millisecond precision.

### Required Format

Example:

```json
{
  "time": "2026-04-16T12:34:56.789Z",
  "level": "ERROR",
  "msg": "embeddings bootstrap failed"
}
```

### Timestamp Rules

- UTC only
- millisecond precision
- trailing `Z`
- stored directly in the existing `time` field

### Rationale

This format is:

- readable in the raw file
- structured and parseable
- lexicographically sortable
- unambiguous across machines and time zones

The MVP will not keep a second numeric timestamp field. A single canonical readable timestamp is the simpler default.

---

## Viewer Behavior

`bitloops daemon logs` remains a raw JSONL viewer over the active file.

### Supported Flags

- `--tail <N>`
- `--follow`
- `--path`
- repeatable `--level <level>`

### Level Filtering Semantics

When one or more `--level` flags are supplied, the viewer includes only log entries whose JSON `level` field exactly matches one of the requested severities.

Examples:

```bash
bitloops daemon logs --level warn
bitloops daemon logs --level error
bitloops daemon logs --level warn --level error
bitloops daemon logs --tail 500 --follow --level error
```

### Accepted User Input

The CLI should normalize level input case-insensitively and map:

- `debug` -> `DEBUG`
- `info` -> `INFO`
- `warn` -> `WARN`
- `warning` -> `WARN`
- `error` -> `ERROR`

### Tail Then Filter

For MVP, filtering happens after tail selection, not before.

That means:

- `--tail 200 --level error` reads the last 200 raw lines
- it then emits only the entries among those 200 lines whose `level` matches

The viewer does not scan backward until it finds 200 matching lines. This keeps the implementation simple and preserves the current `--tail` mental model.

### Follow Behavior

`--follow` keeps the existing polling model. New lines are filtered before printing. The polling cadence can remain as currently implemented unless rotation handling requires a minor adjustment.

### Non-JSON Lines

The file is expected to be Bitloops-owned JSONL. If the viewer encounters malformed or non-JSON lines while filtering, it should skip those lines and continue rather than abort the entire stream.

---

## Writer Threshold Vs Viewer Filter

Bitloops already has a producer-side threshold through:

- `BITLOOPS_LOG_LEVEL`
- `[logging].level`

That mechanism remains unchanged in MVP and continues to control what gets written in the first place.

The new `--level` flag is a viewer-side filter only. It does not affect the producer threshold and does not mutate daemon configuration.

This distinction is important:

- producer threshold controls log volume
- viewer filtering controls operator inspection

For MVP, viewer filtering is the preferred new capability because it lets operators focus on warnings and errors without reducing what the daemon records overall.

---

## Logging Contract

The MVP introduces a minimal logging contract for daemon-owned workflows.

### Severity Rules

- `INFO`: workflow start and successful completion
- `WARN`: degraded, retryable, or recoverable conditions
- `ERROR`: terminal failures or failures that are about to be swallowed, retried later, or surfaced to callers
- `DEBUG`: noisy inner-loop progress and developer diagnostics

### Ownership Rule

The component that owns the background workflow is responsible for the terminal log entry. Lower-level helpers should attach error context and return errors upward, but the owner should emit the canonical terminal `ERROR` log.

This avoids both silent failures and duplicate failure spam.

### Required Workflow Coverage

The contract applies to these daemon-owned surfaces:

- daemon start, ready, stop, restart, and shutdown
- supervisor start, stop, restart, and handoff failures
- daemon config resolution and store/bootstrap failures
- DevQL task enqueue, start, complete, fail, retry, and cancel
- enrichment workflow failures
- embeddings bootstrap phase failures
- external runtime launch, timeout, crash, and recovery paths

### Error Visibility Rule

Any background failure that can cause:

- a daemon workflow to stop making progress
- a user-visible task to fail
- a retry loop to defer work
- a background service to restart or degrade

must result in at least one `ERROR` log entry at the workflow owner boundary.

### Noise Control

Bitloops should not log per-path or per-artefact success entries at `INFO` in hot loops. The MVP should focus on workflow boundaries and failures so the active log remains useful without immediately requiring aggressive filtering.

---

## Logger Initialization Policy

Foreground interactive commands may continue falling back to stderr if file logger initialization fails.

Detached, service, and supervisor modes must not do that.

### Required Policy

If file logger initialization fails in:

- detached daemon mode
- service-backed daemon mode
- supervisor mode

startup must fail rather than silently degrade to stderr.

### Rationale

In background modes, stdout/stderr are redirected away from the user, so stderr fallback is effectively log loss. For MVP, reliable file logging is a startup requirement for background execution.

---

## CLI And UX Contract

The user-facing behavior for `bitloops daemon logs` becomes:

- raw JSONL output only
- readable UTC timestamps embedded in each line
- exact-match repeatable severity filtering
- unchanged `--path` behavior
- unchanged default tail count unless explicitly changed in implementation

Examples:

```bash
bitloops daemon logs
bitloops daemon logs --tail 50
bitloops daemon logs --level error
bitloops daemon logs --level warn --level error
bitloops daemon logs --follow --level error
bitloops daemon logs --path
```

This stays aligned with standard daemon-log inspection expectations while avoiding a larger log-query surface.

---

## Testing Requirements

The MVP must add focused tests for the failure-prone areas.

### Rotation

- log file rotates when it exceeds the configured size threshold
- existing archives shift upward correctly
- oldest archive is deleted once the retention limit is exceeded
- active `daemon.log` continues receiving new entries after rotation

### Timestamp

- log entries encode `time` in RFC3339 UTC format with millisecond precision and trailing `Z`

### Viewer Filtering

- `--level error` emits only `ERROR` entries
- repeated `--level warn --level error` emits both levels
- input normalization handles `warn` and `warning`
- `--follow` applies filtering to appended lines

### Failure Policy

- background logger initialization failure is fatal in detached mode
- background logger initialization failure is fatal in service mode
- background logger initialization failure is fatal in supervisor mode

### Workflow Coverage

Representative tests should prove that major daemon-owned workflows emit terminal `ERROR` entries on failure, especially:

- daemon startup/bootstrap failure
- supervisor handoff failure
- task execution failure
- enrichment failure
- embeddings bootstrap failure
- runtime timeout or crash recovery

The goal is not exhaustive log snapshot testing for every branch. The goal is to prove that the major failure boundaries cannot fail silently.

---

## Implementation Notes

This spec does not prescribe a specific concrete writer type, but the implementation should preserve:

- append-safe writes
- a single JSON formatter path
- minimal call-site churn outside the major workflow boundaries

The preferred shape is to keep the current structured log entry builder and replace the raw file target with a small Bitloops-owned rotating file sink.

The CLI-side filtering should be added in the existing `bitloops daemon logs` implementation rather than as a separate query layer or a new subcommand.

---

## Risks And Tradeoffs

### Known MVP Tradeoffs

- `bitloops daemon logs` reads only the active file, not rotated archives
- severity filtering happens after tailing, not semantic tail-by-level
- file-only logging remains less integrated than platform-native sinks

### Why These Are Acceptable

They keep the MVP small while solving the main operational problems:

- bounded file growth
- usable inspection of warnings and errors
- readable timestamps
- explicit rules against silent background failures

---

## Rollout Recommendation

Implement the MVP in this order:

1. update the timestamp format and log sink abstraction
2. add internal rotation and retention
3. add viewer-side `--level` filtering
4. enforce background logger-init failure policy
5. add the workflow-boundary logs required by the contract
6. add focused tests

This ordering establishes the durable sink first, then improves inspection, then closes silent-failure gaps.

---

## Summary

The daemon logging MVP keeps the current Bitloops logging model deliberately simple:

- one shared JSONL file
- internal size-based rotation with bounded retention
- readable UTC timestamps
- exact-match severity filtering in `bitloops daemon logs`
- explicit logging requirements for major daemon-owned workflow failures

That is sufficient for an MVP because it gives Bitloops a reliable, cross-platform, low-complexity debugging surface without committing to a heavier logging subsystem too early.
