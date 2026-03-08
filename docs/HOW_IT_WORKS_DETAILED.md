# Entire Go CLI Deep Research Report

## Scope and method

This report documents a deep code-level analysis of:

- `docs/PROBLEM.md`
- `golang-reference/cmd/entire/...`
- `golang-reference/docs/architecture/...`
- Strategy, checkpoint store, hook integration, and command behavior
- Integration/e2e test intent and covered edge cases

The goal is to explain how the Entire CLI actually works end-to-end, including the tricky details that matter in production workflows.

## 1. Problem statement and product intent

From `docs/PROBLEM.md`, Entire exists to bridge a structural gap:

- AI sessions are ephemeral and hard to recover
- Git captures what changed, not why/how the AI got there
- There is no natural rewind mechanism at session boundaries
- Teams/orgs lack auditability and attribution for AI contribution

Entire’s design answer is:

- Persist AI session context in git-native structures
- Link commit history to session metadata with stable IDs
- Support rewind/resume as first-class CLI flows
- Preserve developer ergonomics (normal `git commit`, optional automation)

## 2. High-level architecture

### 2.1 Core model

Three layers are central:

- Session state (active working memory)
- Temporary checkpoints (full snapshot state during work)
- Committed checkpoints (durable metadata snapshots)

### 2.2 Storage surfaces

1. Session state files
- Location: `.git/entire-sessions/<session-id>.json`
- Shared via git common dir across worktrees
- Tracks phase, base commit, touched files, transcript offsets, checkpoint IDs, attribution accumulators

2. Temporary checkpoint storage
- Location: shadow branches
- Branch naming: `entire/<baseCommit[:7]>-<worktreeHash[:6]>`
- Stores full working tree + metadata overlay
- Used for fast rewind and pre-condensation staging

3. Committed checkpoint storage
- Branch: `entire/checkpoints/v1`
- Checkpoint directory: sharded by stable checkpoint ID
- Path: `<id[:2]>/<id[2:]>/`
- Contains root summary + one subdirectory per session in that checkpoint (`0/`, `1/`, ...)

### 2.3 Linking mechanism

The durable join between code commits and metadata is the commit trailer:

- `Entire-Checkpoint: <12-hex-id>`

This trailer is the key invariant used by:

- `resume`
- `rewind` (logs-only mode)
- `explain`
- orphan detection / cleanup
- audit and attribution tracing

## 3. CLI surface and command map

Root command (`entire`) is Cobra-based.

Visible user commands:

- `enable`
- `disable`
- `status`
- `rewind`
- `resume`
- `clean`
- `reset`
- `explain`
- `doctor`
- `version`

Internal/hidden commands:

- `hooks` (git + agent hook entrypoints)
- `__send_analytics` (detached telemetry sender)
- `curl-bash-post-install` (post-install shell completion)
- `debug` (developer support)

Behavioral details:

- Unknown command/flag paths print usage + suggestion via main wrapper
- `SilentError` is used where command already printed an end-user-safe message
- Post-command pipeline may run telemetry and version check

## 4. Settings and effective configuration

Settings load model:

- Base: `.entire/settings.json`
- Override: `.entire/settings.local.json`
- Local file is merged over project file
- Unknown keys are rejected (strict decoder)

Important fields:

- `strategy` (`manual-commit` default, `auto-commit` optional)
- `enabled`
- `local_dev`
- `log_level`
- `strategy_options` (e.g. `push_sessions`, `summarize.enabled`)
- `telemetry`

Defaults:

- strategy defaults to `manual-commit`
- enabled defaults to `true` when no file exists

## 5. End-to-end user flow

### 5.1 Onboarding (`entire enable`)

Flow:

1. Validate git repository
2. Detect/select one or more agents
3. Create `.entire/` and ensure `.entire/.gitignore`
4. Write settings file (project or local target)
5. Install agent hooks
6. Install generic git hooks
7. Ensure strategy setup (notably metadata branch)
8. Optional telemetry consent (interactive path)

Key specifics:

- Interactive auto-detect can multi-select agents
- Non-interactive mode is `--agent ...`
- Supports `--strategy manual-commit|auto-commit`
- `--skip-push-sessions` writes `strategy_options.push_sessions=false`
- Hook manager detection warns about Husky/Lefthook/pre-commit/Overcommit behavior

### 5.2 Agent lifecycle (normal prompt)

Canonical lifecycle:

1. Turn start hook (UserPromptSubmit / BeforeAgent / TurnStart)
- Capture pre-prompt untracked baseline
- Capture transcript position offset
- Initialize/update session state

2. Agent performs edits

3. Turn end hook (Stop / AfterAgent / TurnEnd)
- Read transcript
- Extract prompts/summary/files
- Detect new/deleted files via git status diff vs pre-state
- Build step context and call strategy `SaveStep`
- Transition phase and dispatch turn-end handler

### 5.3 Manual commit flow (default)

- Session work first goes to shadow branch checkpoints
- User does `git commit`
- `prepare-commit-msg` decides whether to inject `Entire-Checkpoint`
- `post-commit` condenses matching session(s) into `entire/checkpoints/v1`
- Carry-forward keeps uncommitted overlap files for next split commit

### 5.4 Auto commit flow

- On turn end, strategy commits code directly to active branch with trailer
- Then writes checkpoint metadata to metadata branch
- If commit is empty, metadata write is skipped (prevents orphan metadata)

### 5.5 Resume flow

- `entire resume <branch>` checks out/switches branch
- Finds latest branch-relevant commit containing checkpoint trailer
- Fetches metadata branch from remote if needed
- Restores one or many session logs (multi-session supported)
- Emits per-session resume commands (agent-specific)

### 5.6 Rewind flow

- Lists both temporary (full rewind) and committed (logs-only) points
- For logs-only points user can:
  - restore logs only
  - detached checkout
  - hard reset branch (with warnings and recovery hint)
- Transcript restoration prefers:
  1. committed checkpoint store
  2. shadow commit tree
  3. local transcript fallback

## 6. Session state machine and lifecycle rigor

Session phases:

- `idle`
- `active`
- `ended`

Events:

- `TurnStart`
- `TurnEnd`
- `GitCommit`
- `SessionStart`
- `SessionStop`
- `Compaction`

Actions emitted by transition logic include:

- condense
- condense-if-files-touched
- discard-if-no-files
- warn-stale-session
- clear-ended-at
- update-last-interaction

Important behaviors:

- `active_committed` is a legacy phase mapped to `active`
- `Compaction` can condense while staying active and reset transcript offset
- Common bookkeeping actions always run even if strategy handler action fails

## 7. Manual-commit strategy deep dive

### 7.1 Why this is the default

It preserves native git commit flow while capturing rich session data and supports split-commit workflows with carry-forward semantics.

### 7.2 Session initialization behavior

On prompt start:

- loads existing session state or creates new one
- generates new per-turn `TurnID`
- computes pending prompt attribution before agent writes anything
- migrates shadow branch when HEAD moved mid-session
- clears old turn checkpoint IDs for fresh turn

### 7.3 `PrepareCommitMsg` behavior by source

The hook handles source-specific behavior:

- skip for `merge`, `squash`
- amend (`source=commit`) preserves/restores prior checkpoint trailer via `LastCheckpointID`
- agent commit fast path when no TTY and active session: add trailer directly
- message mode (`-m`, `-F`) can prompt user on TTY for linkage consent
- normal editor mode inserts trailer plus explanatory comment block

### 7.4 `CommitMsg` safety behavior

If commit message contains only Entire trailer and no user content, trailer is stripped so git can abort empty commit naturally.

### 7.5 `PostCommit` condensation behavior

Core loop per session on the same worktree:

- parse checkpoint trailer from current commit
- determine session relevance (`hasNew`) with fail-open behavior for resilience
- run state-machine `GitCommit` transition with per-session action handler
- condense when appropriate
- record checkpoint IDs for deferred finalization when still active
- compute remaining files with content-aware carry-forward
- preserve/delete shadow branches safely with multi-session awareness

### 7.6 Content-aware overlap logic

The strategy avoids false linkage when user reverted/replaced content:

- modified tracked files count as overlap
- new files require content/hash overlap checks
- partial staging (`git add -p`) is handled to keep remaining agent changes

This prevents stale sessions from being condensed into unrelated commits.

### 7.7 Deferred finalization (critical behavior)

Mid-turn commits can happen before full transcript is complete.

Process:

- post-commit writes provisional checkpoint content
- session state records turn checkpoint IDs
- turn end (`HandleTurnEnd`) updates committed entries with full transcript/prompts/context
- checkpoint IDs are then cleared

This guarantees 1:1 commit linkage without losing full turn context.

### 7.8 Carry-forward after split commits

When first commit includes only subset of touched files:

- condensed checkpoint gets only committed overlap
- remaining files are written to a new carry-forward shadow checkpoint
- next commit gets distinct checkpoint ID

Enables clean split-commit history while preserving session traceability.

### 7.9 Attribution model

Manual strategy computes `Entire-Attribution` using accumulated prompt-level snapshots:

- prompt-start capture of user edits before agent run
- final commit-time calculation combines:
  - accumulated user edits
  - post-checkpoint user edits
  - agent deltas from base/shadow/head trees

Uses per-file pool heuristic to reduce mis-attribution during user self-edits.

## 8. Auto-commit strategy deep dive

### 8.1 Save path

`SaveStep` does:

1. Generate checkpoint ID
2. Commit code to active branch with trailer
3. Write committed checkpoint metadata to `entire/checkpoints/v1`

Empty commit path:

- code commit skipped and metadata not written

### 8.2 Rewind model

- rewind points are derived from commit trailers + metadata lookup
- `Rewind` performs protected hard reset
- logs-only determination compares ancestry against main/default branch reachability

### 8.3 Task checkpoint support

Auto strategy supports task checkpoints with incremental/final variants and stores task metadata under checkpoint tree task paths.

### 8.4 Orphan checkpoint cleanup

Orphans are auto-commit metadata entries with no referencing trailer on non-`entire/*` branches (e.g. after rebase/squash).

## 9. Checkpoint store internals

## 9.1 Temporary storage details

`WriteTemporary`:

- validates session/base IDs
- creates/uses shadow branch
- first checkpoint can capture all changed files via `git status --porcelain -z -uall`
- subsequent checkpoints use explicit modified/new/deleted lists
- builds tree with metadata overlay
- deduplicates by tree hash (skips identical checkpoint)

Safety specifics:

- metadata directory copy has symlink and traversal protections
- paths are repo-root normalized

### 9.2 Committed storage details

`WriteCommitted`:

- ensures metadata branch
- writes root summary + session subdirectory files
- supports multi-session checkpoints (stable index allocation per session ID)
- stores transcript/prompt/context/content hash/session metadata
- supports task and incremental task files
- writes optional export payload (e.g., OpenCode)

### 9.3 Transcript chunking

Transcripts can be chunked by agent-aware logic:

- chunk files are reassembled on read
- content hash file tracks integrity

### 9.4 Update semantics

`UpdateCommitted` uses replacement semantics (not append) for transcript/prompts/context and is used by deferred finalization.

### 9.5 Lookup APIs used by commands

- `ReadCommitted`, `ReadSessionContent`, `ReadSessionContentByID`, `ReadLatestSessionContent`
- `LookupSessionLog` convenience path for resume/rewind
- `GetCheckpointAuthor` for explain view

## 10. Agent abstraction and integrations

Agent interface separates:

- identity/detection
- hook event mapping
- transcript read/chunk/reassemble
- optional capabilities (hook install, transcript analyzer, token calculator, subagent-aware extraction)

### 10.1 Claude Code integration

Key characteristics:

- hooks installed in `.claude/settings.json`
- lifecycle hooks include session/turn + task hooks
- supports transcript flush sentinel wait before reading stop transcript
- supports subagent-aware file and token extraction
- adds permission deny rule for `.entire/metadata/**` reads

### 10.2 Gemini CLI integration

Key characteristics:

- hooks installed in `.gemini/settings.json`
- richer hook set including model/tool notifications, compression
- transcript model is JSON message array (offset is message index)
- lifecycle maps before-agent/after-agent to turn boundaries

### 10.3 OpenCode integration

Key characteristics:

- plugin-based hook integration (`.opencode/plugins/entire.ts`)
- JSONL transcript parsing for tool edits
- supports export/import restoration path into OpenCode native storage (`opencode import`)
- resume/rewind can restore file + export-backed native session state

## 11. Command deep dive

### 11.1 `status`

- shows effective merged status
- optional `--detailed` prints per-file settings sources
- groups active sessions by worktree path and branch
- shows truncated first prompt and age/activity timestamps

### 11.2 `resume`

- branch-aware checkpoint discovery (feature-vs-default logic)
- handles missing local branch with remote fetch prompt
- warns when newer non-checkpoint commits exist
- auto-fetches metadata branch if checkpoint exists only remotely
- multi-session logs restore with per-session commands

### 11.3 `rewind`

Interactive and non-interactive modes:

- supports `--list`, `--to`, `--logs-only`, `--reset`
- temporary checkpoints support full file rewind
- logs-only points support restore/checkout/reset paths
- hard reset path has explicit safety checks and undo guidance
- task checkpoint restoration truncates transcript at checkpoint UUID

### 11.4 `explain`

Modes:

- branch list (default)
- commit explain (`--commit`)
- checkpoint explain (`--checkpoint`)

Output levels:

- default detailed
- `--short`
- `--full`
- `--raw-transcript`

Other specifics:

- can generate and persist AI summary (`--generate`, `--force`)
- associated commits found by scanning trailers
- branch/default-branch traversal uses first-parent filters to avoid noise
- handles committed IDs and temporary SHA-prefix checkpoints

### 11.5 `doctor`

Identifies stuck sessions:

- active but stale by interaction threshold
- ended with uncondensed checkpoint data

Actions:

- condense if possible
- discard state/branch if safe
- skip
- `--force` auto-fix path

### 11.6 `clean`

Default dry-run; `--force` deletes.

Detects and reports:

- orphan shadow branches
- orphan session states
- orphan checkpoint metadata (auto-commit only)

Invariant:

- never deletes `entire/checkpoints/v1` branch itself

### 11.7 `reset`

- strategy capability-gated (manual-commit supports)
- bulk reset for sessions on current HEAD or single `--session`
- active session guard unless `--force`
- preserves working-directory file content

### 11.8 `disable` and uninstall

- standard disable writes `enabled=false`
- `--uninstall` removes hooks, `.entire`, states, and shadow branches
- agent hooks removed for Claude/Gemini/OpenCode if present

## 12. Git hook framework and operational safety

Entire-managed git hooks:

- `prepare-commit-msg`
- `commit-msg`
- `post-commit`
- `pre-push`

Install model:

- if existing non-Entire hooks found, backups are created as `<hook>.pre-entire`
- generated hooks can chain to backup hooks
- uninstall restores backups when safe

Branch and worktree correctness:

- hooks path resolution uses git plumbing (`git rev-parse --git-path hooks`)
- session state is stored in git common dir, making worktree-aware behavior explicit

## 13. Logging and telemetry

Logging:

- structured `slog` JSON logs to `.entire/logs/entire.log`
- configurable via `ENTIRE_LOG_LEVEL` or settings
- hook context includes component and available identifiers (session/tool)

Telemetry:

- command-level detached analytics sender (when opted in)
- respects telemetry setting and opt-out environment variable

## 14. Security and privacy model

### 14.1 Important distinction

- Shadow branches can hold transient unredacted session data
- Committed metadata branch writes include redaction safety-net behavior

### 14.2 Redaction

- redaction is applied before committed storage writes
- JSONL-aware redaction for transcript-like content when possible
- fallback string redaction paths exist

### 14.3 File traversal safety

Metadata copy paths include explicit protections:

- skip symlinks
- prevent `..` traversal entries
- only include intended directory content

## 15. Known limitations (documented in repo)

1. Amend with `-m` edge cases can lose link in specific no-TTY/new-content paths (mostly mitigated by `LastCheckpointID` restoration).
2. Worktree + `git gc --auto` can corrupt index cache-tree in some scenarios; operational mitigation is disabling auto-GC.
3. Concurrent ACTIVE sessions in same directory can create spurious minimal checkpoints on one session’s commit.

## 16. Subtle implementation details that matter

1. Worktree-sensitive shadow branch naming
- Shadow branches include worktree hash to avoid cross-worktree collisions.

2. Legacy compatibility
- old phase values and transcript offset fields are normalized on state load.

3. Content-overlap fail-open strategy
- On uncertainty/errors, code usually avoids data loss even at cost of occasional extra metadata entries.

4. Multi-session checkpoint indexing
- session slot directories are index-based and stable per session ID; not strictly chronological folder semantics.

5. Deferred-finalization checkpoint IDs
- IDs condensed mid-turn are explicitly recorded and later finalized with full transcript context.

6. Metadata branch push synchronization
- pre-push can auto-push sessions branch and handle non-fast-forward by fetch+merge.

## 17. Test coverage signals (from integration/e2e suite)

The reference includes broad scenario tests for:

- setup/enable/disable/uninstall
- manual and auto strategies
- hook install/wiring
- deferred finalization
- carry-forward overlap behavior
- mid-session commit/rebase/migration
- attribution behavior
- resume and logs-only rewind
- subagent checkpoint flows
- worktree behavior

This test density indicates high confidence around the nuanced lifecycle behavior rather than only happy paths.

## 18. Overall architecture assessment

The Go CLI is architected around two strong invariants:

1. Durable linkage invariant
- Every relevant code commit can be linked to session metadata via a stable checkpoint trailer.

2. Recoverability invariant
- Session state is recoverable through shadow branches (temporary/full) and metadata branch (committed/logs-only), with explicit resume/rewind tooling.

The implementation is intentionally conservative about data loss:

- best-effort hooks with fail-open semantics
- explicit safety prompts for destructive actions
- compatibility shims for older data formats
- content-aware overlap/carry-forward to preserve correct 1:1 commit semantics

In practical terms: this is not just a hook bundle, it is a full lifecycle engine coordinating agent events, git state, metadata persistence, and developer UX across real-world branch/worktree/session complexity.

## 19. Concrete user journey summary

A typical, fully-featured journey is:

1. Developer runs `entire enable` and selects agent(s)
2. Agent hooks emit lifecycle events into Entire dispatcher
3. Entire captures pre-state at turn start
4. Turn end writes checkpoint data through strategy
5. User commit gets `Entire-Checkpoint` trailer injected when session-related
6. Post-commit condenses to `entire/checkpoints/v1` (manual strategy) or is already committed (auto strategy)
7. If split commits happen, carry-forward preserves remaining session files
8. `entire explain` later reconstructs intent/outcome/commits/transcript scope
9. `entire resume <branch>` restores logs and exact resume command(s)
10. `entire rewind` restores files and/or logs depending on checkpoint type
11. `entire clean` and `entire doctor` maintain health of long-lived repos

That flow is the core reason this CLI solves the original problem definition: session context stops being ephemeral and becomes queryable, recoverable, and tied to git history in a machine-readable way.

## 20. Additional implementation specifics (validated from source)

### 20.1 Entrypoint and error routing

- `cmd/entire/main.go` owns final error handling; root command uses `SilenceErrors=true`, `SilenceUsage=true` to avoid duplicate output.
- Unknown command/flag errors trigger brew-style usage + `Error: Invalid usage: ...`.
- `SilentError` suppresses duplicate printing when command logic already emitted user-facing messaging.
- `root.go` runs telemetry + version-check in `PersistentPostRun`, but explicitly skips hidden commands by walking command parents.

### 20.2 Checkpoint ID and path invariants

- Checkpoint IDs are generated via `crypto/rand` (6 random bytes -> 12 lowercase hex chars).
- IDs are regex-validated as exactly `[0-9a-f]{12}`.
- Sharded metadata path is computed deterministically by ID helper (`<id[:2]>/<id[2:]>`), used consistently across read/write/list/update flows.

### 20.3 Settings merge semantics (strict, field-aware)

- Both `.entire/settings.json` and `.entire/settings.local.json` are decoded with `DisallowUnknownFields` (unknown keys fail fast).
- Local overrides are presence-based:
  - booleans override even when `false`
  - strings override only when non-empty
  - `strategy_options` merges by key (not full replacement)
- Defaults are reapplied post-merge (`strategy=manual-commit`, `enabled=true` baseline behavior).

### 20.4 Lifecycle no-op and fail-open safeguards

- Turn-end exits early on empty repositories (`Entire: skipping checkpoint. Will activate after first commit.`).
- Transcript/export copy is best-effort where possible (warnings on partial failures, hard errors only for critical write paths).
- No-change turns explicitly skip checkpoint creation after file-diff detection.

### 20.5 Manual strategy commit hook edge behavior

- `prepare-commit-msg` intentionally skips git sequence operations (rebase/cherry-pick/revert) and auto-generated sources (`merge`, `squash`).
- Non-TTY + ACTIVE session path adds trailer directly to support agent-initiated commits (no interactive prompt possible).
- `post-commit` path without trailer still updates `BaseCommit` for ACTIVE sessions so future commit matching remains correct.
- `commit-msg` removes trailer-only messages so git can naturally abort empty commits.

### 20.6 Deferred finalization data integrity

- Mid-turn condensed checkpoint IDs are stored in `TurnCheckpointIDs` and finalized at stop.
- Finalization updates committed checkpoints with full transcript/prompts/context via `UpdateCommitted` replace semantics.
- Finalization redacts transcript/prompt/context content before writing to metadata branch, matching committed-write privacy guarantees.

### 20.7 Resume and rewind nuance

- `resume` checks branch-local checkpoint ancestry and warns if newer branch-only commits lack checkpoints.
- Resume includes overwrite protection when local logs are newer than checkpoint logs (unless forced).
- `rewind --to` supports `--logs-only` and `--reset` for logs-only checkpoints; interactive mode offers restore-logs / detached checkout / destructive reset choices for those points.

### 20.8 Uninstall ordering and safety

`disable --uninstall` executes removal in a deliberate order:

1. agent hooks
2. git hooks
3. session state files (`.git/entire-sessions`)
4. `.entire/` directory
5. shadow branches

This ordering minimizes risk of leaving active hook entrypoints after metadata/state removal.
