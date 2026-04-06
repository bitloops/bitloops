---
sidebar_position: 3
title: DevQL
---

# DevQL

DevQL is Bitloops's typed query surface for codebase intelligence. It is implemented as a GraphQL-compatible schema, with a terminal-friendly DSL layered on top for CLI use.

That means you can use DevQL in three ways:

- DevQL DSL pipelines in the CLI, such as `repo("bitloops")->artefacts()->limit(10)`
- Raw GraphQL documents in the CLI, such as `bitloops devql query '{ repo(name: "bitloops") { ... } }'`
- HTTP and WebSocket clients against `/devql` and `/devql/ws` when the dashboard server is running

## Why It Exists

DevQL gives agents and developers a typed, introspectable view of repository state so they can ask for precise context instead of scraping ad hoc text.

It is designed for:

- Structural queries over files, artefacts, dependency edges, commits, checkpoints, telemetry, and knowledge
- Temporal queries with `asOf(...)` so reads can be pinned to a commit, branch ref, or save state
- Monorepo scoping through `project(path: ...)`
- Capability-pack enrichments such as knowledge, test coverage, covering tests, and semantic clones

## Core Model

The schema starts at a repository and then narrows from there:

```text
Repository
  -> Project
  -> FileContext
  -> Artefact
  -> Checkpoint / Telemetry / Knowledge
```

The same graph can also be traversed historically:

```text
Repository
  -> asOf(input: ...)
  -> Project / FileContext / Artefact
```

Most list fields use cursor-based GraphQL connections, so pagination, field selection, and introspection all work with standard GraphQL tooling.

## Query Modes

`bitloops devql query` is daemon-backed:

- If the query contains `->`, the CLI treats it as DevQL DSL and compiles it to GraphQL first
- Otherwise, the CLI treats the input as raw GraphQL
- `--graphql` is available as an explicit raw-GraphQL override

The CLI parses or compiles locally, then executes against the local Bitloops daemon GraphQL surface. The result is one execution engine for CLI, dashboard, and external GraphQL clients.

## Capability Packs

Capability packs extend DevQL through typed fields and generic stage execution:

- `knowledge(...)`
- `tests(...)`
- `coverage(...)`
- `clones(...)`
- `extension(stage:, args:, first:)`

This keeps the core schema typed while still leaving room for pack-specific extensions.

## Storage Model

DevQL still uses the three-store Bitloops architecture:

| Store | Default | Purpose |
|---|---|---|
| Relational | SQLite | Artefacts, dependency edges, semantic features, and pack-owned relational state |
| Event | DuckDB | Checkpoints, sessions, and telemetry |
| Blob | Local filesystem | Large payloads, knowledge bodies, and other blob-backed content |

## Learn More

Start with [Configuring DevQL](/guides/configuring-devql), then use:

- [DevQL GraphQL](/guides/devql-graphql) for endpoints, SDL export, mutations, subscriptions, and migration notes
- [DevQL Query Cookbook](/guides/devql-query-cookbook) for practical query examples
