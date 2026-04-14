---
sidebar_position: 4
title: How Bitloops Works
---

# How Bitloops Works

Bitloops now follows a daemon-first architecture.

## High-Level Flow

1. The fastest default onboarding path is `bitloops init --install-default-daemon` from inside the repository or subproject you want to capture.
2. That command bootstraps the default daemon service if needed, creates or updates `.bitloops.local.toml`, and installs hooks.
3. If you want to bootstrap the daemon separately, `bitloops start` launches the global daemon and, on a fresh machine, can prompt to create the default daemon config.
4. During that first default-config bootstrap, `bitloops start` also owns the first interactive telemetry consent prompt unless you pass an explicit telemetry flag.
5. `bitloops enable` and `bitloops disable` toggle capture in the nearest discovered project policy.
6. If telemetry consent later becomes unresolved for an existing daemon config, interactive `bitloops init` and `bitloops enable` can prompt again.
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
