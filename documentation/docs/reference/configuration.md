---
sidebar_position: 2
title: Configuration
---

# Configuration Reference

Bitloops uses two TOML configuration surfaces:

- A global daemon config in the platform config directory.
- A project policy discovered by walking upwards to the nearest `.bitloops.local.toml` or `.bitloops.toml`.

This is a hard break from the older JSON model. There is no automatic migration or legacy fallback. See the [upgrade note](./upgrading-to-the-daemon-architecture.md).

## Global Daemon Config

Bitloops stores daemon configuration at:

- Linux: `${XDG_CONFIG_HOME:-~/.config}/bitloops/config.toml`
- macOS and Windows: the platform-equivalent config directory returned by the OS

`bitloops start` and `bitloops daemon start` use this file.

- In interactive mode, plain `bitloops start` prompts to create the default file when it is missing.
- `bitloops start --create-default-config` creates the default file and the matching default local SQLite, DuckDB, and blob-store paths.
- `bitloops init --install-default-daemon` uses that same bootstrap path before continuing project init.
- `--config /path/to/config.toml` uses an explicit daemon config file. If that explicit path is missing, `start` fails instead of creating it.
- `bitloops start --config /path/to/config.toml --bootstrap-local-stores` keeps that explicit config path and creates the matching local SQLite, DuckDB, and blob-store artefacts before startup.
- `bitloops start`, `bitloops init`, and `bitloops enable` all accept `--telemetry`, `--telemetry=false`, and `--no-telemetry` to resolve telemetry consent explicitly.

The daemon config owns:

- Store backends and custom store paths
- Provider credentials
- Semantic and embeddings runtime settings
- Dashboard defaults
- Daemon runtime defaults such as `local_dev`, logging, and telemetry

Example:

```toml title="config.toml"
[runtime]
local_dev = false
cli_version = "1.2.3"

[telemetry]
enabled = true

[logging]
level = "info"

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

[semantic_clones]
summary_mode = "auto"
embedding_mode = "semantic_aware_once"
embedding_profile = "local-code"

[embeddings.runtime]
command = "bitloops-embeddings"
startup_timeout_secs = 10
request_timeout_secs = 60

[embeddings.profiles.local-code]
kind = "local_fastembed"
model = "jinaai/jina-embeddings-v2-base-code"

[dashboard]
bundle_dir = "/Users/alex/Library/Caches/bitloops/dashboard/bundle"

[dashboard.local_dashboard]
tls = true
```

### Accepted Top-Level Daemon Sections

The current daemon parser accepts these top-level surfaces:

- `runtime`
- `telemetry`
- `logging`
- `stores`
- `knowledge`
- `semantic`
- `semantic_clones`
- `embeddings`
- `dashboard`

### Telemetry Consent

Telemetry consent is stored in the global daemon config.

- `[telemetry].enabled = true` means telemetry is enabled.
- `[telemetry].enabled = false` means the current CLI version was explicitly opted out.
- If `[telemetry].enabled` is absent, consent is unresolved and interactive commands may prompt.
- `[runtime].cli_version` stores the CLI version that most recently reconciled telemetry consent.
- When a newer CLI version starts and the stored value is `false`, Bitloops clears the stored opt-out and asks again on a later interactive `init` or `enable`.
- A stored opt-in (`true`) carries forward across CLI upgrades.
- First-run consent is asked during `bitloops start` when the default daemon config is being created.

### Default Path Categories

Bitloops uses platform app directories by default:

| Category | Linux example | Purpose |
| --- | --- | --- |
| Config | `${XDG_CONFIG_HOME:-~/.config}/bitloops/` | `config.toml` |
| Data | `${XDG_DATA_HOME:-~/.local/share}/bitloops/` | SQLite, DuckDB, blob store |
| Cache | `${XDG_CACHE_HOME:-~/.cache}/bitloops/` | Embedding model downloads, dashboard bundle |
| State | `${XDG_STATE_HOME:-~/.local/state}/bitloops/` | Daemon runtime metadata, supervisor state, daemon runtime SQLite, hook scratch |

Bitloops also keeps repo-scoped workflow runtime state in a dedicated local runtime SQLite database derived from the repository root.

If you want to remove these platform directories again, use `bitloops uninstall` with explicit targets or `bitloops uninstall --full`.

## RuntimeStore And RelationalStore

Bitloops now uses two internal storage boundaries:

- `RuntimeStore`: local-only SQLite for workflow and daemon runtime state
- `RelationalStore`: the approved relational boundary for queryable checkpoint and DevQL relational state

The runtime store paths are derived by the host and are not configured under `[stores]`:

| Runtime surface | Default path | Purpose |
| --- | --- | --- |
| Daemon runtime store | `<state dir>/daemon/runtime.sqlite` | daemon runtime state, service metadata, supervisor metadata, sync queue state, enrichment queue state |
| Repo runtime store | derived from the repository root | sessions, temporary checkpoints, pre-prompt states, pre-task markers, interaction spool |

Configured relational, events, and blob stores still come from the daemon config:

- `[stores.relational]` selects the `RelationalStore` backend, using SQLite or Postgres
- `[stores.events]` selects the event backend, using DuckDB or ClickHouse
- `[stores.blob]` selects the blob backend, using local disk or a remote object store

## Project Policy

`bitloops init` bootstraps the current directory as a Bitloops project by creating or updating `.bitloops.local.toml`, adding it to `.git/info/exclude`, and installing hooks.

Interactive `bitloops init` can also ask whether you want to queue an initial DevQL current-state sync after hook setup. Use `--sync=true` or `--sync=false` when you want to make that choice explicit; non-interactive runs require one of those flags.

Use DevQL commands separately for ingestion and for any later explicit sync or validation runs. `bitloops init` does not perform DevQL ingest.

The thin CLI and hook layer resolve project policy by walking upwards from the current working directory towards the enclosing `.git` root.

Resolution rules:

- In each directory, check `.bitloops.local.toml` first, then `.bitloops.toml`.
- A standalone `.bitloops.local.toml` is a valid project root.
- If both files exist in the same directory, `.bitloops.toml` is loaded first and `.bitloops.local.toml` overlays it.
- Discovery stops at the first matching directory. Bitloops does not merge policy from multiple ancestors.
- If Bitloops reaches the enclosing `.git` root without finding either file, project-scoped commands tell you to run `bitloops init`.

Project policy controls what the slim CLI and hooks send to the daemon. It does not configure store backends or daemon runtime paths.

### Accepted Top-Level Repo-Policy Sections

The current repo-policy surface is:

- `capture`
- `watch`
- `scope`
- `agents`
- `imports`

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

- SQLite, DuckDB, ClickHouse, PostgreSQL, and blob paths
- Provider credentials and service defaults
- Semantic summary settings, semantic clone settings, and embeddings runtime profiles
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
