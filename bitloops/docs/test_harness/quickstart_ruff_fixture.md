# Ruff Workspace Quickstart

Last updated: 2026-03-19

This restores the old Ruff workspace quickstart after the `TestLens` migration.

Historical note:

- the old filename is preserved because other notes referenced it
- the old in-repo Ruff snapshot is gone
- this replacement quickstart uses a fresh real Ruff clone plus `bitloops testlens`

## Goal

Run the migrated Bitloops test harness against Ruff so you can:

- ingest Rust tests
- ingest workspace coverage
- query helpful artefacts before changing Ruff code

## Starting point

Complete the DevQL setup first:

- [quickstart_ruff_bitloops_devql.md](/Users/markos/code/bitloops/bitloops/bitloops/docs/test_harness/quickstart_ruff_bitloops_devql.md)

Assume:

- repo path: `/tmp/ruff`
- target commit: `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`
- relational DB: `/tmp/ruff/.bitloops/stores/relational/relational.db`

## Important boundary

Because DevQL ingest is checkpoint-scoped, this quickstart only works for Ruff production files that you already materialized through committed Bitloops checkpoints.

If a query later says an artefact is missing or unlinked, first check whether the source file defining that artefact was part of your DevQL ingest.

## 1) Ingest Ruff tests

```bash
cd /tmp/ruff

export RUFF_COMMIT="$(git rev-parse HEAD)"

bitloops testlens ingest-tests --commit "$RUFF_COMMIT"
```

Expected behavior:

- Rust tests are discovered from the Ruff workspace
- links are created from discovered tests to the production artefacts already present in the relational DB
- enumeration may be `source-only`, `hybrid-partial`, or `hybrid-full` depending on local cargo enumeration success and timeout behavior

## 2) Generate workspace coverage

Optional but strongly recommended.

This is the current high-value coverage mode for Ruff because it tells you whether a production artefact was exercised somewhere in the workspace, even when static linkage is incomplete.

```bash
cargo llvm-cov --lcov --output-path ./.bitloops/ruff-fixture.lcov -p ruff_linter
```

## 3) Ingest coverage into the test harness

```bash
bitloops testlens ingest-coverage \
  --lcov ./.bitloops/ruff-fixture.lcov \
  --commit "$RUFF_COMMIT" \
  --scope workspace \
  --tool cargo-llvm-cov
```

Important note:

- Ruff workspace LCOV is still workspace-level coverage
- query output will report `coverage_mode: artefact_only`
- this tells you that an artefact was exercised somewhere, not which exact Ruff test exercised it

## 4) Example queries

### F523 rule entry point

```bash
bitloops testlens query \
  --artefact string_dot_format_extra_positional_arguments \
  --commit "$RUFF_COMMIT" \
  --view summary
```

What this is good for:

- checking whether the main F523 rule logic is already protected
- deciding whether you are editing a blind spot

### F523 linked tests

```bash
bitloops testlens query \
  --artefact string_dot_format_extra_positional_arguments \
  --commit "$RUFF_COMMIT" \
  --view tests \
  --min-strength 0.0
```

What this is good for:

- discovering the Ruff harness cases you should run after a change
- seeing whether you have only a rule-harness link or also nearby doctest/static links

### ERA001 rule entry point

```bash
bitloops testlens query \
  --artefact commented_out_code \
  --commit "$RUFF_COMMIT" \
  --view tests \
  --min-strength 0.0
```

What this is good for:

- finding the main ERA001 regression harness before touching the parser logic

### ERA001 helper surface

```bash
bitloops testlens query \
  --artefact skip_script_comments \
  --commit "$RUFF_COMMIT" \
  --view tests \
  --min-strength 0.0
```

What this is good for:

- checking whether the script-boundary helper has direct unit tests

## 5) How to interpret the output

Useful rules of thumb:

- linked tests + coverage: strongest signal
- no linked tests + high workspace coverage: probably exercised indirectly
- no linked tests + no coverage: real blind spot
- missing artefact entirely: DevQL never materialized the defining production file

## 6) Troubleshooting

- If `bitloops testlens ingest-tests` fails because production rows are missing, go back to the DevQL quickstart and create a committed checkpoint for the relevant Ruff source file.
- If cargo-based enumeration times out on the full workspace, `ingest-tests` can still succeed with source-driven discovery.
- If coverage ingestion succeeds but a helper still shows no coverage, it may be outside the crate you covered or outside the files you materialized through DevQL.
