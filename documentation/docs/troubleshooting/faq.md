---
sidebar_position: 2
title: FAQ
---

# FAQ

### Do I still run `bitloops init` inside every repo?

You can, but `init` now prepares the global daemon config rather than repo hooks. Run `bitloops enable` inside each repo to install hooks.

### Where does Bitloops keep its data now?

In platform app directories by default:

- config directory for `config.toml`
- data directory for relational, event, and blob stores
- cache directory for embedding downloads and dashboard bundle assets
- state directory for daemon runtime metadata and hook scratch files

### Do I need a repo config file?

No. If no `.bitloops.toml` exists, Bitloops uses built-in thin-CLI defaults.

### What should go in `.bitloops.toml`?

Repo capture policy such as:

- `capture.enabled`
- `capture.strategy`
- watch settings
- scope rules
- imported knowledge references

### What should go in the daemon config?

Machine-scoped settings such as:

- store paths and backends
- provider credentials
- dashboard defaults
- daemon runtime defaults

### Does `bitloops dashboard` still run the server?

No. It launches the browser and ensures the daemon is running.

### What replaced `bitloops status` for repo capture status?

Use `bitloops checkpoints status`.

### Is there an automatic migration from the older JSON config?

No. The change is a hard break. See the [upgrade note](../reference/upgrading-to-the-daemon-architecture.md).
