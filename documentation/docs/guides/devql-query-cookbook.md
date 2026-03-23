---
sidebar_position: 4
title: DevQL Query Cookbook
---

# DevQL Query Cookbook

Practical query examples for the DevQL knowledge graph. All examples assume you've run `bitloops devql init` and `bitloops devql ingest`.

## Finding Artefacts

### List all artefacts for a language

```bash
bitloops devql query "artefacts(language:'rust')"
```

```
┌──────────────────────────┬──────────┬─────────────────────────────┐
│ name                     │ type     │ file                        │
├──────────────────────────┼──────────┼─────────────────────────────┤
│ main                     │ function │ src/main.rs                 │
│ Config                   │ struct   │ src/config.rs               │
│ run_cli                  │ function │ src/cli.rs                  │
│ StorageBackend           │ trait    │ src/storage/mod.rs          │
│ ...                      │          │                             │
└──────────────────────────┴──────────┴─────────────────────────────┘
```

### Filter by artefact kind

```bash
# Only functions
bitloops devql query "artefacts(kind:'function', language:'typescript')"

# Only interfaces
bitloops devql query "artefacts(kind:'interface', language:'typescript')"

# Only structs
bitloops devql query "artefacts(kind:'struct', language:'rust')"
```

### Find a specific symbol

```bash
bitloops devql query "artefacts(symbol_fqn:'auth::validate_token')"
```

## Dependency Queries

### What does a symbol depend on? (outgoing)

```bash
bitloops devql query "artefacts(symbol_fqn:'auth::validate_token') → deps(direction:'out')"
```

Returns every symbol that `validate_token` imports, calls, or references.

### What depends on a symbol? (incoming / reverse dependencies)

```bash
bitloops devql query "artefacts(symbol_fqn:'auth::validate_token') → deps(direction:'in')"
```

Returns every symbol that imports, calls, or references `validate_token`.

### Filter by edge kind

```bash
# Only call relationships
bitloops devql query "artefacts(symbol_fqn:'db::connect') → deps(direction:'in', kind:'calls')"

# Only import relationships
bitloops devql query "artefacts(symbol_fqn:'utils::format') → deps(direction:'in', kind:'imports')"
```

## Blast Radius — Impact Analysis

### "What will break if I change this function?"

```bash
bitloops devql query "artefacts(symbol_fqn:'auth::validate_token') → deps(direction:'in', kind:'calls')"
```

Returns every function that directly or transitively calls `validate_token`. If you change its signature, these are the artefacts that break.

### "What does this module's public API affect?"

```bash
bitloops devql query "artefacts(kind:'function', symbol_fqn:'api::handlers') → deps(direction:'in')"
```

## Historical Queries

### Query at a specific commit

```bash
bitloops devql query "asOf(commit:'a1b2c3d') → artefacts(kind:'function', language:'rust')"
```

### Query at a branch ref

```bash
bitloops devql query "asOf(ref:'main') → artefacts(kind:'struct')"
```

### Compare current vs historical

Run the same query with and without `asOf` to see what changed:

```bash
# Current state
bitloops devql query "artefacts(symbol_fqn:'auth')"

# State at last release
bitloops devql query "asOf(ref:'v1.0.0') → artefacts(symbol_fqn:'auth')"
```

## Checkpoint & Session Queries

### View all checkpoints

```bash
bitloops devql query "checkpoints"
```

```
┌──────────┬─────────────────────────────────────────┬──────────────┬────────┐
│ commit   │ message                                 │ agent        │ files  │
├──────────┼─────────────────────────────────────────┼──────────────┼────────┤
│ f4e5d6c  │ feat: add rate limiting middleware       │ claude-code  │ 3      │
│ a1b2c3d  │ refactor: switch auth to JWT             │ claude-code  │ 3      │
│ b9c8d7e  │ fix: handle null user in profile handler │ cursor       │ 1      │
└──────────┴─────────────────────────────────────────┴──────────────┴────────┘
```

### Browse AI conversation history

```bash
bitloops devql query "chat_history"
```

## Tips

- **Re-ingest after significant changes** — `bitloops devql ingest` updates the knowledge graph
- **Use `asOf` for safe exploration** — historical queries never affect current state
- **Combine with the dashboard** — `bitloops dashboard` provides a visual interface to the same graph
- **Check connectivity** — if queries fail, run `bitloops --connection-status`
