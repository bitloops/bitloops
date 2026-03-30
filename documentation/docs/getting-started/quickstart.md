---
sidebar_position: 1
title: Quickstart
---

# Quickstart

This quickstart assumes you want the new daemon-first Bitloops setup.

If you are coming from the old JSON and repo-local storage model, read the [upgrade note](../reference/upgrading-to-the-daemon-architecture.md).

## 1. Install Bitloops

Choose one install method:

```bash
curl -fsSL https://bitloops.com/install.sh | bash
```

```bash
brew tap bitloops/tap && brew install bitloops
```

```bash
cargo install bitloops
```

## 2. Start The Daemon

```bash
bitloops start
```

If the default daemon config does not exist yet, `start` creates it at the platform config location, for example `~/.config/bitloops/config.toml` on Linux.

## 3. Initialise A Project

From inside a git repository or subproject:

```bash
bitloops init
```

This creates `.bitloops.local.toml` in the current directory, adds it to `.git/info/exclude`, installs hooks, and runs the initial baseline sync through the daemon.

If you want to pin the supported agent set during bootstrap, pass `--agent <name>`.

## 4. Add Optional Shared Project Policy

If you want shared capture policy in git, create `.bitloops.toml` in the project root:

```toml title=".bitloops.toml"
[capture]
enabled = true
strategy = "manual-commit"

[watch]
watch_debounce_ms = 750
watch_poll_fallback_ms = 2500
```

Keep `.bitloops.local.toml` for local-only overrides.

## 5. Start Or Open Bitloops

Open the dashboard:

```bash
bitloops dashboard
```

Or manage the daemon yourself:

```bash
bitloops start -d
bitloops start --until-stopped
```

## 6. Query And Ingest

Initial project bootstrap already initialises the schema. You can then ingest and query:

```bash
bitloops devql ingest
bitloops devql query "files changed last 7 days"
```

## 7. Check Status

```bash
bitloops status
bitloops checkpoints status --detailed
```

`bitloops status` reports daemon status. `bitloops checkpoints status` reports repo capture status and shows the resolved policy root and fingerprint.

## Toggle Capture Later

```bash
bitloops disable
bitloops enable
```

These commands edit the nearest discovered project policy and leave installed hooks in place.

## Remove Bitloops Later

Use `bitloops disable` when you want hooks and watchers to stay installed but stop capturing.

Use `bitloops uninstall` when you want to remove Bitloops-managed machine artefacts as well:

```bash
bitloops uninstall --full
```
