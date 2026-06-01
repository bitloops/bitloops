# Agent Lifecycle Hook Spooling

## Purpose

Bitloops-managed agent hooks run inside short-lived `bitloops hooks ...` processes launched by agents. Slow lifecycle work should move to the long-lived daemon when the hook does not need to return stdout, an allow/deny decision, prompt mutation, or immediately visible state.

The durable handoff uses repo `runtime.sqlite` as an outbox/inbox-style work queue:

1. The hook process records a raw lifecycle hook payload plus any required boundary snapshot.
2. The hook process returns quickly.
3. The daemon claims one job at a time.
4. The daemon parses the raw payload through the existing agent lifecycle adapter.
5. The daemon runs lifecycle handlers with an explicit repo root.

No daemon RPC and no file-inbox fallback are used.

## Queue Semantics

- One generic table stores lifecycle hook jobs: `agent_lifecycle_spool_jobs`.
- Jobs are processed sequentially by durable monotonic SQLite enqueue sequence, not by timestamp ordering or UUID tie-breaks.
- The initial ordering scope is repo-runtime-wide: the daemon processes jobs globally and sequentially, not as per-session queues in parallel.
- The daemon claims one job at a time.
- A failed head job blocks later jobs until it retries or becomes failed after the max attempts budget.
- Once a head job is marked failed/dead-lettered, later jobs may proceed globally, but this is a degraded path. Ordered hook handlers must be duplicate-tolerant and should tolerate missing earlier state.
- Processing is at-least-once. Handlers must be idempotent or duplicate-tolerant.
- Git hooks are out of scope.

## Hook Classification

### Default Async Policy

Lifecycle-producing hooks should enqueue through `agent_lifecycle_spool_jobs` unless the hook must synchronously return agent-visible output, allow/deny/modify a tool call, or inject prompt/session content before the agent continues.

### Boundary Snapshot Policy

The hook process captures volatile state before enqueue:

| Snapshot kind | Captures | Used by |
| --- | --- | --- |
| none | raw hook payload only | SessionStart, SessionEnd, Compaction, pure observation hooks |
| pre-boundary | untracked files; transcript offset for TurnStart only | TurnStart, SubagentStart |
| workspace | modified/new/deleted files | TurnEnd, SubagentEnd, shell TurnEnd, mixed Codex post-tool hooks |
| workspace plus branch | workspace plus default-branch decision | TodoCheckpoint |

The daemon consumes these snapshots when replaying the raw hook payload. It must not recompute hook-boundary state when a snapshot is present.

### No-Op Pass-Through Hooks

Gemini tool/model/notification hooks and Copilot optional tool/error hooks still parse to no lifecycle event. They are not enqueued until they perform actual Bitloops lifecycle work.
