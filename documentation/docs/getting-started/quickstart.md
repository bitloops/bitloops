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

## 2. Initialise The Daemon Config

```bash
bitloops init
```

This creates the global daemon config at the platform config location, for example `~/.config/bitloops/config.toml` on Linux.

## 3. Enable A Repository

From inside a git repository:

```bash
bitloops enable
```

This installs git hooks and supported agent hooks for that repository.

## 4. Add Optional Repo Policy

Create `.bitloops.toml` at the repo root if you want shared capture policy:

```toml title=".bitloops.toml"
[capture]
enabled = true
strategy = "manual-commit"

[watch]
watch_debounce_ms = 750
watch_poll_fallback_ms = 2500
```

Use `.bitloops.local.toml` for local-only overrides.

## 5. Start Or Open Bitloops

Open the dashboard:

```bash
bitloops dashboard
```

Or start the daemon yourself:

```bash
bitloops start
bitloops start -d
bitloops start --until-stopped
```

## 6. Initialise DevQL Storage

```bash
bitloops devql init
```

Then ingest and query:

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

## Remove Bitloops Later

Use `bitloops disable` to remove hooks from the current repository.

Use `bitloops uninstall` when you want to remove Bitloops-managed machine artefacts as well:

```bash
bitloops uninstall --full
```
