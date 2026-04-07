---
sidebar_position: 2
title: Checkpoints and Sessions
---

# Checkpoints and Sessions

Bitloops captures work as sessions and checkpoints, then stores the durable record in daemon-managed backends.

## Sessions

A session represents an agent-assisted working run in a repository or worktree.

## Checkpoints

A checkpoint is a persisted summary of a meaningful step in that session, usually tied to the configured capture strategy such as `manual-commit`.

## Where State Lives Now

Under the current architecture:

- queryable checkpoint and session history live in the configured relational and event stores
- daemon runtime metadata and queue state live in the platform state directory runtime store
- repo-scoped workflow runtime state lives in `<config root>/stores/runtime/runtime.sqlite`

## Repo Policy

Capture behaviour comes from repo policy:

```toml
[capture]
enabled = true
strategy = "manual-commit"
```

Use:

```bash
bitloops checkpoints status
bitloops checkpoints status --detailed
```

The detailed view shows the resolved policy root and config fingerprint.
