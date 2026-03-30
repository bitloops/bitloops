---
sidebar_position: 1
title: Using the Dashboard
---

# Using the Dashboard

`bitloops dashboard` is now a launcher. It opens the dashboard in your browser and starts the daemon if required.

## Open The Dashboard

```bash
bitloops dashboard
```

Behaviour:

- If the daemon is already serving, Bitloops opens the current dashboard URL.
- If the global service exists, Bitloops starts or reuses it.
- Otherwise Bitloops prompts for foreground, detached, or always-on mode.

## Start The Daemon Yourself

```bash
bitloops start
bitloops start -d
bitloops start --until-stopped
```

The always-on mode installs or refreshes the global user service `com.bitloops.daemon`.

## Useful Commands

```bash
bitloops status
bitloops stop
bitloops restart
```

## Default Bundle Location

The dashboard bundle is treated as cache:

- Linux example: `${XDG_CACHE_HOME:-~/.cache}/bitloops/dashboard/bundle`
- macOS and Windows: the platform-equivalent cache location

Override it temporarily:

```bash
bitloops daemon start --bundle-dir /path/to/bundle
```

Or persist it in the daemon config:

```toml
[dashboard]
bundle_dir = "/path/to/bundle"
```

## HTTPS Hints

Local HTTPS hints also belong in the daemon config:

```toml
[dashboard.local_dashboard]
tls = true
```

Use `bitloops daemon start --recheck-local-dashboard-net` if you need Bitloops to re-evaluate the local TLS setup.
