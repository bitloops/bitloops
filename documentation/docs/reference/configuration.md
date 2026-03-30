---
sidebar_position: 2
title: Configuration
---

# Configuration Reference

Bitloops uses two separate TOML configuration surfaces:

- A global daemon config in the platform config directory.
- A project policy discovered by walking upwards to the nearest `.bitloops.local.toml` or `.bitloops.toml`.

This is a hard break from the older JSON model. There is no automatic migration or legacy fallback. See the [upgrade note](./upgrading-to-the-daemon-architecture.md).

## Global Daemon Config

Bitloops stores daemon configuration at:

- Linux: `${XDG_CONFIG_HOME:-~/.config}/bitloops/config.toml`
- macOS and Windows: the platform-equivalent config directory returned by the OS

`bitloops start` and `bitloops daemon start` use this file.

- Plain `bitloops start` requires the default file to already exist.
- `bitloops start --create-default-config` creates the default file and the matching default local SQLite, DuckDB, and blob-store paths.
- `bitloops init --install-default-daemon` uses that same bootstrap path before continuing project init.
- `--config /path/to/config.toml` uses an explicit daemon config file. If that explicit path is missing, `start` fails instead of creating it.

The daemon config owns:

- Store backends and custom store paths
- Provider credentials
- Dashboard defaults
- Daemon runtime defaults such as `local_dev`, logging, and telemetry

Example:

```toml title="config.toml"
[runtime]
local_dev = false

[telemetry]
enabled = true

[logging]
level = "info"

[stores]
embedding_provider = "local"
embedding_model = "jinaai/jina-embeddings-v2-base-code"
embedding_cache_dir = "/Users/alex/Library/Caches/bitloops/embeddings/models"

[stores.relational]
sqlite_path = "/Users/alex/.local/share/bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = "/Users/alex/.local/share/bitloops/stores/event/events.duckdb"

[stores.blob]
local_path = "/Users/alex/.local/share/bitloops/stores/blob"

[knowledge.providers.github]
token = "${GITHUB_TOKEN}"

[knowledge.providers.atlassian]
site_url = "https://example.atlassian.net"
email = "${ATLASSIAN_EMAIL}"
token = "${ATLASSIAN_TOKEN}"

[semantic]
provider = "openai_compatible"
model = "qwen2.5-coder"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1"

[dashboard]
bundle_dir = "/Users/alex/Library/Caches/bitloops/dashboard/bundle"

[dashboard.local_dashboard]
tls = true
```

### Default Path Categories

Bitloops uses platform app directories by default:

| Category | Linux example | Purpose |
| --- | --- | --- |
| Config | `${XDG_CONFIG_HOME:-~/.config}/bitloops/` | `config.toml` |
| Data | `${XDG_DATA_HOME:-~/.local/share}/bitloops/` | SQLite, DuckDB, blob store |
| Cache | `${XDG_CACHE_HOME:-~/.cache}/bitloops/` | Embedding model downloads, dashboard bundle |
| State | `${XDG_STATE_HOME:-~/.local/state}/bitloops/` | Daemon runtime metadata, supervisor state, hook scratch |

The default repo footprint is now limited to optional policy files at the repo root. Bitloops no longer uses repo-local runtime storage by default.

If you want to remove these platform directories again, use `bitloops uninstall` with explicit targets or `bitloops uninstall --full`.

## Project Policy

`bitloops init` bootstraps the current directory as a Bitloops project by creating or updating `.bitloops.local.toml`, adding it to `.git/info/exclude`, installing hooks, and running the initial baseline sync through the daemon.

The thin CLI and hook layer resolve project policy by walking upwards from the current working directory towards the enclosing `.git` root.

Resolution rules:

- In each directory, check `.bitloops.local.toml` first, then `.bitloops.toml`.
- A standalone `.bitloops.local.toml` is a valid project root.
- If both files exist in the same directory, `.bitloops.toml` is loaded first and `.bitloops.local.toml` overlays it.
- Discovery stops at the first matching directory. Bitloops does not merge policy from multiple ancestors.
- If Bitloops reaches the enclosing `.git` root without finding either file, project-scoped commands tell you to run `bitloops init`.

Project policy controls what the slim CLI and hooks send to the daemon. It does not configure store backends or daemon runtime paths.

Example shared policy:

```toml title=".bitloops.toml"
[capture]
enabled = true
strategy = "manual-commit"

[capture.summarize]
enabled = true

[watch]
watch_debounce_ms = 750
watch_poll_fallback_ms = 2500

[scope]
project_root = "packages/app"
include = ["src/**", "tests/**"]
exclude = ["dist/**", "coverage/**"]

[agents]
default = "claude-code"
allowed = ["claude-code", "cursor", "codex"]
normalise_branches = true

[imports]
knowledge = ["bitloops/knowledge.toml"]
```

Example local project file created by `bitloops init`:

```toml title=".bitloops.local.toml"
[capture]
enabled = true
strategy = "manual-commit"

[agents]
supported = ["claude-code"]
```

Example local override layered on top of a shared project file:

```toml title=".bitloops.local.toml"
[capture]
enabled = false

[watch]
watch_debounce_ms = 1500
```

## Knowledge Imports

Knowledge source references belong in separate TOML files that are imported from repo policy:

```toml title="bitloops/knowledge.toml"
[sources.github]
repositories = ["bitloops/bitloops"]
labels = ["documentation", "devql"]

[sources.atlassian]
spaces = ["ENG", "DOCS"]
projects = ["BIT"]
```

Imported knowledge files:

- Resolve relative to the repo policy file that declares them
- Affect the repo policy fingerprint
- Describe what the thin CLI should reference when talking to the daemon

Provider authentication still belongs in the global daemon config.

## Precedence

Daemon config precedence:

1. Explicit CLI flags such as `bitloops daemon start --bundle-dir`
2. Global daemon config `config.toml`
3. Platform default paths and built-in defaults

Project policy precedence:

1. `.bitloops.local.toml`
2. `.bitloops.toml`
3. No active project policy

Arrays replace lower-precedence arrays. They are not deep-merged.

## What Belongs Where

Use the global daemon config for:

- SQLite, DuckDB, ClickHouse, PostgreSQL, blob, and embedding cache paths
- Provider credentials and service defaults
- Dashboard bundle overrides and TLS hints

Use project policy for:

- Capture enablement and checkpoint strategy
- Watch behaviour
- Monorepo scope rules
- Agent-side policy and knowledge imports

Do not put the following in project policy:

- Store paths
- Dashboard runtime paths
- Provider secrets
- Telemetry settings
- Daemon lifecycle state
