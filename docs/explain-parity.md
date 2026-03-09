# `explain` Command Parity Report

**Reference:** `golang-reference/cmd/entire/cli/explain.go` (1592 lines)
**Rust implementation:** `bitloops_cli/src/commands/explain.rs` (~2300 lines)
**Last updated:** 2026-02-27

**Overall status: 100% parity.** All previously documented gaps are now resolved.

---

## What is at parity

- Branch-list mode (default): full real implementation including worktree filtering,
  shadow branch enumeration via `get_reachable_temporary_checkpoints_shell()`,
  main-reachable filtering, grouping/sorting, and all display
  constants (`MAX_INTENT_DISPLAY_LENGTH`, `COMMIT_SCAN_LIMIT`, etc.)
- Commit explain mode (`--commit`): resolves ref, extracts trailer, delegates to checkpoint
  detail view
- All flags defined, validated, and marked mutually-exclusive correctly
- `--raw-transcript` output (binary bytes to stdout)
- `--short` / `--full` / default verbosity levels
- `--search-all` toggle — unlimited DAG walk (Go parity: no depth cap when `searchAll=true`)
- Paging with `PAGER` env var and terminal-height detection
- Associated commits lookup — unlimited when `searchAll=true`, capped at 500 otherwise
- `has_code_changes` filtering of metadata-only shadow commits
- Committed checkpoint lookup with prefix matching and ambiguity handling (up to 5
  examples shown)
- Author lookup via `get_checkpoint_author`
- "No associated Bitloops checkpoint" message (intentional name adaptation from Go's
  "Entire" — see `docs/go-to-rust-adaptations.md`)
- **`--generate` / `--force`** — fully wired: scopes transcript, calls
  `summarize::generate_from_transcript`, persists via `update_summary`
- **Agent type read from metadata** — `meta.agent_type` is populated via
  `metadata_from_json()` from the stored `"agent"` field; not hardcoded
- **Gemini transcript scoping** — `scope_transcript_for_checkpoint` delegates to
  `summarize::scope_transcript_for_checkpoint()` which branches: Gemini →
  `geminicli::transcript::slice_from_message()` (JSON message-index), others →
  `transcript::parse::slice_from_line()` (JSONL line-count)
- **Temporary checkpoint explain** — `explain_temporary_checkpoint_real()` enumerates ALL
  shadow branches (no worktree/reachability filter), matches by SHA prefix, reads
  agent type / prompt / transcript from the shadow commit tree via `git show <sha>:<path>`,
  and formats output with `[temporary]` header, session ID, created timestamp, intent,
  and optional scoped/full transcript. Resolved 2026-02-27.
- **Unlimited DAG walk on default branch** — `build_commit_graph_from_git()` now accepts a
  `limit: usize` parameter (0 = unlimited). Default-branch branch-listing passes 0;
  feature branches pass `COMMIT_SCAN_LIMIT`. Resolved 2026-02-27.

---

## Previously documented gaps (all resolved)

### Former Gap 1 — `--generate` / `--force` stub
`generate_checkpoint_summary()` now calls `summarize::generate_from_transcript()` with
the scoped transcript and persists via `manual_commit::update_summary()`.
**Resolved 2026-02-27.**

### Former Gap 2 — Temporary checkpoint explain is a stub
`explain_temporary_checkpoint_real()` now enumerates all shadow branches (`bitloops/*`
and `entire/*`), matches the SHA prefix across all commits on all branches, reads
`metadata.json` / `prompt.txt` / `full.jsonl` from the matched shadow commit tree via
`git show <sha>:<path>`, and formats output matching Go's `explainTemporaryCheckpoint`.
Ambiguity is surfaced as an error showing up to 5 candidates with SHA, timestamp, session ID.
**Resolved 2026-02-27.**

### Former Gap 3 — Agent type hardcoded as `ClaudeCode`
`format_checkpoint_output()` now reads `let agent_type = meta.agent_type;`,
populated from the stored `"agent"` field in committed checkpoint metadata.
**Resolved — confirmed in current code.**

### Former Gap 4 — Gemini transcript scoping not differentiated
`scope_transcript_for_checkpoint` in `explain.rs` now delegates to
`summarize::scope_transcript_for_checkpoint()` which explicitly handles Gemini (message-index
slicing) and Claude Code / OpenCode (JSONL line-count slicing).
**Resolved — confirmed in current code.**

### Former Gap 5 — 500-commit ceiling on unlimited Go paths
`build_commit_graph_from_git()` now accepts `limit: usize` (0 = unlimited).
- `get_branch_checkpoints_real`: passes `0` on default branch, `COMMIT_SCAN_LIMIT` on feature branches
- `run_explain_checkpoint_in`: passes `0` when `opts.search_all`, `COMMIT_SCAN_LIMIT` otherwise
**Resolved 2026-02-27.**

---

## Dead code (test scaffolding, not parity gaps)

| Symbol | Note |
|--------|------|
| `format_session_info` | Used by tests only; not called from main flow |
| `compute_reachable_from_main` | Mock version operating on `MockRepository`; real reachability computed inside `get_reachable_temporary_checkpoints_shell` |
| `get_branch_checkpoints` | Mock version using `MockRepository`; real impl is `get_branch_checkpoints_real` |
| `get_reachable_temporary_checkpoints` | Mock version; real impl is `get_reachable_temporary_checkpoints_shell` |
| `is_shadow_branch_reachable` | Mock version |
| `convert_temporary_checkpoint` | Mock version |
| `walk_first_parent_commits` | Mock version using `CommitNode` map |
| `transcript_offset` | Utility; not called from production paths |
| `build_parity_matrix_stub` | Hardcoded placeholder, never called |
| `MOCK_COMMITTED_CHECKPOINTS` | Test scaffold constant |
