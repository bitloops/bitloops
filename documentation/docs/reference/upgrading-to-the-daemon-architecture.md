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

The older repo-local defaults are no longer used automatically for configured relational, event, blob, cache, and scratch paths:

- `config.json`
- `config.local.json`
- `settings.json`
- `settings.local.json`
- `.bitloops/stores/relational/...`
- `.bitloops/stores/event/...`
- `.bitloops/stores/blob/...`
- `.bitloops/embeddings/...`
- `.bitloops/tmp/...`
- `.bitloops/metadata/...`
- `~/.bitloops/dashboard/bundle`

There is no automatic migration and no silent legacy fallback.

Bitloops does still keep repo-scoped workflow runtime state locally in `<config root>/stores/runtime/runtime.sqlite`.

## New Configuration Model

Use:

- Global daemon config: `config.toml` in the platform config directory
- Shared repo policy: `.bitloops.toml`
- Local repo override: `.bitloops.local.toml`

Default path categories now follow platform app directories:

- Config directory: daemon config
- Data directory: relational DB, event DB, blob store
- Cache directory: embedding model downloads, dashboard bundle
- State directory: daemon runtime metadata, daemon runtime SQLite, and hook scratch files
- Repo runtime directory: `<config root>/stores/runtime/runtime.sqlite`

## What To Update

1. Move daemon settings, store paths, provider credentials, and dashboard defaults into the global daemon config.
2. Move repo capture policy into `.bitloops.toml`.
3. Move local repo overrides into `.bitloops.local.toml`.
4. Run `bitloops start` interactively on each machine, or `bitloops start --create-default-config --telemetry` in non-interactive setups, to create the default daemon config and default local store files.
5. If you use an explicit repo-scoped or test config, run `bitloops start --config /path/to/config.toml --bootstrap-local-stores` to create the matching local file-backed stores before the first start.
6. Answer the telemetry prompt during that first `start`, or pass an explicit telemetry flag.
7. Run `bitloops init --sync=true` or `bitloops init --sync=false` in each repo or subproject to create `.bitloops.local.toml` and install hooks. Use `bitloops init --install-default-daemon` if you want init to bootstrap the default daemon service first; that path also auto-configures the default local embeddings profile when embeddings are not already configured.
8. Use `bitloops enable` and `bitloops disable` to toggle capture in project policy. Use `bitloops enable --install-embeddings` or `bitloops daemon enable --install-embeddings` when you also want the default local embeddings profile set up in the effective daemon config.
9. Use `bitloops devql ingest` for checkpoint/history ingestion, and `bitloops devql sync --status` when you want to queue and follow a current-state reconciliation.
10. Use `bitloops uninstall --full` if you need to clear the new platform-directory installation completely.

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
