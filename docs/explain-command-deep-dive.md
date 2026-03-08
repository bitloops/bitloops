# `explain` Command Deep-Dive Report

**Go reference:** `golang-reference/cmd/entire/cli/explain.go` (1592 lines)
**Rust implementation:** `bitloops_cli/src/commands/explain.rs` (~2183 lines)
**Generated:** 2026-02-27

---

## 1. Purpose and scope

The `explain` command gives developers visibility into AI-assisted work that has been
persisted in the checkpoint store. It answers three questions:

1. *What checkpoints exist on the current branch?* (default mode — branch list)
2. *What happened during the session that produced this commit?* (`--commit <sha>`)
3. *What are the full details of a specific checkpoint?* (`--checkpoint <id>`)

---

## 2. Types

### `interaction`
Single prompt + one or more assistant responses for display. Fields: `Prompt string`,
`Responses []string`, `Files []string`. Multiple responses arise when the agent makes
tool calls between text responses within a single user turn.

### `associatedCommit`
A git commit that references a checkpoint via the `Entire-Checkpoint` (or `Bitloops-Checkpoint`)
commit trailer. Fields: `SHA`, `ShortSHA`, `Message` (first line), `Author`, `Date`.

### `checkpointDetail`
Structured representation of a single checkpoint for display. Fields: `Index int`,
`ShortID string`, `Timestamp time.Time`, `IsTaskCheckpoint bool`, `Message string`,
`Interactions []interaction`, `Files []string` (aggregate across all interactions).

### `checkpointGroup`
Groups multiple rewind points (commits) that share the same checkpoint ID. Fields:
`checkpointID string`, `prompt string`, `isTemporary bool`, `isTask bool`,
`commits []commitEntry`.

### `commitEntry`
A single git commit within a checkpoint group. Fields: `date time.Time`, `gitSHA string`
(7-char abbreviated), `message string`.

---

## 3. Command definition (`newExplainCmd`)

Flags:

| Flag | Short | Mutual exclusions | Requires |
|------|-------|-------------------|----------|
| `--session <id>` | | cannot combine with `--commit` or `--checkpoint` | |
| `--commit <ref>` | | cannot combine with `--session` or `--checkpoint` | |
| `--checkpoint <id>` | `-c` | cannot combine with `--session` or `--commit` | |
| `--no-pager` | | | |
| `--short` | `-s` | `--full`, `--raw-transcript` | |
| `--full` | | `--short`, `--raw-transcript` | |
| `--raw-transcript` | | `--short`, `--full`, `--generate` | `--checkpoint` |
| `--generate` | | `--raw-transcript` | `--checkpoint` |
| `--force` | | | `--generate` |
| `--search-all` | | | |

Custom argument validator: positional arguments are rejected with a hint to use
`--checkpoint`, `--session`, or `--commit`.

---

## 4. Routing (`runExplain`)

Counts mutually exclusive flags; validates that `--session` cannot combine with
`--commit`/`--checkpoint`; validates `--commit` and `--checkpoint` are mutually exclusive.

Routes to:
- `runExplainCommit()` if `--commit` was given
- `runExplainCheckpoint()` if `--checkpoint` was given
- `runExplainBranchWithFilter(sessionFilter)` otherwise (session filter may be empty)

`runExplainBranchDefault()` is a convenience wrapper for `runExplainBranchWithFilter("")`.

---

## 5. Branch-list mode (`getBranchCheckpoints` / `runExplainBranchWithFilter`)

This is the default view — no flags required.

### 5.1 Execution path

```
runExplainBranchWithFilter
  -> getRepoRoot
  -> getCurrentBranchName  (handles detached HEAD, unborn HEAD)
  -> getBranchCheckpoints(repo, limit=unlimited)
       -> open checkpoint store
       -> list ALL committed checkpoints (builds ID -> info map)
       -> compute reachability from HEAD (see §5.2 / §5.3)
       -> collectCheckpoint() callback per commit
       -> getReachableTemporaryCheckpoints()
       -> sort by date DESC
  -> formatBranchCheckpoints(branchName, points, sessionFilter)
  -> outputWithPager()
```

### 5.2 Default branch traversal

On `main` / `master`:

- Uses **full DAG walk** via `repo.Log()` (the go-git library). This follows ALL parents,
  including second parents of merge commits, so it catches checkpoints on feature branches
  that have been merged into main.
- **No depth limit.** All reachable commits are traversed.
- For each commit, `collectCheckpoint()` extracts the `Entire-Checkpoint` trailer and looks
  up the corresponding metadata in the committed-checkpoints map.
- Best-effort reads the session prompt from the metadata branch for display.

### 5.3 Feature-branch traversal

On any non-default branch:

1. `computeReachableFromMain()` builds a `map[Hash]bool` of up to 1000 commits reachable
   from the first-parent chain of `main` / `master` (or `origin/main`, `origin/master`).
2. First-parent walk from HEAD with depth limit **500 commits**.
3. Stops (via `errStopIteration`) when the walker reaches a commit that is in the
   reachable-from-main set.
4. Only commits on the feature branch side are collected.

### 5.4 Temporary checkpoint collection (`getReachableTemporaryCheckpoints`)

After committed checkpoints are collected:

1. Computes current worktree hash via `paths.GetWorktreeID()` + `checkpoint.HashWorktreeID()`
   (6-char hex prefix of worktree ID hash).
2. Lists all `entire/<base7>-<wt6>` shadow branches via the checkpoint store.
3. Filters by worktree hash to exclude branches from other worktrees.
4. For each matching shadow branch:
   - `isShadowBranchReachable()` checks if the base commit is reachable from HEAD.
     - On default branch: always true.
     - On feature branch: first-parent scan of the chain to match base commit prefix.
   - Lists commits on the shadow branch via the store.
   - `convertTemporaryCheckpoint()` filters metadata-only commits via `hasCodeChanges()`
     (checks if any changed file is outside `.entire/`).
   - Best-effort reads session prompt from the shadow commit tree.
   - Returns `RewindPoint` with `IsLogsOnly=false` (temporary = fully rewindable).

### 5.5 Output formatting

`formatBranchCheckpoints`:
1. Prints `Branch: <name>`.
2. Applies session filter if provided (string prefix match on session ID).
3. If 0 checkpoints: prints "Checkpoints: 0" + help message.
4. Groups via `groupByCheckpointID()` (see §5.6).
5. Prints "Checkpoints: N".
6. Formats each group via `formatCheckpointGroup()`.

`groupByCheckpointID`:
- Committed checkpoints: grouped by `CheckpointID`.
- Temporary checkpoints: grouped by `SessionID` (preserves per-session prompts when multiple
  shadow commits share a session).
- Commits within each group sorted by date DESC.
- Groups sorted by their latest commit date DESC.

`formatCheckpointGroup` output per group:
```
[<checkpoint_id[:12]>] [Task]? [temporary]? "<prompt[:60]>"
  MM-DD HH:MM (git_sha) message[:80]
  ...
```
The `[temporary]` marker appears when any member commit is not logs-only.

### 5.6 Display constants

```go
maxIntentDisplayLength  = 80
maxMessageDisplayLength = 80
maxPromptDisplayLength  = 60
checkpointIDDisplayLength = 12
```

---

## 6. Commit explain mode (`runExplainCommit`)

1. `repo.ResolveRevision(commitRef)` — resolves SHA or ref.
2. `repo.CommitObject(hash)` — gets commit object.
3. Extracts `Entire-Checkpoint` trailer from message.
4. If no trailer: prints "No associated Entire checkpoint" message and returns nil (no error).
5. If trailer found: delegates to `runExplainCheckpoint(checkpointID)`.

---

## 7. Checkpoint detail mode (`runExplainCheckpoint`)

### 7.1 Committed checkpoint lookup

1. `store.ListAllCommittedCheckpoints()` → builds sorted list.
2. Prefix-matches `checkpointIDPrefix` against all IDs:
   - 0 matches → falls through to temporary checkpoint lookup (§7.2).
   - 1 match → uses the full ID.
   - Multiple matches → ambiguity error showing up to 5 examples.
3. `store.ReadCommitted(checkpointID)` → loads `CheckpointSummary` (root file).
4. `store.ReadLatestSessionContent(checkpointID)` → loads `SessionContent` (session-level files).
5. If `--generate`: calls `generateCheckpointSummary()` (§7.4), then reloads content.
6. If `--raw-transcript`: writes raw transcript bytes directly to stdout and returns.
7. Best-effort `store.GetCheckpointAuthor()` → author name/email from git log.
8. Best-effort `getAssociatedCommits()` (§7.3) → related commits.
9. `formatCheckpointOutput()` (§7.5) → builds display string.
10. `outputWithPager()` → optionally pipes through `$PAGER`.

### 7.2 Temporary checkpoint lookup (`explainTemporaryCheckpoint`)

When the committed lookup returns 0 matches:

1. `store.ListAllTemporaryCheckpoints()` → all shadow branches across ALL worktrees.
2. SHA prefix match (checking all checkpoint SHAs in all shadow branches).
   - 0 matches → "checkpoint not found" error.
   - Multiple matches → ambiguity error (SHA, timestamp, session ID).
3. Loads shadow commit and tree.
4. Reads agent type from tree metadata via `strategy.ReadAgentTypeFromTree()`.
5. If `--raw-transcript`: writes transcript to stdout and returns.
6. Reads session prompt from tree.
7. Formats output:
   ```
   Checkpoint: <sha[:7]> [temporary]
   Session: <session-id>
   Created: <timestamp>
   Intent: <first line of prompt>
   Outcome: (not generated)
   Transcript (checkpoint scope):  ...  OR  Transcript (full session): ...
   ```
8. Returns formatted string.

**This is the only part of the explain command not yet implemented in Rust.**

### 7.3 Associated commits (`getAssociatedCommits`)

Finds all git commits that reference a given checkpoint ID via trailer.

**`searchAll=false` (default):**
1. `computeReachableFromMain()` → set of main-branch commit hashes (limit 1000).
2. First-parent walk from HEAD, depth limit **500 commits**.
3. Stops when hitting a main-reachable commit.
4. Extracts `Entire-Checkpoint` from each commit via `trailers.ParseCheckpoint()`.
5. Returns matching commits.

**`searchAll=true`:**
1. Full DAG walk via `repo.Log()`.
2. **No depth limit.**
3. Returns all commits matching the checkpoint ID.

### 7.4 AI summary generation (`generateCheckpointSummary`)

1. Returns error if summary exists and `--force` is false.
2. Returns error if no transcript content.
3. `scopeTranscriptForCheckpoint(fullTranscript, startOffset, agentType)` → slices
   transcript to this checkpoint's portion only (agent-aware: Gemini uses JSON message-index,
   others use JSONL line count).
4. `summarize.GenerateFromTranscript(scoped, files, agentType)` → calls AI model.
5. `store.UpdateSummary(checkpointID, summary)` → persists back to checkpoint store.
6. Prints success message to stdout.

### 7.5 Checkpoint output formatting (`formatCheckpointOutput`)

Header (always):
```
Checkpoint: <id>
Session: <session-id>
Created: <timestamp>
Author: <name> <<email>>   # if available
Tokens: <input + output>
```

Associated commits section (if any):
```
Commits: (N)
  <sha7> <date> <message>
```

Intent/Outcome section:
- If AI summary available: shows `Intent:` and `Outcome:` from summary.
- Otherwise: extracts from scoped prompts first line, falls back to stored prompts.
  Truncates intent to 80 chars. Outcome is "(not generated)".

If `verbose` or `full`:
- Summary details (learnings, friction, open items) from AI summary if available.
- Files section:
  ```
  Files: (N)
    - path/to/file
  ```

Transcript section (via `appendTranscriptSection`):
- `--full`: "Transcript (full session):" with entire session transcript.
- Default (`verbose=true`): "Transcript (checkpoint scope):" with scoped transcript.
- `--short`: no transcript.

Transcript formatted via `formatTranscriptBytes` (parses JSONL, prints `[User] prompt`
and `[Assistant] response` lines).

---

## 8. Transcript scoping by agent type

`scopeTranscriptForCheckpoint` branches on agent type:

| Agent | Mechanism | Note |
|-------|-----------|------|
| Gemini CLI | `geminicli.SliceFromMessage(bytes, messageIndex)` | JSON message array; offset is message count |
| Claude Code | `transcript.SliceFromLine(bytes, lineOffset)` | JSONL; offset is line count |
| OpenCode | `transcript.SliceFromLine(bytes, lineOffset)` | same as Claude Code |
| Unknown | `transcript.SliceFromLine(bytes, lineOffset)` | default path |

`transcriptOffset` returns the appropriate unit for each agent type (message count for
Gemini, line count otherwise).

---

## 9. Pager behavior (`outputWithPager`)

1. Checks if output is stdout AND stdout is a terminal (`golang.org/x/term`).
2. Gets terminal height (default 24 rows).
3. Counts newlines in content.
4. If `lines > termHeight - 2`: spawns `$PAGER` (default `"less"`) and pipes content.
   Falls back to direct output if pager launch fails.
5. Otherwise: outputs directly.

---

## 10. Test coverage in `explain_test.go`

| Test | What it exercises |
|------|-------------------|
| `TestNewExplainCmd` | Correct flags registered |
| `TestExplainCmd_SearchAllFlag` | `--search-all` default |
| `TestExplainCmd_RejectsPositionalArgs` | Positional arg rejection + hint message |
| `TestExplainCommit_NotFound` | Non-existent commit → error |
| `TestExplainCommit_NoEntireData` | Commit without trailer → "No associated" message (not error) |
| `TestExplainCommit_WithMetadataTrailerButNoCheckpoint` | Metadata trailer without checkpoint trailer → "no checkpoint" |
| `TestExplainDefault_ShowsBranchView` | Default mode → "Branch:" and "Checkpoints:" in output |
| `TestExplainDefault_NoCheckpoints_ShowsHelpfulMessage` | Empty repo → "Checkpoints: 0" + help text |
| `TestExplainBothFlagsError` | `--session` + `--commit` → error |
| `TestFormatSessionInfo` | Session header, checkpoint details, prompts, files |
| `TestFormatSessionInfo_WithSourceRef` | Source ref included when provided |
| `TestStrategySessionSourceInterface` | `ManualCommitStrategy` implements `SessionSource` |
| `TestFormatSessionInfo_CheckpointNumberingReversed` | Oldest checkpoint = 1, newest = N |
| `TestFormatSessionInfo_EmptyCheckpoints` | "Checkpoints: 0" |
| `TestFormatSessionInfo_CheckpointWithTaskMarker` | `[Task]` marker |
| `TestFormatSessionInfo_CheckpointWithDate` | Full date in checkpoint header |

---

## 11. Rust implementation status

### 11.1 What is at full parity

| Feature | Rust location | Notes |
|---------|--------------|-------|
| Three-mode routing | `run_explain()` | |
| All flags + mutual exclusions | `ExplainArgs`, `new_explain_command()` | |
| Positional arg rejection | `validate_no_positional_args()` | |
| Branch-list mode (real) | `run_explain_branch_with_filter()` → `get_branch_checkpoints_real()` | Full DAG walk on default branch; first-parent + main-filter on feature branches |
| Shadow branch enumeration | `get_reachable_temporary_checkpoints_shell()` | Worktree filtering, reachability check, `has_code_changes` filter |
| Committed checkpoint display | `run_explain_checkpoint_in()` | Prefix match, ambiguity error, author lookup |
| Commit explain mode | `run_explain_commit_in()` | Resolves ref, extracts trailer, delegates to checkpoint mode |
| `--raw-transcript` | `run_explain_checkpoint_in()` | Writes bytes to stdout |
| `--short` / `--full` / default verbosity | `format_checkpoint_output()` | |
| `--search-all` mode | `get_associated_commits()` | |
| AI summary generation (`--generate` / `--force`) | `generate_checkpoint_summary()` | Wired 2026-02-27; scopes transcript, calls AI, persists result |
| Pager support | `output_explain_content()` | `$PAGER` env var, terminal height detection |
| Agent type read from metadata | `metadata_from_json()` → `meta.agent_type` | Not hardcoded |
| Gemini transcript scoping | `summarize::scope_transcript_for_checkpoint()` | Branches on agent type: Gemini → message-index; others → line-count |
| Associated commits lookup | `get_associated_commits()` | 500-commit scan limit via `build_commit_graph_from_git()` |
| "No associated checkpoint" message | `run_explain_commit_in()` | |

### 11.2 Gap A — Temporary checkpoint explain is stubbed (significant)

**Go path:** `runExplainCheckpoint` → `explainTemporaryCheckpoint(shaPrefix)`:
1. Lists ALL shadow branches via `store.ListAllTemporaryCheckpoints()`.
2. SHA prefix match, ambiguity detection.
3. Reads agent type from shadow commit tree.
4. Reads session prompt and transcript from shadow commit tree.
5. Formats full output with header, intent, and scoped transcript.

**Rust path:** `explain_temporary_checkpoint_real()` calls `explain_temporary_checkpoint()`
which searches `MOCK_TEMPORARY_CHECKPOINTS` (a hardcoded constant with two fake SHAs).

Any `bitloops explain --checkpoint <sha>` targeting a real shadow-branch commit will return
"checkpoint not found" instead of reading the actual shadow commit.

**Fix:** Replace `explain_temporary_checkpoint_real()` with real shadow-branch enumeration.
The underlying infrastructure already exists in `get_reachable_temporary_checkpoints_shell()`.
The missing pieces are:
- SHA-prefix match across commits on all shadow branches (not just the current worktree's).
- Reading agent type from the shadow commit's tree metadata directory.
- Reading transcript bytes directly from the shadow commit's tree.
- Formatting with `[temporary]` header, session ID, intent, outcome, and transcript.

### 11.3 Gap B — 500-commit ceiling on unlimited Go paths (minor)

`build_commit_graph_from_git()` runs `git log --max-count 500 HEAD`. This affects two Go
paths that are *unlimited* in the reference:

| Go path | Go depth | Rust depth |
|---------|----------|------------|
| `getBranchCheckpoints` on default branch | Unlimited DAG | 500 commits |
| `getAssociatedCommits` with `searchAll=true` | Unlimited DAG | 500 commits |

In practice this only matters for repositories with more than 500 commits between the HEAD
and the earliest checkpoint of interest. The 500 limit matches the feature-branch depth
limit in Go, so default-branch behavior diverges for very large repos.

**Fix:** For `searchAll=true`, run a second `git log --format=...` without `--max-count`.
For the default-branch branch-list, also drop the limit (or increase it significantly).

---

## 12. Stale assumptions found in prior parity documentation

The following items were documented as open gaps but are already resolved in the current
Rust code:

### `explain-parity.md` Gap 1 — `--generate` stub
**Was:** `generate_checkpoint_summary()` bailed unconditionally.
**Now:** Fully wired (2026-02-27): scopes transcript, calls `summarize::generate_from_transcript`,
persists via `update_summary`.
→ **Document updated accordingly.**

### `explain-parity.md` Gap 3 — Agent type hardcoded as `ClaudeCode`
**Was:** `scope_transcript_for_checkpoint(...)` and `extract_prompts_from_transcript(...)`
called with `AgentType::ClaudeCode` hardcoded.
**Now:** `format_checkpoint_output()` reads `let agent_type = meta.agent_type;` (line 966)
which is populated via `metadata_from_json()` from the stored `"agent"` field.
→ **Document updated accordingly.**

### `explain-parity.md` Gap 4 — Gemini transcript scoping not differentiated
**Was:** All agent types fell through to `transcript::parse::slice_from_line()`.
**Now:** `scope_transcript_for_checkpoint` in `explain.rs` delegates to
`summarize::scope_transcript_for_checkpoint()` which explicitly branches: Gemini →
`geminicli::transcript::slice_from_message()`, others → `transcript::parse::slice_from_line()`.
→ **Document updated accordingly.**

### `module-parity-tracker.md` P0 explain row
**Was:** "Default branch wrapper wired; branch listing (list all checkpoints for a session)
and AI generate mode remain open."
**Now:** Branch listing is fully implemented via `get_branch_checkpoints_real()`. AI generate
mode is wired. Only Gap A (temporary checkpoint explain) remains open.
→ **Tracker updated accordingly.**

---

## 13. Recommended fix order

1. **Gap A (temporary checkpoint explain):** Replace the mock in
   `explain_temporary_checkpoint_real()` with real shadow-branch traversal. Reuse
   `get_reachable_temporary_checkpoints_shell()` to enumerate shadow branches, match
   by SHA prefix, then read metadata/transcript from the matched shadow commit's tree.
   Estimated scope: ~60-100 lines touching only `explain.rs`.

2. **Gap B (500-commit ceiling):** For `searchAll=true`, rebuild `build_commit_graph_from_git`
   without `--max-count` (or pass limit as parameter). For the default-branch branch-list,
   same change. Estimated scope: ~10-15 lines.
