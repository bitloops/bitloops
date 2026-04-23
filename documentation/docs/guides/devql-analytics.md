---
title: DevQL Analytics
---

# DevQL Analytics

`bitloops devql analytics sql` runs read-only SQL over DevQL analytics views. Use it when you need aggregate or operational answers that are easier to express as SQL than as graph traversal: sync freshness, current file-state coverage, agent activity, token usage, shell command usage, and cross-repository comparisons.

This is a local analysis surface over Bitloops data. It is separate from Bitloops product telemetry.

## When To Use It

Use DevQL analytics when you want to answer questions like:

- Which repositories have stale current-state sync data?
- Which files are currently indexed, skipped, or resolved to a particular language?
- Which agent sessions and turns touched a repository recently?
- Which shell commands were run most often during captured interactions?
- How much token usage is associated with recent turns, grouped by model or repository?
- How do these answers compare across all known repositories?

Use `bitloops devql query` instead when the question is primarily about typed code graph traversal, such as artefacts, dependencies, clones, tests, checkpoints, or knowledge linked to selected artefacts.

## Prerequisites

Run the command from inside a Bitloops-enabled Git repository.

For current file-state analytics, make sure the repository has been synced:

```bash
bitloops devql tasks enqueue --kind sync --status
```

For interaction analytics, Bitloops needs captured interaction events. Recent runtime interaction spool data is overlaid during analytics refresh, so in-flight or recently captured sessions can appear before they have been fully compacted into the event store.

## Basic Usage

The default scope is the current repository:

```bash
bitloops devql analytics sql \
  'SELECT path, resolved_language, effective_source, last_synced_at
   FROM analytics.current_file_state
   ORDER BY path
   LIMIT 20'
```

Emit JSON instead of the default table:

```bash
bitloops devql analytics sql \
  'SELECT repo_id, COUNT(*) AS file_count
   FROM analytics.current_file_state
   GROUP BY repo_id
   ORDER BY file_count DESC' \
  --json
```

Query every known repository in the current daemon scope:

```bash
bitloops devql analytics sql \
  'SELECT name, repo_id, COUNT(path) AS indexed_files
   FROM analytics.repositories r
   LEFT JOIN analytics.current_file_state f USING (repo_id)
   GROUP BY name, repo_id
   ORDER BY indexed_files DESC' \
  --all-repos
```

Select explicit repositories with one or more `--repo` flags. The selector can be a repo id, full identity, or unique repository name:

```bash
bitloops devql analytics sql \
  "SELECT repo_id, path, resolved_language
   FROM analytics.current_file_state
   WHERE resolved_language = 'rust'
   ORDER BY repo_id, path
   LIMIT 50" \
  --repo bitloops
```

## SQL Contract

The analytics command intentionally exposes a constrained SQL surface:

| Rule | Behaviour |
| --- | --- |
| Statement shape | Exactly one statement is accepted. It must start with `SELECT` or `WITH`. |
| Mutations | `INSERT`, `UPDATE`, `DELETE`, DDL, `COPY`, `ATTACH`, `LOAD`, `INSTALL`, `PRAGMA`, import/export, and procedure calls are rejected. |
| Engine | Queries execute in a request-scoped in-memory DuckDB connection. |
| Data access | External access and extension autoloading are disabled, so file readers and arbitrary extensions are not available. |
| Result limit | Results are capped at 5,000 rows. A truncation warning is included when the cap is hit. |
| Timeout | Query execution times out after 30 seconds. |
| Output | Table output is the default. `--json` returns columns, rows, row count, duration, selected repo ids, and warnings. |

The command refreshes the daemon-wide analytics DuckDB cache as needed before each query. The cache lives under the daemon config root at `stores/analytics/analytics.duckdb`, but user SQL only sees request-scoped `analytics.*` and `analytics_raw.*` objects.

## View Catalogue

Prefer the curated `analytics.*` views. The matching `analytics_raw.*` tables expose the request materialisation behind most curated views and are mainly useful for diagnostics.

| View | What It Contains |
| --- | --- |
| `analytics.repositories` | Known repositories in scope, including `repo_id`, `repo_root`, provider, organisation, name, identity, default branch, metadata, and creation time. |
| `analytics.repo_sync_state` | Current-state sync metadata such as active branch, head commit/tree, parser and extractor versions, exclusion fingerprint, last sync timing, status, and reason. |
| `analytics.current_file_state` | Current file inventory, classification, resolved language, contexts, framework metadata, content ids, effective source, parser/extractor versions, existence flags, and last sync time. |
| `analytics.interaction_sessions` | Captured agent or user interaction sessions, including actor details, agent type, model, first prompt, transcript/worktree paths, and session timing. |
| `analytics.interaction_turns` | Per-turn prompts, model metadata, token counters, API call counts, summaries, transcript offsets/fragments, modified files, checkpoint id, and timing. |
| `analytics.interaction_events` | Raw interaction events with event type, source, sequence number, tool/subagent identifiers, task description, model metadata, and JSON payload. |
| `analytics.interaction_tool_invocations` | Derived tool invocation spans built from tool invocation/result events, including input/output summaries, shell command fields, transcript path, and start/end sequence numbers. |
| `analytics.interaction_subagent_runs` | Derived subagent spans built from subagent start/end events, including task description, child session id, transcript path, and start/end sequence numbers. |
| `analytics.shell_commands` | A filtered view of tool invocations that have a non-empty `command_binary`. |

Most repository-scoped views include `repo_id` and `repo_root`, which makes cross-repository grouping and joins straightforward.

Inspect exact columns with an empty-result JSON query:

```bash
bitloops devql analytics sql \
  'SELECT *
   FROM analytics.interaction_turns
   LIMIT 0' \
  --json
```

## Example Queries

### Current Sync Freshness

```bash
bitloops devql analytics sql \
  "SELECT
     r.name,
     s.active_branch,
     s.head_commit_sha,
     s.last_sync_completed_at,
     s.last_sync_status
   FROM analytics.repositories r
   LEFT JOIN analytics.repo_sync_state s USING (repo_id)
   ORDER BY COALESCE(s.last_sync_completed_at, '') DESC" \
  --all-repos
```

### File Coverage By Language

```bash
bitloops devql analytics sql \
  "SELECT
     COALESCE(resolved_language, language, 'unknown') AS language,
     COUNT(*) AS files,
     SUM(CASE WHEN exists_in_worktree = 1 THEN 1 ELSE 0 END) AS in_worktree
   FROM analytics.current_file_state
   GROUP BY 1
   ORDER BY files DESC"
```

### Recent Sessions

```bash
bitloops devql analytics sql \
  'SELECT
     session_id,
     branch,
     actor_name,
     agent_type,
     model,
     first_prompt,
     started_at,
     last_event_at
   FROM analytics.interaction_sessions
   ORDER BY COALESCE(last_event_at, ended_at, started_at) DESC
   LIMIT 10'
```

### Token Usage By Model

```bash
bitloops devql analytics sql \
  'SELECT
     model,
     COUNT(*) AS turns,
     SUM(input_tokens) AS input_tokens,
     SUM(cache_creation_tokens) AS cache_creation_tokens,
     SUM(cache_read_tokens) AS cache_read_tokens,
     SUM(output_tokens) AS output_tokens
   FROM analytics.interaction_turns
   WHERE has_token_usage = 1
   GROUP BY model
   ORDER BY output_tokens DESC'
```

### Shell Command Usage

```bash
bitloops devql analytics sql \
  'SELECT
     command_binary,
     COUNT(*) AS invocations,
     MIN(started_at) AS first_seen,
     MAX(ended_at) AS last_seen
   FROM analytics.shell_commands
   GROUP BY command_binary
   ORDER BY invocations DESC, command_binary'
```

### Tool Activity Joined To Turns

```bash
bitloops devql analytics sql \
  'SELECT
     t.session_id,
     t.turn_number,
     t.model,
     ti.tool_name,
     ti.command_binary,
     ti.command,
     ti.started_at,
     ti.ended_at
   FROM analytics.interaction_tool_invocations ti
   JOIN analytics.interaction_turns t
     ON t.repo_id = ti.repo_id
    AND t.turn_id = ti.turn_id
   ORDER BY ti.started_at DESC
   LIMIT 25'
```

## Troubleshooting

If a view is empty, check that the relevant data plane exists:

- Run `bitloops devql tasks enqueue --kind sync --status` for `analytics.current_file_state` and `analytics.repo_sync_state`.
- Run `bitloops status` to confirm the daemon and configured stores are available.
- Use `--all-repos` only when the local repository catalogue has been populated with the repositories you expect.
- Use `--repo <selector>` with a repo id or full identity when repository names are ambiguous.

If a query is rejected, make sure it is a single read-only `SELECT` or `WITH` statement and does not rely on external files or DuckDB extension loading.
