# Bitloops Test-Harness Quickstart (`rust-lang/cfg-if`)

Last updated: 2026-03-18

This quickstart shows a full local flow:

1. install the `bitloops` CLI from this workspace
2. clone a small Rust repo under `/tmp`
3. initialize Bitloops
4. create a real committed Bitloops checkpoint
5. run `bitloops devql ingest` to materialize production artefacts
6. continue with `bitloops testlens` against that same SQLite DB

This uses `rust-lang/cfg-if` because it is small and quick to re-run.

For Ruff-specific flows, keep these alongside this quickstart:

- [quickstart_ruff_bitloops_devql.md](/Users/markos/code/bitloops/bitloops/bitloops_cli/docs/test_harness/quickstart_ruff_bitloops_devql.md)
- [quickstart_ruff_fixture.md](/Users/markos/code/bitloops/bitloops/bitloops_cli/docs/test_harness/quickstart_ruff_fixture.md)

## Prerequisites

- Rust / Cargo
- `git`
- `sqlite3`
- this workspace checked out at:
  `/Users/markos/code/bitloops/bitloops`

## 1) Install the CLI from this workspace

```bash
cd /Users/markos/code/bitloops/bitloops

cargo install --path ./bitloops_cli --force

export PATH="$HOME/.cargo/bin:$PATH"

bitloops --version
```

Note:

- this quickstart assumes the current local workspace build, not an older globally installed `bitloops`
- `bitloops init --agent codex` is accepted by the current build even if some help text still omits `codex`

## 2) Clone `cfg-if`

```bash
cd /tmp
rm -rf cfg-if
git clone https://github.com/rust-lang/cfg-if.git
cd cfg-if

git config user.name "Codex"
git config user.email "codex@example.com"
```

## 3) Initialize Bitloops in the cloned repo

```bash
bitloops init --agent codex
bitloops enable --project
bitloops devql init
```

That creates the repo-local stores:

- `./.bitloops/stores/relational/relational.db`
- `./.bitloops/stores/event/events.duckdb`

## 4) Create one committed Bitloops checkpoint

`bitloops devql ingest` is checkpoint-driven. A plain git commit is not enough by itself.

The normal path is: make a change through a Bitloops-enabled agent turn, end the turn, then commit.

For a shell-only quickstart, the commands below drive the Codex hooks directly so you can reproduce the checkpoint from the terminal:

```bash
cd /tmp/cfg-if

export SESSION_ID="cfg-if-codex-demo"
export TRANSCRIPT_PATH="$PWD/codex-flow.jsonl"

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks codex session-start

python - <<'PY'
from pathlib import Path

lib = Path("src/lib.rs")
marker = "// bitloops quickstart checkpoint marker\n"
text = lib.read_text()
if marker not in text:
    text = text.replace("#![no_std]\n", marker + "#![no_std]\n", 1)
lib.write_text(text)

Path("codex-flow.jsonl").write_text("")
PY

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks codex stop

git add src/lib.rs codex-flow.jsonl
git commit -m "bitloops checkpoint demo"
```

At this point Bitloops should have:

- a committed row in `checkpoints`
- a mapping row in `commit_checkpoints`

## 5) Materialize production artefacts with DevQL

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

## 6) Verify that production rows exist

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

You want the important rows to be non-zero, especially:

- `checkpoints`
- `commit_checkpoints`
- `artefacts_current`

## 7) Continue with `bitloops testlens` on the same DB

```bash
export CFG_IF_DB="/tmp/cfg-if/.bitloops/stores/relational/relational.db"
export CFG_IF_COMMIT="$(git -C /tmp/cfg-if rev-parse HEAD)"

cd /tmp/cfg-if

bitloops testlens ingest-tests --commit "$CFG_IF_COMMIT"
```

Optional verification:

```bash
sqlite3 "$CFG_IF_DB" "
select 'test_suites', count(*) from test_suites
union all
select 'test_scenarios', count(*) from test_scenarios
union all
select 'test_links', count(*) from test_links;
"
```

## 8) What was validated

This flow was re-checked on 2026-03-18 against a disposable `/tmp/cfg-if` clone.

Observed outcomes:

- Bitloops checkpoint creation succeeded
- `bitloops devql ingest` succeeded against the repo-local SQLite DB
- production rows were materialized in the same DB
- `bitloops testlens ingest-tests` completed successfully for the current `cfg-if` commit

One validated `bitloops testlens ingest-tests` run reported:

```text
ingest-tests complete for commit <sha> (files: 2, suites: 4, scenarios: 5, links: 0, enumeration: hybrid-full, enumerated_scenarios: 2)
```

## Troubleshooting

- If `bitloops devql ingest` completes with `checkpoints_processed=0`, you do not have committed Bitloops checkpoint state yet. Repeat the `session-start -> edit -> stop -> git commit` cycle.
- If `artefacts_current` stays `0`, `bitloops testlens ingest-tests` will stop because there is no production state to link against.
- `bitloops devql init` creates the relational and test-harness schema but does not clear old data. If you want a clean rerun, start from a fresh clone or delete the DB first.
