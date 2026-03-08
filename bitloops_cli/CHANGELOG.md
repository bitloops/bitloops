# Changelog

## [0.0.7]

### Changed

- Performed a repository-wide Clippy cleanup and resolved strict `-D warnings` issues across engine, commands, dashboard, and test modules.
- Refactored `src/engine/strategy/auto_commit.rs` metadata commit writer to use a structured input object, reducing argument count and improving maintainability.
- Applied non-functional code-quality improvements (collapsed conditionals, derived defaults, borrow/slice simplifications, and test helper cleanup) without intended behavior changes.

## [0.0.6]

### Changed

- `src/engine/hooks/runtime/agent_runtime.rs` — Stop hook is now tolerant when session/pre-prompt state is missing (Go parity): it normalizes empty session IDs to `unknown`, still computes and persists step checkpoints when file changes exist, preserves best-effort turn-end handling, and always clears pre-prompt state for the normalized session.
- `src/engine/session/state.rs` — Extended `PrePromptState` with a `source` field and added `PRE_PROMPT_SOURCE_CURSOR_SHELL` so shell-fallback turn boundaries can be tagged explicitly.
- `src/engine/hooks/dispatcher.rs` — Added Cursor CLI fallback flow: `before-shell-execution` synthesizes a pre-prompt boundary (only when one does not already exist), tags it as shell-originated, and `after-shell-execution` finalizes the turn by calling stop only for shell-originated pre-prompts; also normalizes empty Cursor conversation IDs and synthesizes prompt text from executed commands.
- `src/engine/hooks/dispatcher.rs` — `session-end` now includes a defensive turn-finalization fallback: it runs Cursor turn-end (`stop`) when pre-prompt state exists, session state is missing, or the session is still ACTIVE, then marks the session ENDED. This covers Cursor CLI runs where `stop` is not emitted reliably.
- `src/engine/hooks/dispatcher.rs` — Expanded Cursor `session-end` fallback criteria to also finalize turns for `IDLE` sessions with `checkpoint_count == 0` (no prior saved step), addressing Cursor CLI runs that emit `session-start`/`session-end` without `before-submit-prompt`/`stop`.
- `src/engine/agent/cursor/types.rs` — Added Cursor hook schema support for `beforeShellExecution` and `afterShellExecution`, plus raw payload structs for both events.
- `src/engine/agent/cursor/lifecycle.rs` and `src/engine/agent/cursor/agent.rs` — Registered Cursor lifecycle hook names/verbs for `before-shell-execution` and `after-shell-execution`.
- `src/engine/agent/cursor/hooks.rs` — Hook installer/uninstaller and detection now manage `beforeShellExecution`/`afterShellExecution` alongside existing Cursor hooks, including legacy migration and idempotency behavior updates; hook presence detection now requires the full managed hook set so legacy 7-hook installs are treated as incomplete and upgraded.
- `src/engine/hooks/runtime/agent_runtime.rs` tests — Added regressions for tolerant stop behavior without pre-existing session state, empty-session fallback to `unknown`, shell-fallback pre-prompt creation, shell-fallback stop execution, and ignoring `after-shell-execution` for non-shell pre-prompts.

## [0.0.5]

### Changed

- `src/engine/hooks/runtime/agent_runtime.rs` — Stop hook now follows Go turn-end parity for modified files: transcript-derived files are the primary list, git-status modified files are always merged in as fallback, and merge order preserves transcript-first semantics; added uncommitted-only filtering so files already committed to `HEAD` mid-turn are excluded from `save_step`.
- `src/engine/lifecycle/mod.rs` — Lifecycle turn-end now includes transcript-derived modified files (when a transcript analyzer is available), merges them with git-status modified files using transcript-first merge semantics, and filters out files already committed to `HEAD`.
- `src/engine/hooks/runtime/agent_runtime.rs` tests — Added regressions for (1) transcript-first merge ordering and (2) filtering transcript-only files already committed mid-turn.

## [0.0.4]

### Changed

- `src/server/dashboard/dto.rs` — Added `ApiCommitFileDiffDto` struct with `additionsCount` and `deletionsCount` fields; extended `ApiCommitDto` with a `files_touched` map (`HashMap<String, ApiCommitFileDiffDto>`); registered `ApiCommitFileDiffDto` in the OpenAPI schema components.
- `src/server/dashboard/mod.rs` — Added `parse_numstat_output` helper that parses `git show --numstat` output (handles binary files and malformed lines); added `read_commit_numstat` that runs git and delegates to the parser.
- `src/server/dashboard/handlers.rs` — `handle_api_commits` now fetches per-file diff stats for each commit via `read_commit_numstat` and populates `commit.files_touched`; failures are logged as warnings and result in an empty map rather than a request error; updated `api_commit_row_from_pair` signature to accept a precomputed `files_touched` map.
- `src/server/dashboard/tests.rs` — Extended `api_commits_filters_by_user_agent_and_time` to assert `files_touched` additions/deletions counts; added unit tests for `parse_numstat_output` covering normal lines, binary files, malformed lines, and duplicate-path accumulation.
