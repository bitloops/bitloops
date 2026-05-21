---
title: Test Harness Sync And DevQL Queries
---

# Test Harness Sync And DevQL Queries

This guide explains how `tests()` and `coverage()` in DevQL get populated, what `sync` updates automatically, and how to query test-harness data for a specific artefact.

## How `sync` Updates Test Artefacts

`tests()` is provided by the `test_harness` capability pack.

When you run:

```bash
bitloops devql tasks enqueue --kind sync
```

or:

```bash
bitloops init --sync=true
```

the daemon runs a current-state sync. After a successful non-`validate` sync, it emits `SyncCompleted`, and the test-harness sync handler updates pack-owned current tables as a side effect:

- discovers tests in added/changed test files
- rewrites `test_artefacts_current` and `test_artefact_edges_current` for touched paths
- removes rows for deleted files
- removes edges pointing to deleted production symbols

`bitloops devql tasks enqueue --kind sync --validate` is read-only and does not trigger this update path.

## What Is Not Automatic

Sync-side updates cover source-based discovery and linkage refresh for current test artefacts.

Coverage and test-run results are separate ingestion flows. Use `devql test-harness` commands for those:

```bash
bitloops devql test-harness ingest-coverage --lcov bitloops/target/llvm-cov.info --tool cargo-llvm-cov
bitloops devql test-harness ingest-results --jest-json reports/jest.json --commit <sha>
```

Coverage ingest defaults to the current workspace when `--commit` is omitted. In current mode, the ingester maps LCOV file/line entries to `artefacts_current`, so run a current-state sync before ingesting a fresh coverage report:

```bash
bitloops devql init
bitloops devql tasks enqueue --kind sync --full --status
cargo dev-coverage
bitloops devql test-harness ingest-coverage --lcov bitloops/target/llvm-cov.info --tool cargo-llvm-cov
```

Pass `--commit <sha>` only for the legacy historical coverage path. Historical mode maps coverage to historical file state and commit-scoped artefacts.

## Query One Artefact And Its Covering Tests

Run queries from inside a git repository (or one of its subdirectories).

### 1) Select a specific production artefact

```bash
bitloops devql query 'artefacts(symbol_fqn:"src/lib.rs::add")->limit(1)'
```

### 2) Attach test-harness data with `tests()`

```bash
bitloops devql query 'artefacts(symbol_fqn:"src/lib.rs::add")->tests()'
```

In table mode, nested arrays are summarized as `[N entries]`. That is expected for columns like `tests`.

### 3) Expand nested payloads with compact JSON

```bash
bitloops devql query --compact 'artefacts(symbol_fqn:"src/lib.rs::add")->tests()'
```

To inspect only covering tests:

```bash
bitloops devql query --compact 'artefacts(symbol_fqn:"src/lib.rs::add")->tests()' | jq '.[0].tests[0].coveringTests'
```

Typical shape:

```json
[
  {
    "filePath": "src/lib.rs",
    "startLine": 209,
    "endLine": 213,
    "suiteName": "tests",
    "testId": "8fef3e25-fbf7-8780-464d-6228cb599f9e",
    "testName": "test_add"
  }
]
```

### 4) Filter by confidence or linkage source

```bash
bitloops devql query 'artefacts(symbol_fqn:"src/lib.rs::add")->tests(min_confidence:0.6, linkage_source:"static_analysis")'
```

## Query Coverage

Run current coverage queries after generating an LCOV report, running `ingest-coverage` without `--commit`, and syncing current artefacts.

### 1) Check one function

Use `symbol_fqn` for exact matching. Function `name` may be absent in the current artefact payload, so prefer `symbolFqn` when scripting.

```bash
bitloops devql query --compact \
  'repo("bitloops")->artefacts(symbol_fqn:"bitloops/src/capability_packs/test_harness/ingest/coverage.rs::execute_current")->coverage()->limit(5)' \
  | jq '.[] | {
      path,
      symbolFqn,
      hasCoverageData: ((.coverage // []) | length > 0),
      lineCoveragePct: (.coverage[0].coverage.lineCoveragePct // null),
      lineDataAvailable: (.coverage[0].coverage.lineDataAvailable // false),
      uncoveredLineCount: (.coverage[0].summary.uncoveredLineCount // null)
    }'
```

### 2) List functions with coverage data

```bash
bitloops devql query --compact \
  'repo("bitloops")->artefacts(kind:"function")->coverage()->limit(1000)' \
  | jq '.[] | select((.coverage // []) | length > 0) | {
      path,
      symbolFqn,
      lineCoveragePct: .coverage[0].coverage.lineCoveragePct
    }'
```

### 3) List functions with no coverage row

```bash
bitloops devql query --compact \
  'repo("bitloops")->artefacts(kind:"function")->coverage()->limit(1000)' \
  | jq '.[] | select(((.coverage // []) | length) == 0) | {
      path,
      symbolFqn
    }'
```

No output means every artefact returned within the query window had a mapped coverage row. That is different from `0%` coverage: `0%` means the report mapped to the artefact and none of its executable lines were hit.

### 4) List functions with mapped coverage but zero hits

```bash
bitloops devql query --compact \
  'repo("bitloops")->artefacts(kind:"function")->coverage()->limit(1000)' \
  | jq '.[] | select((.coverage[0].coverage.lineCoveragePct // null) == 0) | {
      path,
      symbolFqn,
      uncoveredLineCount: .coverage[0].summary.uncoveredLineCount
    }'
```

The artefact-nested DSL shape is:

```json
[
  {
    "path": "bitloops/src/example.rs",
    "symbolFqn": "bitloops/src/example.rs::run",
    "coverage": [
      {
        "coverage": {
          "lineCoveragePct": 84.5,
          "lineDataAvailable": true
        },
        "summary": {
          "uncoveredLineCount": 7
        }
      }
    ]
  }
]
```

If you add `project("...")`, the CLI checks that the project matches the directory scope. From the repository root, omit `project(...)`; from a crate subdirectory, use the matching project path.

## GraphQL Equivalents (Concrete Example)

If you prefer raw GraphQL instead of DSL, this is the same flow with a real symbol from this codebase.

### 1) Find one artefact by `symbolFqn`

```bash
bitloops devql query --compact '{
  artefacts(
    filter: { symbolFqn: "bitloops/src/host/checkpoints/lifecycle/adapters.rs::impl@64::parse_hook_event" }
    first: 1
  ) {
    edges {
      node {
        id
        path
        symbolFqn
        canonicalKind
        startLine
        endLine
      }
    }
  }
}'
```

### 2) Fetch covering tests for that artefact

```bash
bitloops devql query --compact '{
  artefacts(
    filter: { symbolFqn: "bitloops/src/host/checkpoints/lifecycle/adapters.rs::impl@64::parse_hook_event" }
    first: 1
  ) {
    edges {
      node {
        symbolFqn
        tests {
          coveringTests {
            testId
            testName
            suiteName
            filePath
            startLine
            endLine
          }
          summary {
            totalCoveringTests
          }
        }
      }
    }
  }
}'
```

If you want only the list quickly:

```bash
bitloops devql query --compact '{
  artefacts(
    filter: { symbolFqn: "bitloops/src/host/checkpoints/lifecycle/adapters.rs::impl@64::parse_hook_event" }
    first: 1
  ) {
    edges {
      node {
        tests {
          coveringTests { testId testName suiteName filePath startLine endLine }
        }
      }
    }
  }
}' | jq '.artefacts.edges[0].node.tests[0].coveringTests'
```

### 3) Validate summary count matches returned list

`tests.summary.totalCoveringTests` should match the number of rows in `coveringTests` for the same query window.

```bash
bitloops devql query --compact '{
  artefacts(
    filter: { symbolFqn: "bitloops/src/host/checkpoints/lifecycle/adapters.rs::impl@64::parse_hook_event" }
    first: 1
  ) {
    edges {
      node {
        tests {
          summary { totalCoveringTests }
          coveringTests { testId }
        }
      }
    }
  }
}' | jq '.artefacts.edges[0].node.tests[0] | {summary: .summary.totalCoveringTests, list: (.coveringTests|length)}'
```

Example output:

```json
{
  "summary": 25,
  "list": 25
}
```

## DSL Notes

- DevQL DSL mode is used when the query contains `->`.
- `tests()` is a stage and must follow `artefacts(...)`.
- `coverage()` is a stage and must follow `artefacts(...)`.
- `tests()` by itself is not a valid DSL pipeline.
- `coverage()` by itself is not a valid DSL pipeline.

For broader DevQL syntax and GraphQL mode details, see [DevQL GraphQL](/guides/devql-graphql) and [DevQL Query Cookbook](/guides/devql-query-cookbook).
