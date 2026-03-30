---
sidebar_position: 4
title: How Bitloops Works
---

# How Bitloops Works

Bitloops now follows a daemon-first architecture.

## High-Level Flow

1. `bitloops start` launches the global daemon and bootstraps the default daemon config when needed.
2. `bitloops init` bootstraps the current project or subproject, installs hooks, and runs the initial baseline sync.
3. Hooks and the slim CLI resolve the nearest project policy locally.
4. The daemon receives mutations and queries over the local transport.
5. The daemon stores durable data in configured backends and serves the dashboard and DevQL.

## Components

- Global daemon service: `com.bitloops.daemon`
- Thin CLI: lifecycle, hooks, DevQL, dashboard launcher
- Repo policy: `.bitloops.toml` and optional `.bitloops.local.toml`
- Global daemon config: `config.toml` in the platform config directory

## Default Storage Categories

- Config directory: daemon config
- Data directory: SQLite, DuckDB, blob storage
- Cache directory: embedding downloads, dashboard bundle
- State directory: runtime metadata and hook scratch files

This separation is why repo-local runtime directories are no longer the default.
