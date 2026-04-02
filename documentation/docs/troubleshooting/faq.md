---
sidebar_position: 2
title: FAQ
---

# FAQ

### Do I still run `bitloops init` inside every repo?

Yes. Run `bitloops init` in each repository or subproject you want Bitloops to manage. `init` creates `.bitloops.local.toml`, installs hooks, and prepares local repo policy for capture. Use DevQL commands separately for ingestion and sync.

### Where does Bitloops keep its data now?

In platform app directories by default:

- config directory for `config.toml`
- data directory for relational, event, and blob stores
- cache directory for embedding downloads and dashboard bundle assets
- state directory for daemon runtime metadata and hook scratch files

### How do I remove Bitloops completely?

Use:

```bash
bitloops uninstall --full
```

Use `bitloops disable` if you only want to stop capture for the current project while leaving hooks installed.

### Do I need a repo config file?

Yes for project-scoped commands. `bitloops init` creates `.bitloops.local.toml`, and Bitloops discovers the nearest `.bitloops.local.toml` or `.bitloops.toml` while walking up to the enclosing `.git` root.

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

### What creates the daemon config now?

Interactive `bitloops start` prompts to create the default daemon config when it is missing. For scripted or non-interactive setups, use `bitloops start --create-default-config` together with an explicit telemetry flag. `bitloops init --install-default-daemon` uses that same bootstrap path before continuing project init.

### When does Bitloops ask about telemetry?

On a fresh machine, the first interactive prompt happens during `bitloops start` when the default daemon config is created.

After that:

- `bitloops init` and `bitloops enable` only ask when the daemon config already existed and telemetry consent is unresolved
- non-interactive `start`, `init`, or `enable` require `--telemetry`, `--telemetry=false`, or `--no-telemetry` when consent is unresolved
- a previous opt-in carries forward across CLI upgrades
- a previous opt-out is cleared on a newer CLI version so Bitloops can ask again later

### What replaced `bitloops status` for repo capture status?

Use `bitloops checkpoints status`.

### Is there an automatic migration from the older JSON config?

No. The change is a hard break. See the [upgrade note](../reference/upgrading-to-the-daemon-architecture.md).
