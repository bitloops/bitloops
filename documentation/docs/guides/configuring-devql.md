---
sidebar_position: 3
title: Configuring DevQL
---

# Configuring DevQL

Set up DevQL to index your codebase and build the knowledge graph.

## Defaults Work Out of the Box

DevQL uses SQLite, DuckDB, and local filesystem by default — all bundled with Bitloops. You don't need to configure anything to get started.

## Step 1: Initialize the Schema

```bash
bitloops devql init
```

```
✔ Relational store (SQLite) initialized
✔ Event store (DuckDB) initialized
✔ Blob store (local) initialized
✔ Schema ready
```

## Step 2: Verify Connectivity

```bash
bitloops --connection-status
```

```
Relational (SQLite): ✔ connected  (.bitloops/stores/relational/relational.db)
Event (DuckDB):      ✔ connected  (.bitloops/stores/event/events.duckdb)
Blob (local):        ✔ available  (.bitloops/stores/blob/)
```

## Step 3: Ingest Your Codebase

```bash
bitloops devql ingest
```

```
✔ Scanning repository...
✔ Parsed 142 artefacts across 38 files
✔ Mapped 89 dependency relationships
✔ Knowledge graph updated

Languages: Rust (98 artefacts), TypeScript (44 artefacts)
Duration: 3.2s
```

Bitloops uses Tree-sitter to parse source files and extract functions, structs, classes, modules, and their relationships.

## Step 4: Query

```bash
bitloops devql query "artefacts(language='rust')"
```

```
┌──────────────────────────┬──────────┬─────────────────────────────┐
│ name                     │ type     │ file                        │
├──────────────────────────┼──────────┼─────────────────────────────┤
│ main                     │ function │ src/main.rs                 │
│ Config                   │ struct   │ src/config.rs               │
│ run_cli                  │ function │ src/cli.rs                  │
│ ...                      │          │                             │
└──────────────────────────┴──────────┴─────────────────────────────┘
```

See the [DevQL Query Cookbook](/guides/devql-query-cookbook) for more query examples.

## Step 5: Launch the Dashboard (Optional)

```bash
bitloops dashboard
```

Open `http://localhost:5667` to visually browse artefacts, relationships, and store health.

## Re-ingesting After Changes

After significant code changes, update the knowledge graph:

```bash
bitloops devql ingest
```

DevQL processes changes incrementally where possible.

## Custom Store Configuration

To use alternative backends (PostgreSQL, ClickHouse, S3), see [Configuring Storage](/guides/configuring-storage).

Example for a team setup with PostgreSQL:

```json title=".bitloops/config.json"
{
  "stores": {
    "relational": {
      "provider": "postgres",
      "postgres_dsn": "${BITLOOPS_PG_DSN}"
    },
    "event": {
      "provider": "duckdb"
    },
    "blob": {
      "provider": "local"
    }
  }
}
```

After changing store configuration, re-run `bitloops devql init` to create the schema in the new backend.
