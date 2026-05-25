# Agent Lifecycle Hook Spooling

## Purpose

Bitloops-managed agent hooks run inside short-lived `bitloops hooks ...` processes launched by agents. Slow lifecycle work should move to the long-lived daemon when the hook does not need to return stdout, an allow/deny decision, prompt mutation, or immediately visible state.

The durable handoff uses repo `runtime.sqlite` as an outbox/inbox-style work queue:

1. The hook process records a raw lifecycle hook payload.
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

### Already Async

| Agent | Hook | Event |
| --- | --- | --- |
| Codex | `stop` | `TurnEnd` |
| Claude Code | `stop` | `TurnEnd` |
| Gemini | `after-agent` | `TurnEnd` |
| Cursor | `stop` | `TurnEnd` |
| Copilot | `agent-stop` | `TurnEnd` |
| OpenCode | `turn-end` | `TurnEnd` |

### First Pilot

| Agent | Hook | Event | Reason |
| --- | --- | --- | --- |
| Claude Code | `session-end` | `SessionEnd` | Tail recording only. |
| Gemini | `session-end` | `SessionEnd` | Tail recording only. |
| Gemini | `pre-compress` | `Compaction` | Compaction state recording only. |
| Cursor | `pre-compact` | `Compaction` | Compaction state recording only. |
| Copilot | `session-end` | `SessionEnd` | Tail recording only. |
| OpenCode | `compaction` | `Compaction` | Compaction state recording only. |
| OpenCode | `session-end` | `SessionEnd` | Tail recording only. |
| Cursor | `session-end` | `SessionEnd` | Safe only with strict FIFO plus explicit failure logging/dead-letter visibility because it may finalize an open turn. |

### Keep Synchronous

Session-start and turn-start/prompt hooks stay synchronous in this phase because they establish state used by later hooks.

### Later Ordered Candidates

Subagent/task/todo and tool-observation hooks can move after the generic queue proves ordered and repo-root-safe.

### Pass-Through Hooks

Gemini tool/model/notification hooks and Copilot optional tool/error hooks currently parse to no lifecycle event. Do not enqueue them until they do useful lifecycle work.
