---
sidebar_position: 3
title: DevQL
---

# DevQL — Development Query Language

DevQL is a graph-navigation query language for codebase intelligence. It traverses your repository's knowledge graph by chaining stages — navigating from repositories to files to artefacts to dependencies to tests, all resolved against immutable commit snapshots.

## Why DevQL?

Without structured context, AI agents waste tokens navigating your codebase and miss architectural patterns. DevQL gives agents precise, high-signal context:

- **Structural queries** — find specific artefacts by kind, language, or symbol name
- **Dependency traversal** — follow imports, calls, references, inheritance edges
- **Blast radius** — compute what breaks if a symbol changes
- **Historical snapshots** — query the codebase at any commit
- **External knowledge** — surface linked issues, tickets, and decisions

## Query Model — Stage Chaining

DevQL queries chain stages together, each narrowing or expanding the result set:

```
artefacts(kind:"function", language:"typescript")
  → deps(direction:"out", kind:"imports")
```

### Core Stages

| Stage | Purpose | Example |
|-------|---------|---------|
| `artefacts()` | Select code symbols | `artefacts(kind:"function", symbol_fqn:"auth::validate")` |
| `deps()` | Traverse dependencies | `deps(direction:"in", kind:"calls")` |
| `asOf()` | Query at a specific point in time | `asOf(commit:"a1b2c3d")` |

### Artefact Filters

```
artefacts(
  kind: "function | method | class | interface | type | enum | module | struct | trait",
  language: "typescript | javascript | rust",
  symbol_fqn: "module::symbol_name"
)
```

### Dependency Traversal

```
deps(
  direction: "out | in | both",
  kind: "imports | calls | references | extends | implements | exports"
)
```

- **`direction:"out"`** — what does this symbol depend on?
- **`direction:"in"`** — what depends on this symbol? (reverse dependencies)
- **`direction:"both"`** — full dependency neighbourhood

### Edge Kinds

| Kind | Meaning |
|------|---------|
| `imports` | Module A imports from module B |
| `calls` | Function A calls function B |
| `references` | Symbol A references symbol B |
| `extends` | Class A extends class B |
| `implements` | Struct A implements trait B |
| `exports` | Module A exports symbol B |

## Blast Radius — Impact Analysis

One of DevQL's most powerful capabilities: answering **"what will this break?"**

```bash
bitloops devql query "artefacts(symbol_fqn:'auth::validate_token') → deps(direction:'in', kind:'calls')"
```

This computes the transitive impact by following all incoming call edges — every function that directly or indirectly calls `validate_token`. If you change that function's signature, these are the artefacts that break.

## Historical Snapshots

DevQL resolves queries against immutable commit snapshots:

```bash
# Query current workspace state (default)
bitloops devql query "artefacts(language:'rust')"

# Query at a specific commit
bitloops devql query "asOf(commit:'a1b2c3d') → artefacts(kind:'function')"

# Query at a branch ref
bitloops devql query "asOf(ref:'main') → artefacts(kind:'struct')"
```

### Current vs Historical State

DevQL maintains two state models:

| State | Tables | Use Case |
|-------|--------|----------|
| **Current** | `artefacts_current`, `artefact_edges_current` | Latest workspace, including uncommitted changes |
| **Historical** | `artefacts`, `artefact_edges` | Committed state at any point in git history |

The default query (no `asOf`) returns current workspace state. Historical queries use `asOf(commit:"...")` or `asOf(ref:"...")`.

## Knowledge Graph Contents

**Artefacts** (nodes):
- Functions, methods, classes, interfaces, types, enums, structs, traits, modules
- Each with: source content, file path, line numbers, language, metadata

**Edges** (relationships):
- Imports, calls, references, extends, implements, exports
- Each with: source/target artefact, edge kind, line numbers, metadata

**External knowledge** (optional):
- GitHub issues/PRs, Jira tickets, Confluence pages
- Versioned and linked to specific commits and artefacts

## Supported Languages

| Language | Parser | Extracted Artefact Types |
|----------|--------|------------------------|
| Rust | tree-sitter-rust | Functions, structs, enums, traits, modules, impls |
| TypeScript | tree-sitter-typescript | Functions, classes, interfaces, types, modules |
| JavaScript | tree-sitter-javascript | Functions, classes, modules, exports |

Parsing is deterministic and parser-backed (Tree-sitter), not heuristic.

## Three-Store Architecture

| Store | Default | Contents |
|-------|---------|----------|
| **Relational** | SQLite (bundled) | Artefacts, dependency edges, semantic features |
| **Event** | DuckDB (bundled) | Checkpoints, transcripts, telemetry |
| **Blob** | Local filesystem | Raw content, embeddings, knowledge documents |

Zero configuration required — SQLite and DuckDB are compiled into the binary. See [Configuring Storage](/guides/configuring-storage) for team setups.

## Getting Started

See [Configuring DevQL](/guides/configuring-devql) for setup, and the [DevQL Query Cookbook](/guides/devql-query-cookbook) for practical examples.
