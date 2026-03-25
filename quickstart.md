# Bitloops Test-Harness Quickstart (`rust-lang/cfg-if`)

Last updated: 2026-03-20

This quickstart shows a full local flow:

1. install the `bitloops` CLI from this workspace
2. clone a small Rust repo under `/tmp`
3. initialize Bitloops
4. create a real committed Bitloops checkpoint
5. run `bitloops devql ingest` to materialize production artefacts
6. continue with `bitloops testlens` against that same SQLite DB

This uses `rust-lang/cfg-if` because it is small and quick to re-run.

For Ruff-specific flows, keep these alongside this quickstart:

- [quickstart_ruff_bitloops_devql.md](/Users/markos/code/bitloops/bitloops/bitloops/docs/test_harness/quickstart_ruff_bitloops_devql.md)
- [quickstart_ruff_fixture.md](/Users/markos/code/bitloops/bitloops/bitloops/docs/test_harness/quickstart_ruff_fixture.md)

## Prerequisites

- Rust / Cargo
- `git`
- `sqlite3`
- this workspace checked out at:
  `/Users/markos/code/bitloops/bitloops`

## 1) Install the CLI from this workspace

```bash
cd /Users/markos/code/bitloops/bitloops

cargo install --path ./bitloops --force

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

python3 - <<'PY'
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

## 8) Verify that test rows exist

```bash
sqlite3 "$CFG_IF_DB" "
select 'test_suites', count(*) from test_suites
union all
select 'test_scenarios', count(*) from test_scenarios
union all
select 'test_links', count(*) from test_links;
"
```

You want `test_suites` and `test_scenarios` to be non-zero. `test_links` may be `0` if no static linkage was resolved (for example when production artefacts do not match any test imports). That is normal for small repos.

## 9) Generate coverage data

Coverage tells you which production artefacts were actually exercised by the test suite, even when static linkage is incomplete.

Generate an LCOV report with `cargo-llvm-cov`:

```bash
cd /tmp/cfg-if

cargo install cargo-llvm-cov    # one-time install

cargo llvm-cov --lcov --output-path coverage.lcov
```

## 10) Ingest coverage into the test harness

```bash
cd /tmp/cfg-if

bitloops testlens ingest-coverage \
  --lcov coverage.lcov \
  --commit "$CFG_IF_COMMIT" \
  --scope workspace \
  --tool cargo-llvm-cov
```

Expected output:

```text
ingested lcov coverage for commit <sha> (scope: workspace, hits: N, classifications: N, diagnostics: 0)
```

Key flags:

- `--scope workspace` — aggregate workspace-level coverage (most common for initial ingestion)
- `--scope package` — package-scoped coverage (when you generate LCOV per crate/package)
- `--tool` — name of the tool that generated the report (for traceability)
- `--format` — auto-detected from extension (`.lcov` or `.json` for LLVM JSON), can be overridden with `--format lcov` or `--format llvm-json`

## 11) Verify coverage data

```bash
sqlite3 "$CFG_IF_DB" "
select 'coverage_captures', count(*) from coverage_captures
union all
select 'coverage_hits', count(*) from coverage_hits
union all
select 'coverage_diagnostics', count(*) from coverage_diagnostics;
"
```

- `coverage_captures` should be `1` (one ingestion run)
- `coverage_hits` should be non-zero (line-level execution data mapped to artefacts)
- `coverage_diagnostics` should be `0` for a clean report (non-zero means some files could not be mapped)

## 12) Query artefacts with coverage

```bash
bitloops testlens query \
  --artefact src/lib.rs::tests \
  --commit "$CFG_IF_COMMIT" \
  --view summary
```

On this `cfg-if` flow, `src/lib.rs::tests` is a stable query target after the checkpointed ingest. With coverage ingested, the summary includes `line_coverage_pct` and `branch_coverage_pct`. The `coverage_mode` field tells you the kind of coverage available:

- `none` — no coverage data ingested for this commit
- `artefact_only` — workspace-level coverage exists but per-test attribution is not available
- `per_test_line` — per-test line coverage exists
- `per_test_branch` — per-test line and branch coverage exist

For detailed branch info:

```bash
bitloops testlens query \
  --artefact cfg_if \
  --commit "$CFG_IF_COMMIT" \
  --view coverage
```

## 13) Batch coverage ingestion (optional)

When your CI produces multiple coverage files (e.g. one per crate), use a JSON manifest instead of multiple CLI calls:

```bash
cat > coverage-manifest.json <<'JSON'
[
  {"format": "lcov", "path": "crate_a/coverage.lcov", "scope": "package", "tool": "cargo-llvm-cov"},
  {"format": "lcov", "path": "crate_b/coverage.lcov", "scope": "package", "tool": "cargo-llvm-cov"}
]
JSON

bitloops testlens ingest-coverage-batch \
  --manifest coverage-manifest.json \
  --commit "$CFG_IF_COMMIT"
```

## 15) What was validated

This flow was re-checked on 2026-03-20 against a disposable `/tmp/cfg-if` clone.

Observed outcomes:

- Bitloops checkpoint creation succeeded
- `bitloops devql ingest` succeeded against the repo-local SQLite DB
- production rows were materialized in the same DB
- `bitloops testlens ingest-tests` completed successfully for the current `cfg-if` commit
- `bitloops testlens ingest-coverage` ingested LCOV data with line and branch coverage hits
- `bitloops testlens query --view coverage` returned artefact-level coverage percentages
- coverage diagnostics were recorded for unmappable file paths

One validated `bitloops testlens ingest-tests` run reported:

```text
ingest-tests complete for commit <sha> (files: 2, suites: 4, scenarios: 5, links: 0, enumeration: hybrid-full, enumerated_scenarios: 2)
```

## Troubleshooting

- If `bitloops devql ingest` completes with `checkpoints_processed=0`, you do not have committed Bitloops checkpoint state yet. Repeat the `session-start -> edit -> stop -> git commit` cycle.
- If `artefacts_current` stays `0`, `bitloops testlens ingest-tests` will stop because there is no production state to link against.
- `bitloops devql init` creates the relational and test-harness schema but does not clear old data. If you want a clean rerun, start from a fresh clone or delete the DB first.
- If `ingest-coverage` reports `diagnostics: N` with N > 0, some coverage file paths could not be mapped to production artefacts. This usually means the source files were not part of a DevQL checkpoint or the paths in the LCOV report do not match the repo-relative paths in the artefact index.
- If `coverage_mode` in query output is `none`, no coverage has been ingested for that commit yet. Run `ingest-coverage` first.
