---
sidebar_position: 2
title: Architecture Overview
---

# Architecture Overview

A quick map of the Bitloops codebase so you know where things live and how they connect.

## The Big Picture

Bitloops is a Rust CLI built with [Clap](https://docs.rs/clap) for command parsing and [Tokio](https://tokio.rs/) as the async runtime. It has four main responsibilities:

1. **Hook processing** — listen to AI agent events and capture sessions
2. **Code intelligence** — parse codebases, build dependency graphs, run queries
3. **Storage** — manage local databases (SQLite, DuckDB) and blob storage
4. **Dashboard** — serve a local web UI via [Axum](https://docs.rs/axum)

## Source Layout

```
bitloops/src/
├── main.rs                  # Entry point — parses CLI, dispatches commands
├── cli.rs                   # Command definitions (Clap derive)
├── cli/                     # One file per command (init, enable, status, devql, etc.)
│
├── host/                    # Core runtime — where most of the logic lives
│   ├── hooks/               # Agent hook dispatcher
│   ├── checkpoints/         # Session and checkpoint lifecycle
│   ├── devql/               # DevQL query engine
│   ├── capability_host/     # Capability pack hosting
│   └── extension_host/      # Extension pack registration and loading
│
├── adapters/                # External integrations
│   ├── agents/              # Per-agent adapters (Claude, Cursor, Copilot, Codex, Gemini, OpenCode)
│   ├── connectors/          # External service connectors (GitHub, Jira, Confluence)
│   └── model_providers/     # AI model provider adapters
│
├── capability_packs/        # First-party extensions
│   ├── knowledge/           # External knowledge ingestion
│   ├── semantic_clones/     # Code similarity detection
│   └── test_harness/        # Test-to-artefact mapping
│
├── storage/                 # Database backends
│   ├── sqlite.rs            # SQLite connection and queries
│   ├── postgres.rs          # PostgreSQL backend
│   ├── blob/                # Blob storage (local, S3, GCS)
│   └── init/                # Schema initialization and migrations
│
├── api.rs                   # Dashboard HTTP server (Axum routes)
├── api/                     # API endpoint handlers
├── config/                  # Configuration loading and resolution
├── models.rs                # Domain objects (Commit, Artefact, Edge)
├── git.rs                   # Git operations
├── telemetry/               # Anonymous usage analytics
└── utils/                   # Shared utilities
```

## How the Pieces Connect

```
AI Agent → Hook Script → bitloops hooks <agent> <event>
                              ↓
                    Adapter (per-agent translation)
                              ↓
                    Hook Processor (Git state fingerprinting)
                              ↓
                    Checkpoint Manager (Draft Commits → Committed Checkpoints)
                              ↓
                    Storage Layer (SQLite / PostgreSQL / DuckDB / Blob)
                              ↓
                    DevQL Engine ← CLI queries / Dashboard API / Agent queries
```

**Agent adapters** translate agent-specific events into a unified format. The **hook processor** uses Git state fingerprinting to detect real changes (not duplicate events). The **checkpoint manager** handles the Draft Commit → Committed Checkpoint lifecycle. The **storage layer** writes to the appropriate backend. The **DevQL engine** serves queries against the knowledge graph.

## Key Design Patterns

- **Adapter pattern** — each AI agent has its own adapter in `adapters/agents/`. Adding a new agent means writing a new adapter.
- **Capability packs** — features like knowledge ingestion, semantic clones, and test harness are isolated plugins in `capability_packs/`. Each has its own migrations, ingesters, and query stages.
- **Two-state storage** — live state (`*_current` tables) is updated continuously; committed state (`*` tables) is promoted on `git commit`.
- **Extension host** — packs register through a central host that manages their lifecycle (registration → migration → ingestion → query → health check).

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust (Edition 2024) |
| CLI framework | Clap 4 |
| Async runtime | Tokio |
| HTTP server | Axum |
| Code parsing | Tree-sitter (Rust, TypeScript, JavaScript) |
| Relational DB | SQLite (bundled) / PostgreSQL |
| Event store | DuckDB (bundled) / ClickHouse |
| Blob storage | Local filesystem / S3 / GCS |
| Embeddings | FastEmbed |
| Testing | Cargo test + Cucumber (BDD) |

## Where to Start

If you're fixing a bug in a CLI command → `src/cli/`

If you're adding a new agent → `src/adapters/agents/`

If you're working on DevQL queries → `src/host/devql/`

If you're adding a capability pack → `src/capability_packs/`

If you're touching storage → `src/storage/`

If you're working on the dashboard API → `src/api/`
