# Commit-First DevQL Ingestion

## Overview

DevQL historical ingestion is commit-first. The engine processes commits oldest-to-newest over the missing branch segment and records each commit's ingestion state in a `commit_ingest_ledger`. Checkpoint work is companion work triggered only when a processed commit has a real checkpoint mapping.

This replaces the former checkpoint-first model where manual commits not mapped to checkpoints were silently skipped.

---

## Architecture

### Ingestion ownership

- `devql sync` owns live/current state: `current_file_state`, `artefacts_current`, `artefact_edges_current`, `repo_sync_state`.
- Commit-history ingestion owns historical tables: `commits`, `file_state`, `artefacts`, `artefact_edges`.
- Checkpoint companion ingestion owns: `checkpoint_events`, `checkpoint_file_snapshots`.
- These ownership boundaries are enforced in code and schema. Historical ingest never writes to live/current tables.

### Idempotency

The `commit_ingest_ledger` is the source of truth for replay safety. Every processed commit gets a ledger row with independent `history_status` and `checkpoint_status` fields. A replayed commit range skips completed commits and retries partially completed ones based on ledger state.

### Branch watermarks

For SQLite deployments, per-branch ingestion progress is tracked via `sync_state` rows keyed by `HIST_BRANCH:<branch_name>`. When a commit is requested for ingestion, the engine resolves the missing segment as:

1. If the branch has a watermark that is an ancestor of `HEAD`, ingest `(watermark, HEAD]`.
2. If no watermark exists, find the nearest ancestor commit already in the ledger with `history_status = completed`.
3. If no ingested ancestor exists, ingest the full reachable branch segment ending at `HEAD`.
4. If the watermark is no longer an ancestor (rebase, history rewrite), fall back to nearest ingested ancestor.

Detached `HEAD` ingestion never updates branch watermarks.

---

## New Modules and Types

### `src/host/devql/ingestion/history.rs`

Central module for commit ingestion state management. Key functions:

| Function | Purpose |
|---|---|
| `select_missing_branch_commit_segment()` | Determines which commits need ingestion for the active branch |
| `load_commit_ingest_ledger_entry()` | Reads a commit's ledger row |
| `commit_is_fully_ingested()` | Returns true if both `history_status` and `checkpoint_status` are terminal |
| `mark_commit_history_completed()` | Writes `history_status = completed` to the ledger |
| `mark_commit_checkpoint_completed()` | Writes `checkpoint_status = completed` to the ledger |
| `mark_commit_ingest_failed()` | Records failure with error message in `last_error` |
| `checked_out_branch_name()` | Returns the current git branch name |
| `uses_local_ingest_watermarks()` | Returns `true` for SQLite, `false` for Postgres |
| `historical_branch_watermark_key()` | Constructs the `sync_state` key for a branch watermark |
| `nearest_reachable_completed_commit()` | Finds the nearest ancestor already fully ingested |
| `list_commit_range()` | Lists commits in a range via `git rev-list` |

**Status constants:**

```
history_status:     pending | completed | failed
checkpoint_status:  not_applicable | pending | completed | failed
```

### `src/host/devql/ingestion/types.rs`

New and updated types:

- **`IngestionCounters`**: tracks `commits_processed`, `checkpoint_companions_processed`, plus semantic/embedding counters.
- **`IngestionProgressPhase`**: `Initializing | Extracting | Persisting | Complete | Failed`.
- **`IngestionProgressUpdate`**: phase + commit stats + current commit/checkpoint IDs + full counter details.
- **`IngestedCheckpointNotification`**: signals when a checkpoint companion has been ingested, carries optional commit SHA.
- **`IngestionObserver` trait**: allows callers to receive progress and checkpoint notifications.
- **`CheckpointCommitInfo`**: maps a commit to its metadata (SHA, timestamp, author name/email, subject).

---

## Schema Changes

### New table: `commit_ingest_ledger` (SQLite and Postgres)

```sql
CREATE TABLE IF NOT EXISTS commit_ingest_ledger (
    repo_id          TEXT        NOT NULL,
    commit_sha       TEXT        NOT NULL,
    history_status   TEXT        NOT NULL,
    checkpoint_status TEXT       NOT NULL,
    checkpoint_id    TEXT,
    last_error       TEXT,
    updated_at       TEXT/TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repo_id, commit_sha)
);

CREATE INDEX IF NOT EXISTS commit_ingest_ledger_repo_idx
    ON commit_ingest_ledger (repo_id);
```

This table is the idempotency source of truth. All upserts use `ON CONFLICT (repo_id, commit_sha)` so that partial failures (crash after history rows written, checkpoint companion not yet done) can be retried from the correct partial state.

### New table: `sync_state` (SQLite only)

```sql
CREATE TABLE IF NOT EXISTS sync_state (
    repo_id     TEXT NOT NULL,
    state_key   TEXT NOT NULL,
    state_value TEXT NOT NULL,
    updated_at  TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, state_key)
);

CREATE INDEX IF NOT EXISTS sync_state_repo_idx
    ON sync_state (repo_id);
```

Stores arbitrary keyed state per repo. Branch ingestion watermarks are stored here with keys of the form `HIST_BRANCH:<branch_name>`. Only used by SQLite deployments; Postgres does not use local branch watermarks.

> **Note:** The original plan proposed a dedicated `branch_ingest_watermarks` table. The implementation instead reuses `sync_state` with prefixed keys to keep the schema simpler and consistent with existing baseline sync state patterns.

### Existing tables (unchanged)

Historical: `commits`, `file_state`, `artefacts`, `artefact_edges`  
Current/live: `current_file_state`, `artefacts_current`, `artefact_edges_current`, `repo_sync_state`  
Checkpoint companion: `checkpoint_events`, `checkpoint_file_snapshots`

### `src/host/devql/ingestion/baseline.rs` additions

- `load_sync_state_value()`: generic read from `sync_state`.
- `upsert_sync_state_value()`: generic write to `sync_state`.
- `upsert_commit_metadata_row()`: writes commit metadata (author, message, timestamp) from git to the `commits` table.

---

## Ingestion Flow

### `commands_ingest.rs` — main loop

1. Call `select_missing_branch_commit_segment()` to get the ordered list of commits to process.
2. For each commit (oldest to newest):
   a. Load ledger entry via `load_commit_ingest_ledger_entry()`.
   b. If fully ingested, skip and advance the branch watermark.
   c. Extract file states, artefacts, language symbols, and edges.
   d. Write historical rows to `file_state`, `artefacts`, `artefact_edges`.
   e. Call `mark_commit_history_completed()`.
   f. If the commit maps to a checkpoint, run checkpoint companion work:
      - Create `checkpoint_events` rows (idempotent).
      - Project `checkpoint_file_snapshots` rows (idempotent).
      - Call `mark_commit_checkpoint_completed()`.
   g. If the commit has no checkpoint mapping, set `checkpoint_status = not_applicable`.
   h. On any failure, call `mark_commit_ingest_failed()` with the error message and stop advancing the watermark past the failed commit.
3. After the loop, update the branch watermark to `HEAD` in `sync_state`.

---

## GraphQL Integration

### `src/graphql/types/ingestion.rs`

New GraphQL types exposed to subscribers:

- **`IngestionPhase`** enum: maps `IngestionProgressPhase` to the GraphQL schema.
- **`IngestionProgressEvent`**: real-time progress event containing:
  - Current phase
  - `commitsTotal` / `commitsProcessed`
  - `checkpointCompanionsProcessed`
  - `currentCommitSha` / `currentCheckpointId`
  - Artefact and event counters

`From<IngestionProgressUpdate>` bridges internal types to GraphQL.

### Removed from ingest mutation

- `maxCheckpoints` parameter (checkpoint-first loop removed).

### Renamed progress fields

| Old (checkpoint-first) | New (commit-first) |
|---|---|
| `checkpointsTotal` | `commitsTotal` |
| `checkpointsProcessed` | `commitsProcessed` |
| `currentCheckpointId` | `currentCommitSha` + optional `currentCheckpointId` |

---

## Hook Integration

### Post-commit (`strategy_impl/post_commit_refresh.rs`)

1. `run_devql_post_commit_refresh()`: calls `execute_ingest_with_observer()` to catch up historical ingestion for the active branch, then refreshes artefacts for committed files.
2. `run_devql_post_commit_checkpoint_projection_refresh()`: updates checkpoint projections for commits that map to a checkpoint.

### Post-merge (`strategy_impl/post_merge_refresh.rs`)

1. Lists all files changed since the previous `HEAD`.
2. Calls `execute_ingest_with_observer()` to catch up commits reachable in the merged `HEAD`.
3. Refreshes artefacts for all changed files.

### Post-checkout

No historical ingest runs. Live sync only.

All hook paths create a tokio runtime if not already in one and run ingestion in the background. They do not block the git operation.

---

## Test Coverage

### `src/host/devql/tests/devql_tests/commit_history.rs`

| Test | What it verifies |
|---|---|
| Branch watermark preference | When a branch watermark exists and is an ancestor, only commits after it are ingested |
| Ledger fallback | When no watermark exists, falls back to nearest completed commit in the ledger |
| Unmapped commits — historical only | `file_state` and `artefacts` are populated; live tables (`artefacts_current`, `current_file_state`, `repo_sync_state`) remain untouched; ledger shows `history_status = completed`; branch watermark is set |
| Mapped commits — checkpoint companion | Checkpoint file snapshots and events are created exactly once; ledger shows both statuses as `completed`; replay is idempotent |
| Internal max-commits cap | Internal `max_commits` parameter limits replay without exposing a public API flag |

---

## Deviations from Original Plan

| Plan item | Actual implementation |
|---|---|
| Separate `branch_ingest_watermarks` table | Used `sync_state` table with `HIST_BRANCH:<name>` keys instead |
| `last_source TEXT NOT NULL` in ledger | Field not present in the implemented schema |
| Daemon-owned schema lifecycle | Schema migration ownership move is deferred; this change focuses on commit-first ingestion and the ledger |

---

## Key Invariants

- Historical ingest never writes to `artefacts_current`, `artefact_edges_current`, `current_file_state`, or `repo_sync_state`.
- The `commit_ingest_ledger` is append-upsert only; rows are never deleted.
- Branch watermarks only advance when the commit at that position has `history_status = completed`.
- Checkpoint companion work is always attempted after history ingestion completes for the same commit, never before.
- All DDL is `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` — safe to run on daemon start or restart.
