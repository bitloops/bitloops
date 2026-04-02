---
sidebar_position: 4
title: Upgrade Note
---

# Upgrade Note

Bitloops has moved to a daemon-first architecture. This release intentionally breaks the older repo-local JSON configuration model.

## What Changed

- `bitloops start` now owns default daemon-config bootstrap.
- First-run telemetry consent now belongs to `bitloops start` when that default daemon config is created.
- `bitloops init` now bootstraps project policy and hook installation again.
- `bitloops init` can optionally queue an initial DevQL current-state sync after hook setup.
- `bitloops enable` and `bitloops disable` now toggle capture only.
- `bitloops uninstall` is now the command for machine-wide cleanup.
- `bitloops status` now reports daemon status and sync queue summary.
- `bitloops devql sync` now queues sync tasks by default; use `--status` to follow one to completion.
- Repo/session status moved to `bitloops checkpoints status`.
- `bitloops dashboard` is now a browser launcher instead of the server command.
- The always-on service is now the global user-level service `com.bitloops.daemon`.

## Removed Defaults

The older repo-local defaults are no longer used automatically:

- `config.json`
- `config.local.json`
- `settings.json`
- `settings.local.json`
- `.bitloops/stores/...`
- `.bitloops/embeddings/...`
- `.bitloops/tmp/...`
- `.bitloops/metadata/...`
- `~/.bitloops/dashboard/bundle`

There is no automatic migration and no silent legacy fallback.

## New Configuration Model

Use:

- Global daemon config: `config.toml` in the platform config directory
- Shared repo policy: `.bitloops.toml`
- Local repo override: `.bitloops.local.toml`

Default path categories now follow platform app directories:

- Config directory: daemon config
- Data directory: relational DB, event DB, blob store
- Cache directory: embedding model downloads, dashboard bundle
- State directory: daemon runtime metadata and hook scratch files

## What To Update

1. Move daemon settings, store paths, provider credentials, and dashboard defaults into the global daemon config.
2. Move repo capture policy into `.bitloops.toml`.
3. Move local repo overrides into `.bitloops.local.toml`.
4. Run `bitloops start` interactively on each machine, or `bitloops start --create-default-config --telemetry` in non-interactive setups, to create the default daemon config and default local store files.
5. Answer the telemetry prompt during that first `start`, or pass an explicit telemetry flag.
6. Run `bitloops init --sync=true` or `bitloops init --sync=false` in each repo or subproject to create `.bitloops.local.toml` and install hooks. Use `bitloops init --install-default-daemon` if you want init to bootstrap the default daemon service first.
7. Use `bitloops enable` and `bitloops disable` to toggle capture in project policy.
8. Use `bitloops devql ingest` for checkpoint/history ingestion, and `bitloops devql sync --status` when you want to queue and follow a current-state reconciliation.
9. Use `bitloops uninstall --full` if you need to clear the new platform-directory installation completely.

## Examples

Old shared repo JSON:

```json
{
  "settings": {
    "stores": {
      "relational": {
        "sqlite_path": ".bitloops/stores/relational/relational.db"
      }
    }
  }
}
```

New global daemon TOML:

```toml
[stores.relational]
sqlite_path = "/Users/alex/.local/share/bitloops/stores/relational/relational.db"
```

New shared repo policy TOML:

```toml
[capture]
enabled = true
strategy = "manual-commit"
```
