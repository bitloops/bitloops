---
sidebar_position: 4
title: How Bitloops Works
---

# How Bitloops Works

Bitloops now follows a daemon-first architecture.

## High-Level Flow

1. The fastest default onboarding path is `bitloops configure --web`, followed by `bitloops init` inside each repository or subproject you want to capture.
2. `configure` owns the daemon config and daemon lifecycle.
3. `init` creates or updates `.bitloops.local.toml`, installs hooks, and reconciles the repo watcher when the daemon is already running.
4. Telemetry is a daemon configuration setting.
5. `bitloops enable` and `bitloops disable` let you toggle `Capture` and `DevQL Guidance` in the nearest discovered project policy.
6. If telemetry consent later becomes unresolved for an existing daemon config, use `bitloops configure --web`.
7. Hooks and the slim CLI resolve the nearest project policy locally.
8. The CLI parses or compiles requests locally, then the daemon receives mutations and queries over the local GraphQL transport.
9. The daemon stores durable data in configured backends and serves the dashboard and DevQL.

## Components

- Global daemon service: `com.bitloops.daemon`
- Thin CLI: lifecycle, hooks, DevQL, dashboard launcher
- Repo policy: `.bitloops.toml` and optional `.bitloops.local.toml`
- Global daemon config: `config.toml` in the platform config directory

## Default Storage Categories

- Config directory: daemon config
- Data directory: SQLite, DuckDB, blob storage
- Cache directory: embedding downloads, dashboard bundle
- State directory: daemon runtime metadata, daemon runtime SQLite, and hook scratch files
- Repo runtime directory: `<config root>/stores/runtime/runtime.sqlite`

This separation keeps configured relational, event, and blob stores machine-scoped by default while preserving repo-scoped runtime state for active workflow data.
