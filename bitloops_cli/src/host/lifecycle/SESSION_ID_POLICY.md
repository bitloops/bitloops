# Session ID Policy

## Decision

Bitloops uses a **strict-first, fallback-at-tail** session ID policy for lifecycle and hook handling.

1. `SessionStart` and `TurnStart` are **strict**.
2. `TurnEnd`/`stop` is **tolerant** and may fall back to `"unknown"`.
3. Other events (`SessionEnd`, `Compaction`, subagent hooks) generally **preserve empty** values and should avoid creating synthetic sessions.

This policy is centralised in:

- `bitloops_cli/src/engine/lifecycle/mod.rs`
  - `SessionIdPolicy`
  - `apply_session_id_policy(...)`
  - `UNKNOWN_SESSION_ID`

## Why

We previously had agent-specific normalisers in multiple places. That caused drift and mixed behaviour between agents.

This decision improves:

1. **Correctness at turn/session start**: no silent creation of fake session IDs when a hook payload is incomplete.
2. **Operational resilience at turn end**: still attempt best-effort completion when stop arrives without a usable session ID.
3. **Consistency across agents**: one policy, one implementation, predictable behaviour.

## Rules

1. Do not add per-agent `normalize_*session_id` helpers.
2. Always use `apply_session_id_policy(...)` when deriving lifecycle session IDs.
3. Use `SessionIdPolicy::Strict` for any event that starts a session or turn.
4. Use `SessionIdPolicy::FallbackUnknown` only for tail completion (`TurnEnd`/`stop`) paths.
5. Use `SessionIdPolicy::PreserveEmpty` when a hook can legitimately be a no-op without a session ID.

## Current Enforcement

1. Lifecycle core:
   - `handle_lifecycle_session_start` uses strict validation.
   - `handle_lifecycle_turn_start` uses strict validation.
   - `handle_lifecycle_turn_end` uses fallback-to-unknown.
2. Hook runtime (`agent_runtime`):
   - `handle_session_start` is strict.
   - `handle_user_prompt_submit*` is strict.
   - `handle_stop_with_profile` is fallback-to-unknown.
3. Agent lifecycle parsing:
   - Codex: strict on `session-start`, fallback on `stop`.
   - Cursor: strict on `session-start` and `before-submit-prompt`, fallback on `stop`.
4. Dispatcher:
   - Must not pre-normalise empty IDs to `"unknown"` for start paths.
   - Tail fallback remains in stop handling.

## Test Expectations

Any session ID policy change must include tests for:

1. Empty session ID on `SessionStart` and `TurnStart` => strict rejection.
2. Empty session ID on `TurnEnd`/`stop` => fallback to `"unknown"` when intended.
3. No state/checkpoint conflation caused by unknown fallback.
