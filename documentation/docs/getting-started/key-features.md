---
sidebar_position: 2
title: Key Features
---

# Key Features

## Global Daemon, Thin CLI

Bitloops runs as a single global user-level daemon. The CLI is a thin control surface for:

- starting and stopping the daemon
- opening the dashboard
- sending DevQL requests
- installing and removing repository hooks

## Repo Policy Without Repo Runtime State

Repositories can carry `.bitloops.toml` and `.bitloops.local.toml`, but Bitloops no longer relies on repo-local runtime stores by default. Durable data lives in platform app directories instead.

## Queryable Development History

Bitloops stores checkpoints, events, and related enrichment data in configured backends, then exposes them through DevQL and GraphQL.

## Dashboard As A Launcher

`bitloops dashboard` opens the browser and ensures the daemon is running. It no longer owns the long-lived server process itself.

## Flexible Storage

By default Bitloops separates paths by intent:

- config in the config directory
- SQLite, DuckDB, and blob data in the data directory
- embedding downloads and dashboard bundle assets in the cache directory
- daemon runtime metadata and scratch files in the state directory

## Agent And Hook Integration

`bitloops enable` installs git hooks and supported agent hooks for the current repo. `bitloops disable` removes them again.
