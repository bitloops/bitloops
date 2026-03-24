---
sidebar_position: 4
title: The Knowledge Store
---

# The Knowledge Store

The Knowledge Store is where all of Bitloops's intelligence lives. It's a unified, local, persistent database that sits alongside your codebase without modifying it.

## Architecture

Three stores, each optimized for what it holds:

| Store | Technology | What It Holds |
|-------|-----------|---------------|
| **Relational** | SQLite (bundled) | Artefacts, dependency edges, semantic features, metadata |
| **Event** | DuckDB (bundled) | Checkpoints, session transcripts, telemetry |
| **Blob** | Local filesystem | Raw content, embeddings, knowledge documents |

Everything is bundled in the Bitloops binary. No databases to install, no servers to run.

## How Data Flows Through It

### During a session (live)

As you work with your AI agent, Bitloops continuously updates the SQLite database with:

- Local file edits and new artefacts
- Dependency edges as code structure changes
- Agent activity and session data

This is the **current state** — a live view of your workspace that's always up to date.

### On commit (permanent)

When you `git commit`, the current state is promoted to **committed state**:

- Draft Commits become Committed Checkpoints
- Current artefacts and edges are moved to the committed tables
- Session transcripts are finalized in DuckDB
- Raw content is persisted to the blob store

In a default setup, committed data stays in local SQLite. If you've configured PostgreSQL for your team, it goes there instead — giving everyone shared access to the same intelligence.

### The two-state model

| State | Where | Purpose |
|-------|-------|---------|
| **Current** | `artefacts_current`, `artefact_edges_current` | Live workspace — includes uncommitted changes |
| **Committed** | `artefacts`, `artefact_edges` | Permanent history — queryable at any commit |

DevQL queries default to current state. Use `asOf(commit:"...")` to query historical state at any point in git history.

## Where It Lives

```
.bitloops/
├── config.json              # Project configuration
├── settings.json            # Runtime settings
├── checkpoints/v1/          # Committed Checkpoints (git-tracked)
└── stores/                  # Knowledge Store databases (gitignored)
    ├── relational/          # SQLite database
    ├── event/               # DuckDB database
    └── blob/                # Raw files and embeddings
```

**Checkpoints are committed to git** — your team sees the AI reasoning behind any commit. **Stores are gitignored** — each developer has their own local databases, rebuilt with `bitloops devql ingest`.

## Scaling for Teams

The defaults (SQLite + DuckDB + local filesystem) work great for individual developers. For teams:

| Need | Solution |
|------|----------|
| Shared artefact data | PostgreSQL as relational backend |
| High-volume analytics | ClickHouse as event backend |
| Centralized storage | AWS S3 or Google Cloud Storage as blob backend |

See [Configuring Storage](/guides/configuring-storage) for setup details.

## It's Local-First

Your code never leaves your machine by default. The Knowledge Store runs entirely locally — no cloud service, no account, no network access required.

The only scenarios involving network access are optional and explicitly configured: knowledge ingestion (GitHub/Jira APIs), semantic embeddings (OpenAI), and cloud blob storage (S3/GCS).
