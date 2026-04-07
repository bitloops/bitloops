---
title: Test Harness Sync And DevQL Queries
---

# Test Harness Sync And DevQL Queries

This guide explains how `tests()` in DevQL gets populated, what `sync` updates automatically, and how to query covering tests for a specific artefact.

## How `sync` Updates Test Artefacts

`tests()` is provided by the `test_harness` capability pack.

When you run:

```bash
bitloops devql sync
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

`bitloops devql sync --validate` is read-only and does not trigger this update path.

## What Is Not Automatic

Sync-side updates cover source-based discovery and linkage refresh for current test artefacts.

Coverage and test-run results are separate ingestion flows. Use `testlens` commands for those:

```bash
bitloops testlens ingest-coverage --lcov coverage/lcov.info --commit <sha> --scope workspace
bitloops testlens ingest-results --jest-json reports/jest.json --commit <sha>
```

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
- `tests()` by itself is not a valid DSL pipeline.

For broader DevQL syntax and GraphQL mode details, see [DevQL GraphQL](/guides/devql-graphql) and [DevQL Query Cookbook](/guides/devql-query-cookbook).
