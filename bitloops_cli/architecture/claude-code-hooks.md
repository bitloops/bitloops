# Claude Code Hooks

## Go Reference
- `golang-reference/cmd/entire/cli/agent/claudecode/hooks.go` — `InstallHooks`, `UninstallHooks`, `AreHooksInstalled`
- `golang-reference/cmd/entire/cli/agent/claudecode/types.go` — JSON struct definitions
- `golang-reference/cmd/entire/cli/agent/claudecode/hooks_test.go` — unit tests (ported to Rust)
- `golang-reference/cmd/entire/cli/hooks_cmd.go` — hook command dispatcher
- `golang-reference/cmd/entire/cli/hook_registry.go` — hook handler routing
- `golang-reference/cmd/entire/cli/agent/claudecode/lifecycle.go` — per-hook event parsers
- `golang-reference/cmd/entire/cli/lifecycle.go` — lifecycle event handlers (TurnStart/TurnEnd/etc.)
- `golang-reference/cmd/entire/cli/hooks_claudecode_posttodo.go` — PostTodo handler
- `golang-reference/cmd/entire/cli/session/phase.go` — SessionPhase enum, state machine
- `golang-reference/cmd/entire/cli/session/state.go` — SessionState struct, StateStore
- `golang-reference/cmd/entire/cli/strategy/strategy.go` — Strategy interface, StepContext
- `golang-reference/cmd/entire/cli/strategy/hooks.go` — git hook script install/uninstall
- `golang-reference/cmd/entire/cli/hooks_git_cmd.go` — `bitloops hooks git` dispatcher
- `golang-reference/cmd/entire/cli/strategy/manual_commit_git.go` — ManualCommitStrategy save_step/save_task_step
- `golang-reference/cmd/entire/cli/strategy/manual_commit_hooks.go` — ManualCommitStrategy git hook handlers

## Rust Implementation
- `src/engine/agent/claude_code/hooks.rs` — install/uninstall/detect hooks in `.claude/settings.json`
- `src/engine/agent/claude_code/hooks_cmd.rs` — hook handler dispatcher + 7 handler functions
- `src/engine/agent/claude_code/git_hooks.rs` — install/uninstall 4 git hook scripts in `.git/hooks/`
- `src/engine/hooks/git.rs` — `bitloops hooks git <verb>` dispatcher
- `src/engine/session/phase.rs` — SessionPhase, HookEvent, transition()
- `src/engine/session/state.rs` — SessionState, PrePromptState, PreTaskState
- `src/engine/session/backend.rs` — SessionBackend trait
- `src/engine/session/local_backend.rs` — LocalFileBackend (+ list_sessions())
- `src/engine/strategy/mod.rs` — Strategy trait, StepContext, TaskStepContext
- `src/engine/strategy/noop.rs` — NoOpStrategy (all methods are no-ops)
- `src/engine/strategy/manual_commit.rs` — ManualCommitStrategy (shadow branches + checkpoints)

---

## Settings File

Claude Code hooks are written to `.claude/settings.json` relative to the repo root.
The constant `CLAUDE_SETTINGS_FILE_NAME = "settings.json"` (Go reference line 32).

**Important**: The file is parsed as `serde_json::Value` to preserve all unknown fields
and unknown hook types that Claude Code may add in future versions.

## Hooks Installed

| Hook Type | Matcher | Command |
|-----------|---------|---------|
| `SessionStart` | `""` | `bitloops hooks claude-code session-start` |
| `SessionEnd` | `""` | `bitloops hooks claude-code session-end` |
| `Stop` | `""` | `bitloops hooks claude-code stop` |
| `UserPromptSubmit` | `""` | `bitloops hooks claude-code user-prompt-submit` |
| `PreToolUse` | `"Task"` | `bitloops hooks claude-code pre-task` |
| `PostToolUse` | `"Task"` | `bitloops hooks claude-code post-task` |
| `PostToolUse` | `"TodoWrite"` | `bitloops hooks claude-code post-todo` |

Permissions deny rule: `"Read(./.bitloops/metadata/**)"`
(Go original: `"Read(./.entire/metadata/**)"`)

## JSON Structure

```json
{
  "hooks": {
    "SessionStart": [
      {"matcher": "", "hooks": [{"type": "command", "command": "bitloops hooks claude-code session-start"}]}
    ],
    "Stop": [
      {"matcher": "", "hooks": [{"type": "command", "command": "bitloops hooks claude-code stop"}]}
    ],
    "PreToolUse": [
      {"matcher": "Task", "hooks": [{"type": "command", "command": "bitloops hooks claude-code pre-task"}]}
    ],
    "PostToolUse": [
      {"matcher": "Task", "hooks": [{"type": "command", "command": "bitloops hooks claude-code post-task"}]},
      {"matcher": "TodoWrite", "hooks": [{"type": "command", "command": "bitloops hooks claude-code post-todo"}]}
    ]
  },
  "permissions": {
    "deny": ["Read(./.bitloops/metadata/**)"]
  }
}
```

## Hook Identification

`is_bitloops_hook(cmd: &str) -> bool`: returns true if command starts with `"bitloops "`.

Go equivalent: `isEntireHook()` in `hooks.go` (line 457), checking prefix `"entire "`.

## Public API

```rust
pub fn install_hooks(repo_root: &Path, force: bool) -> Result<usize>
pub fn uninstall_hooks(repo_root: &Path) -> Result<()>
pub fn are_hooks_installed(repo_root: &Path) -> bool
```

## Key Behaviors (hooks.rs)

- **Idempotent install**: each hook and deny rule is checked before adding — no duplicates
- **Preserve unknown hook types**: `Notification`, `SubagentStop`, etc. are kept as-is
- **Preserve user hooks**: hooks not starting with `"bitloops "` are never removed
- **Preserve unknown permission fields**: `ask`, `customField`, etc. kept in `permissions` object
- **Force reinstall**: if `force=true`, all bitloops hooks are removed first, then reinstalled
- **Clean empty arrays**: after uninstall, empty hook type arrays are removed from JSON

---

## Hook Handler Commands (`hooks_cmd.rs`)

Claude Code calls `bitloops hooks claude-code <verb>` with JSON on stdin. The command is hidden
(`#[command(hide = true)]`) and not shown in `bitloops --help`.

### Common behavior for all hooks
1. Skip silently if not inside a git repository (walk up from `cwd` looking for `.git/`)
2. Skip silently if Bitloops is disabled (`settings::is_enabled()`)
3. Parse JSON from stdin; fail with error on malformed input
4. Use `LocalFileBackend` for all state persistence

### Handler summary

| Hook verb | Go event | What it does |
|-----------|----------|-------------|
| `session-start` | `SessionStart` | Load/create session, transition → Idle, save |
| `user-prompt-submit` | `TurnStart` | Initialize session, transition → Active, save pre-prompt state |
| `stop` | `TurnEnd` | Call `strategy.save_step()`, transition → Idle, delete pre-prompt state |
| `session-end` | `SessionEnd` | Transition → Ended, set `ended_at`, save |
| `pre-task` | `SubagentStart` | Create pre-task marker file, update session |
| `post-task` | `SubagentEnd` | Call `strategy.save_task_step()`, delete marker, increment step_count |
| `post-todo` | (special) | If pre-task marker exists → call `strategy.save_task_step(is_incremental=true)` |

### Input types (parsed from stdin JSON)

| Struct | Used by |
|--------|---------|
| `SessionInfoInput { session_id, transcript_path }` | session-start, stop, session-end |
| `UserPromptSubmitInput { session_id, transcript_path, prompt }` | user-prompt-submit |
| `TaskHookInput { session_id, transcript_path, tool_use_id }` | pre-task, post-task |
| `PostTodoInput { session_id, transcript_path, tool_use_id, tool_name }` | post-todo |

---

## Session State Machine

Go reference: `golang-reference/cmd/entire/cli/session/phase.go`
Rust: `src/engine/session/phase.rs`

### Phases

| Phase | Meaning |
|-------|---------|
| `Idle` | Session exists, agent not active (default) |
| `Active` | Agent is processing a turn |
| `Ended` | Session has been closed |

### Transitions

| Current | Event | New |
|---------|-------|-----|
| Any | SessionStart | Idle |
| Idle | TurnStart | Active |
| Active | TurnStart | Active (Ctrl-C recovery) |
| Ended | TurnStart | Active (re-entry) |
| Active | TurnEnd | Idle |
| Idle | TurnEnd | Idle |
| Any | SessionEnd | Ended |

---

## Storage Paths

Go uses `.git/entire-sessions/`; Rust adapts to `.git/bitloops-sessions/`.

| Data | Path |
|------|------|
| Session state | `.git/bitloops-sessions/<session_id>.json` |
| Pre-prompt state | `.bitloops/tmp/pre-prompt-<session_id>.json` |
| Pre-task marker | `.bitloops/tmp/<tool_use_id>.pretask` |
| Session metadata | `.bitloops/metadata/<session_id>/` |
| Shadow branch | `bitloops/<hash>-<worktree>` |
| Checkpoints branch | `bitloops/checkpoints/v1` |

---

## Strategy Interface

Go reference: `golang-reference/cmd/entire/cli/strategy/strategy.go`
Rust: `src/engine/strategy/mod.rs`

Hooks delegate checkpoint creation to the strategy rather than creating git objects directly.
This mirrors the Go design where `lifecycle.go` calls `strategy.SaveStep()` / `strategy.SaveTaskStep()`.

Active implementation: `ManualCommitStrategy` — creates shadow branch commits on every `stop` hook,
then condenses session data to `bitloops/checkpoints/v1` when the user runs `git commit`.

```rust
pub trait Strategy: Send + Sync {
    fn save_step(&self, ctx: &StepContext) -> Result<()>;
    fn save_task_step(&self, ctx: &TaskStepContext) -> Result<()>;
    fn prepare_commit_msg(&self, commit_msg_file: &Path, source: Option<&str>) -> Result<()>;
    fn commit_msg(&self, commit_msg_file: &Path) -> Result<()>;
    fn post_commit(&self) -> Result<()>;
    fn pre_push(&self, remote: &str) -> Result<()>;
}
```

---

## Git Hook Scripts

Go reference: `golang-reference/cmd/entire/cli/strategy/hooks.go`
Rust: `src/engine/agent/claude_code/git_hooks.rs`

`bitloops enable` installs 4 shell scripts into `.git/hooks/`. Each script calls the
`bitloops hooks git <verb>` subcommand and exits 0 on error (hooks must not block git).

### Scripts installed

| Script | Command called | On error |
|--------|---------------|----------|
| `prepare-commit-msg` | `bitloops hooks git prepare-commit-msg "$1" "$2"` | `2>/dev/null \|\| true` |
| `commit-msg` | `bitloops hooks git commit-msg "$1"` | `\|\| exit 1` |
| `post-commit` | `bitloops hooks git post-commit` | `2>/dev/null \|\| true` |
| `pre-push` | `bitloops hooks git pre-push "$1"` | `\|\| true` |

### Hook identification and backup

Every script contains the marker comment `# Bitloops git hooks`. This is how
`uninstall_git_hooks` identifies managed scripts vs. user-written hooks.

If a pre-existing hook file exists without the marker, it is moved to `<name>.pre-bitloops`
and chained at the end of the new script:
```sh
_dir="$(dirname "$0")"
[ -x "$_dir/<name>.pre-bitloops" ] && "$_dir/<name>.pre-bitloops" "$@"
```
`uninstall_git_hooks` restores `.pre-bitloops` backups.

### Public API

```rust
pub fn install_git_hooks(repo_root: &Path, local_dev: bool) -> Result<usize>
pub fn uninstall_git_hooks(repo_root: &Path) -> Result<usize>
pub fn is_git_hook_installed(repo_root: &Path) -> bool
```

---

## `bitloops hooks git` Subcommand

Go reference: `golang-reference/cmd/entire/cli/hooks_git_cmd.go`
Rust: `src/engine/hooks/git.rs`

The 4 git hook scripts call `bitloops hooks git <verb>`. This is a hidden subcommand
dispatched from `bitloops hooks`. Guard: skips silently when not in a git repo or when
Bitloops is disabled. All errors are printed as warnings but never propagate (exit 0).

| Verb | Argument | Calls |
|------|----------|-------|
| `prepare-commit-msg` | `<commit_msg_file> [source]` | `ManualCommitStrategy::prepare_commit_msg()` |
| `commit-msg` | `<commit_msg_file>` | `ManualCommitStrategy::commit_msg()` |
| `post-commit` | _(none)_ | `ManualCommitStrategy::post_commit()` |
| `pre-push` | `<remote>` | `ManualCommitStrategy::pre_push()` |

---

## ManualCommitStrategy

Go references: `manual_commit_git.go`, `manual_commit_hooks.go`
Rust: `src/engine/strategy/manual_commit.rs`

### Workflow

1. **`stop` hook** → `save_step()` — takes a snapshot of the working tree on a shadow branch
   `refs/heads/bitloops/<HEAD[:7]>` using `GIT_INDEX_FILE` to avoid touching the staging area
2. **`git commit` — `prepare-commit-msg`** → appends `Bitloops-Checkpoint: <12hexid>` trailer
3. **`git commit` — `commit-msg`** → strips the trailer if the message would otherwise be empty
4. **`git commit` — `post-commit`** → reads the trailer from HEAD, condenses session state to
   `bitloops/checkpoints/v1` branch
5. **`git push` — `pre-push`** → pushes `bitloops/checkpoints/v1` alongside the user's push

### Shadow branch commit (`save_step`)

```
1. head = git rev-parse HEAD
2. shadow_branch = refs/heads/bitloops/<head[:7]>
3. Detect changed files: git status --porcelain (or from StepContext)
4. Build tree via GIT_INDEX_FILE temp file:
   a. git read-tree <parent_tree>      (if branch exists)
   b. git update-index --add -- <modified+new>
   c. git update-index --remove -- <deleted>
   d. tree = git write-tree
5. Skip if tree == parent_tree (dedup)
6. commit = git commit-tree <tree> [-p <parent>] -m "<msg with trailers>"
   Trailers: Bitloops-Metadata, Bitloops-Session, Bitloops-Strategy
7. git update-ref refs/heads/bitloops/<head[:7]> <commit>
8. Update session state: base_commit=head, step_count++, files_touched+=
```

### Checkpoint condensation (`post_commit`)

```
1. Read Bitloops-Checkpoint: <id> trailer from HEAD via git cat-file commit HEAD
2. If no trailer: update base_commit for active sessions, return
3. For each non-ended session: condense_session(state, id, HEAD)
   a. Write CheckpointMetadata JSON to .bitloops/tmp/cp-<id>/<id[:2]>/<id[2:]>/metadata.json
   b. Build tree (optionally reading bitloops/checkpoints/v1 first)
   c. git commit-tree → git update-ref refs/heads/bitloops/checkpoints/v1
   d. Clean up .bitloops/tmp/cp-<id>/
   e. Reset session: base_commit=HEAD, step_count=0, files_touched=[]
```

### Key helpers

```rust
pub fn run_git(repo_root: &Path, args: &[&str]) -> Result<String>
pub fn run_git_env(repo_root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String>
pub fn parse_checkpoint_id(message: &str) -> Option<String>
fn build_tree(repo_root, parent_tree, modified, new_files, deleted) -> Result<String>
fn shadow_branch_ref(head_hash: &str) -> String   // "refs/heads/bitloops/<hash[:7]>"
fn generate_checkpoint_id() -> String             // uuid::Uuid simple [..12]
fn get_checkpoint_id_from_head(repo_root) -> Result<Option<String>>
fn is_git_sequence_operation(repo_root) -> bool   // rebase/cherry-pick detection
fn working_tree_changes(repo_root) -> Result<(modified, new, deleted)>
```

`TempIndexPath` — RAII wrapper that creates a unique temp path and deletes the file on drop,
used to avoid touching the user's real git index during tree construction.

---

## Remaining Work For 100% Go Parity

The current Rust implementation is close, but not yet a full 1:1 port of the Go lifecycle and
manual-commit behavior. The following items are still required for strict parity.

### 1) Lifecycle Dispatcher Parity

Rust currently routes Claude hooks directly to handler functions. Go uses a normalized lifecycle
event pipeline (`ParseHookEvent -> DispatchLifecycleEvent`) with shared orchestration across agents.

Needed:
- Introduce lifecycle event parsing and dispatch flow equivalent to Go.
- Ensure hook behavior is driven by lifecycle events, not ad-hoc per-hook logic.

### 2) Full Turn-End Pipeline (`stop`) Parity

Go `handleLifecycleTurnEnd` performs a full orchestration path that is still simplified in Rust.

Needed:
- Transcript existence and readiness checks equivalent to Go.
- Transcript flush waiting behavior parity.
- Prompt/summary/modified-file extraction from transcript offset.
- Pre-untracked/new/deleted filtering parity.
- Full `StepContext` population parity:
  - `metadata_dir`, `metadata_dir_abs`
  - generated commit message from prompt
  - git author name/email
  - transcript identifier/start offset
  - token usage
- Turn-end phase transition + cleanup parity behavior.

### 3) Subagent Hook Parity (`pre-task`, `post-task`, `post-todo`)

Rust subagent flow is still partial compared to Go.

Needed:
- Parse and use full task payload (`tool_input`, `tool_response.agentId`).
- Resolve subagent transcript path and include subagent file extraction.
- Match Go checkpoint UUID detection and incremental checkpoint sequencing.
- Match Go pre-task state handling for nested subagents and active-marker selection.

### 4) Post-Commit State-Machine Parity

Go `manual_commit_hooks.go` includes richer post-commit behavior than current Rust.

Needed:
- Session content detection parity (`sessionHasNewContent` + live transcript fallback).
- Overlap checks between session files and committed/staged files.
- Carry-forward of remaining uncommitted files to new shadow branches.
- Correct shadow branch cleanup rules based on session phase/condense outcome.
- Full state-machine transition behavior parity around git-commit events.

### 5) Attribution + Token Accounting Parity

Go tracks prompt attribution and token usage with richer lifecycle integration.

Needed:
- Implement/port missing attribution fields/flow:
  - pending prompt attribution
  - historical prompt attributions
- Ensure transcript position fields are updated exactly as in Go after each phase.
- Match token usage accumulation behavior from turn and subagent events.

### 6) Metadata/Schema Exactness

Rust now writes the expected metadata files under `.bitloops/metadata/<session_id>/`, but exact
schema/field semantics still need strict alignment with Go checkpoint metadata and lifecycle outputs.

Needed:
- Verify all JSON fields/semantics against Go output for committed checkpoint metadata.
- Validate context/summary/prompt generation behavior matches Go edge-case handling.

### 7) Test Coverage Parity (Unit + Integration + E2E)

Rust test suite passes, but total coverage scope is not yet equivalent to Go.

Needed:
- Port key Go integration/e2e scenarios for:
  - turn-end/lifecycle orchestration
  - mid-session commits
  - post-commit overlap and carry-forward
  - subagent checkpoint workflows
  - phase transition edge cases
- Add regression tests for transcript flush timing race conditions.

### Current Status Snapshot

- Implemented in Rust:
  - Hook install/uninstall behavior for Claude settings.
  - Session state persistence and tmp state handling.
  - Manual-commit core shadow branch + checkpoint branch flow.
  - Metadata file creation in `.bitloops/metadata/<session_id>/`:
    - `full.jsonl`
    - `prompt.txt`
    - `summary.txt`
    - `context.md`
  - Empty-repo (`no HEAD`) no-op handling for stop/post-commit paths.

- Still pending for strict parity:
  - Full lifecycle orchestration and post-commit/session-content logic from Go.
  - Full subagent and attribution/token parity.
  - Equivalent integration/e2e test breadth.
