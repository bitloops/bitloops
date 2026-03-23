# Ruff + Bitloops DevQL Quickstart

Last updated: 2026-03-19

This quickstart restores the old Ruff Bitloops-first flow after the `TestLens` migration.

Use it when you want to:

- clone the real Ruff workspace
- create committed Bitloops checkpoints
- materialize production artefacts into the Bitloops relational DB
- continue later with `bitloops testlens`

Historical note:

- the old in-repo Ruff fixture snapshot was removed with `TestLens`
- this replacement quickstart uses a fresh local Ruff clone instead

## Target repo

- Repo: `https://github.com/astral-sh/ruff`
- Commit used in the previous validation docs:
  - `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`

## Important boundary

`bitloops devql ingest` is checkpoint-driven.

It does not scan the whole Ruff workspace from scratch. It materializes production artefacts for files that are part of committed Bitloops checkpoint state. If you want specific Ruff artefacts to be queryable later, make sure your checkpoint touches the production source files that define them.

## Prerequisites

- Rust / Cargo
- `git`
- `sqlite3`
- `bitloops` installed from this workspace

## 1) Install the CLI

From `/Users/markos/code/bitloops/bitloops`:

```bash
cargo install --path ./bitloops --force
export PATH="$HOME/.cargo/bin:$PATH"

bitloops --version
```

## 2) Clone Ruff and pin the commit

```bash
cd /tmp
rm -rf ruff
git clone https://github.com/astral-sh/ruff.git
cd ruff
git checkout 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5

git config user.name "Codex"
git config user.email "codex@example.com"
```

## 3) Initialize Bitloops in the Ruff clone

```bash
bitloops init --agent codex
bitloops enable --project
bitloops devql init
```

This creates the repo-local stores:

- `./.bitloops/stores/relational/relational.db`
- `./.bitloops/stores/event/events.duckdb`

## 4) Choose the Ruff production files you want materialized

Because DevQL ingest is checkpoint-scoped, start with the production Rust files you want the test harness to answer questions about.

Examples:

- the F523 rule entry point file
- the ERA001 rule entry point file
- helper files you expect to query directly

The safest workflow is:

1. pick the exact Ruff source file(s) you care about
2. make a tiny harmless local edit in each one
3. commit those edits through a Bitloops checkpointed session

If you later need more Ruff artefacts, repeat the same cycle for additional files and rerun `bitloops devql ingest`.

## 5) Create one committed Bitloops checkpoint

The shell-only flow below is the same checkpoint bootstrap used in the smaller `cfg-if` quickstart. Replace `TARGET_FILE` with the Ruff production file you want materialized first.

```bash
cd /tmp/ruff

export TARGET_FILE="path/to/the/ruff/source/file.rs"
export SESSION_ID="ruff-devql-demo"
export TRANSCRIPT_PATH="$PWD/codex-flow.jsonl"

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks codex session-start

python - <<'PY'
from pathlib import Path
import os

target = Path(os.environ["TARGET_FILE"])
marker = "// bitloops quickstart checkpoint marker\n"
text = target.read_text()
if marker not in text:
    target.write_text(marker + text)

Path("codex-flow.jsonl").write_text("")
PY

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks codex stop

git add "$TARGET_FILE" codex-flow.jsonl
git commit -m "bitloops checkpoint for Ruff DevQL ingest"
```

At this point Bitloops should have:

- a committed row in `checkpoints`
- a mapping row in `commit_checkpoints`

## 6) Materialize production artefacts

Fast boundary-check run:

```bash
BITLOOPS_DEVQL_EMBEDDING_PROVIDER=none \
BITLOOPS_DEVQL_SEMANTIC_PROVIDER=none \
bitloops devql ingest
```

If you want the full semantic / embedding path instead, run:

```bash
bitloops devql ingest
```

## 7) Verify that production rows exist

```bash
sqlite3 ./.bitloops/stores/relational/relational.db "
select 'checkpoints', count(*) from checkpoints
union all
select 'commit_checkpoints', count(*) from commit_checkpoints
union all
select 'commits', count(*) from commits
union all
select 'file_state', count(*) from file_state
union all
select 'artefacts_current', count(*) from artefacts_current
union all
select 'artefact_edges_current', count(*) from artefact_edges_current;
"
```

You want these to be non-zero:

- `checkpoints`
- `commit_checkpoints`
- `artefacts_current`

## 8) Continue with the test harness

Once DevQL has materialized the production rows you need, continue with:

- [quickstart_ruff_fixture.md](/Users/markos/code/bitloops/bitloops/bitloops/docs/test_harness/quickstart_ruff_fixture.md)

That flow picks up from `bitloops testlens ingest-tests`.

## Troubleshooting

- If `bitloops devql ingest` reports `checkpoints_processed=0`, you do not have committed Bitloops checkpoint state yet.
- If `artefacts_current` stays `0`, the test harness will have nothing to link against.
- If a later `bitloops testlens query` says an artefact is missing, that usually means the source file defining it was never part of a committed DevQL checkpoint. Touch that file in a new checkpoint and rerun `bitloops devql ingest`.
