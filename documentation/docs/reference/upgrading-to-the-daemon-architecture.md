---
sidebar_position: 4
title: Upgrade Note
---

# Upgrade Note

Bitloops has moved to a daemon-first architecture. This release intentionally breaks the older repo-local JSON configuration model.

## What Changed

- `bitloops init` now prepares the global daemon config.
- `bitloops enable` and `bitloops disable` now manage hooks only.
- `bitloops status` now reports daemon status.
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
4. Re-run `bitloops init` to create the daemon config if needed.
5. Re-run `bitloops enable` in each repo to install hooks under the new model.

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
