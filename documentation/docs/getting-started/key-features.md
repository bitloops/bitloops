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
- bootstrapping projects and toggling capture

## Repo Policy With Local Runtime State

Repositories can carry `.bitloops.toml` and `.bitloops.local.toml`. Durable relational, event, and blob data still live in platform app directories by default, while Bitloops keeps repo-scoped workflow runtime state in a derived local runtime SQLite for that repository.

## Queryable Development History

Bitloops stores checkpoints, events, and related enrichment data in configured backends, then exposes them through DevQL and GraphQL.

## Dashboard As A Launcher

`bitloops dashboard` opens the browser and ensures the daemon is running. It no longer owns the long-lived server process itself.

## Flexible Storage

By default Bitloops separates paths by intent:

- config in the config directory
- configured SQLite, DuckDB, and blob data in the data directory
- embedding downloads and dashboard bundle assets in the cache directory
- daemon runtime metadata and runtime SQLite in the state directory
- repo workflow runtime state in a derived repository-local runtime SQLite

## Agent And Hook Integration

`bitloops init` installs git hooks and the selected agent hooks for the current project. `bitloops enable` and `bitloops disable` then toggle capture on top of that installed hook setup. `bitloops uninstall` handles wider cleanup across global directories, services, binaries, shell integration, and known repositories.
