---
sidebar_position: 1
title: Common Issues
---

# Common Issues

## Hooks Are Not Firing

Checks:

1. Run `bitloops init` in the repository or subproject you want to capture.
2. Verify the repo is still a git repository.
3. Run `bitloops checkpoints status --detailed` to confirm the effective capture policy.
4. If capture is disabled, re-enable it with `bitloops enable`.

## The Dashboard Does Not Open

Checks:

1. Run `bitloops status`.
2. If needed, start the daemon manually with `bitloops start`.
3. Re-run `bitloops dashboard`.
4. If you use local HTTPS, try `bitloops daemon start --recheck-local-dashboard-net`.
5. Inspect the daemon log with `bitloops daemon logs` or print its location with `bitloops daemon logs --path`.

## DevQL Cannot Reach Storage

Checks:

1. Run `bitloops --connection-status`.
2. Confirm the store paths or remote DSNs in the global daemon config.
3. Re-run `bitloops devql init` if the stores were recreated.

## Daemon Config Is Missing

Bitloops starts with either an explicit daemon config path or the default global config location.

If the default global config does not exist yet:

1. Run `bitloops start --create-default-config`, or
2. Run interactive `bitloops start` and accept the prompt to create the default config.

In non-interactive environments, Bitloops does not create the default config unless you pass `--create-default-config`.

If you already have an explicit config file and only need Bitloops to create the matching local file-backed stores, run:

```bash
bitloops start --config /path/to/config.toml --bootstrap-local-stores
```

That creates the local SQLite, DuckDB, and blob-store artefacts referenced by that config before startup.

## Detached Start Times Out With An Explicit Config

Checks:

1. Confirm the config file itself already exists.
2. Run `bitloops start --config /path/to/config.toml --bootstrap-local-stores ...` so the local relational, event, and blob artefacts exist before startup.
3. Inspect the daemon log with `bitloops daemon logs --tail 200`.
4. If the config lives inside a larger Git repo, make sure daemon startup is actually resolving store backends from that explicit config path rather than from a repo-root daemon config.

## Capture Seems Disabled Unexpectedly

Checks:

1. Inspect `.bitloops.toml`.
2. Inspect `.bitloops.local.toml`.
3. Run `bitloops checkpoints status --detailed` to confirm the active policy root and fingerprint.
4. Re-enable capture with `bitloops enable` if `[capture].enabled = false`.
