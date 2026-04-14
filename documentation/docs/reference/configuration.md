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
- `bitloops init --install-default-daemon` also auto-applies the default local embeddings setup when embeddings are not configured yet. When that setup targets the default local Bitloops-managed runtime, Bitloops installs the standalone `bitloops-embeddings` binary from the `bitloops/bitloops-embeddings` releases and writes its managed absolute path into the runtime config.
- `--config /path/to/config.toml` uses an explicit daemon config file. If that explicit path is missing, `start` fails instead of creating it.
- `bitloops start --config /path/to/config.toml --bootstrap-local-stores` keeps that explicit config path and creates the matching local SQLite, DuckDB, and blob-store artefacts before startup.
- `bitloops start`, `bitloops init`, and `bitloops enable` all accept `--telemetry`, `--telemetry=false`, and `--no-telemetry` to resolve telemetry consent explicitly.
- `bitloops enable --install-embeddings` and `bitloops daemon enable --install-embeddings` can also update the effective daemon config when they add the default local embeddings profile. When that profile uses the default local Bitloops-managed runtime, Bitloops also installs or updates the managed `bitloops-embeddings` binary.

The daemon config owns:

- Store backends and custom store paths
- Provider credentials
- Inference runtimes, profiles, and capability bindings
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

[semantic_clones]
summary_mode = "auto"
embedding_mode = "semantic_aware_once"
ann_neighbors = 5
enrichment_workers = 1

[semantic_clones.inference]
summary_generation = "summary_llm"
code_embeddings = "local_code"
summary_embeddings = "local_code"

[inference.runtimes.bitloops_inference]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-inference/bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.runtimes.bitloops_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-embeddings/bitloops-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = "bge-m3"

[inference.profiles.summary_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "openai_chat_completions"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1/chat/completions"

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
- `semantic_clones`
- `inference`
- `dashboard`

### Text-Generation Profiles

- `task = "text_generation"` profiles must declare `runtime`.
- Bitloops always routes text generation through the configured runtime, typically `bitloops_inference`.
- `driver` on a text-generation profile is interpreted by `bitloops-inference`, not by Bitloops itself.
- Local summary bootstrap uses Ollama by default when `bitloops init --install-default-daemon` or interactive `bitloops enable` can detect it.

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

Bitloops also keeps repo-scoped workflow runtime state in a dedicated local runtime SQLite database under the active daemon config root.

If you want to remove these platform directories again, use `bitloops uninstall` with explicit targets or `bitloops uninstall --full`.

### Effective Daemon Config For Repo Commands

Repo-scoped commands that need daemon settings resolve the effective daemon config in this order:

1. `BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE`
2. The nearest `config.toml` found by walking upwards from the current repo
3. The default global daemon config

`bitloops enable --install-embeddings`, `bitloops daemon enable --install-embeddings`, and `bitloops init --install-default-daemon` all use that same precedence when deciding which daemon config to read, mutate, and bootstrap against.

That means:

- a repo-local `config.toml` is updated when it is the effective config
- the default global config is only updated when no nearer config applies
- the override environment variable is honoured consistently by both config mutation and runtime bootstrap

### Default Embeddings Enablement

When Bitloops auto-enables embeddings through `bitloops enable --install-embeddings`, interactive `bitloops enable`, or `bitloops init --install-default-daemon`, it creates the minimum daemon config needed for the default local profile only when no active profile is already configured:

```toml
[semantic_clones.inference]
code_embeddings = "local_code"
summary_embeddings = "local_code"

[inference.runtimes.bitloops_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-embeddings/bitloops-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = "bge-m3"
```

Notes:

- `local_code` is the default auto-created local embeddings profile name.
- `bitloops_embeddings_ipc` is the default auto-created local embeddings driver.
- `bitloops_embeddings` is the default auto-created runtime id.
- `bge-m3` is the default auto-created local model.
- When Bitloops installs the managed runtime, it writes an absolute path under the Bitloops data directory, as shown above.
- Use `command = "bitloops-embeddings"` only when you are managing that standalone binary yourself on `PATH`.
- Existing active embedding profiles are preserved. Bitloops does not overwrite an already configured non-local active profile.
- The same runtime warm/bootstrap path used by `bitloops embeddings pull local_code` is reused for local-profile setup.

## RuntimeStore And RelationalStore

Bitloops now uses two internal storage boundaries:

- `RuntimeStore`: local-only SQLite for workflow and daemon runtime state
- `RelationalStore`: the approved relational boundary for queryable checkpoint and DevQL relational state

The runtime store paths are derived by the host and are not configured under `[stores]`:

| Runtime surface | Default path | Purpose |
| --- | --- | --- |
| Daemon runtime store | `<state dir>/daemon/runtime.sqlite` | daemon runtime state, service metadata, supervisor metadata, sync queue state, enrichment queue state |
| Repo runtime store | `<config root>/stores/runtime/runtime.sqlite` | sessions, temporary checkpoints, pre-prompt states, pre-task markers, interaction spool |

Configured relational, events, and blob stores still come from the daemon config:

- `[stores.relational]` selects the `RelationalStore` backend, using SQLite or Postgres
- `[stores.events]` selects the event backend, using DuckDB or ClickHouse
- `[stores.blob]` selects the blob backend, using local disk or a remote object store

## Project Policy

`bitloops init` bootstraps the current directory as a Bitloops project by creating or updating `.bitloops.local.toml`, adding it to `.git/info/exclude`, and installing hooks.

Interactive `bitloops init` can also ask whether you want to install the default local embeddings setup when embeddings are still unconfigured, whether you want to queue an initial DevQL current-state sync after hook setup, and whether you want to run initial commit-history ingest. Use `--sync=true|false` and `--ingest=true|false` when you want to make those choices explicit; non-interactive runs require those flags.

When you use `bitloops init --install-default-daemon`, Bitloops can also auto-apply the default local embeddings setup if embeddings are not already configured. When init also runs sync or ingest, any managed `bitloops-embeddings` download happens afterwards.

`bitloops init` also accepts repeatable repo-policy exclusion flags:

- `--exclude <glob>` adds entries to `[scope].exclude`
- `--exclude-from <path>` adds entries to `[scope].exclude_from`

`--exclude-from` paths must stay under the discovered repo-policy root. Init persists these values to `.bitloops.local.toml` before any init-triggered sync/ingest begins.

Use DevQL commands separately when you want to rerun ingest, sync, or validation after initial setup. `bitloops init` can run both initial sync and initial commit-history ingest when you opt into them.

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
exclude_from = [".gitignore", "config/devql.ignore"]

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

### Scope Exclusions

`[scope]` exclusions are evaluated relative to the repo-policy root:

- `exclude = ["glob/**"]` keeps inline glob patterns in policy
- `exclude_from = ["path/to/ignore-file"]` loads additional patterns from files

`exclude_from` files use one glob per line. Blank lines are ignored. Lines beginning with `#` are comments.

Example:

```toml title=".bitloops.local.toml"
[scope]
exclude = ["dist/**", "coverage/**"]
exclude_from = [".gitignore", "config/devql.ignore"]
```

```text title="config/devql.ignore"
# One glob per line
**/*.generated.ts
**/third_party/**
docs/**
```

Notes:

- `exclude_from` can reference any ignore-pattern file under the repo-policy root (for example `.gitignore` or `config/devql.ignore`)
- paths in `exclude_from` must resolve under that same repo-policy root
- you can list multiple files in `exclude_from`
- missing or unreadable `exclude_from` files fail sync/ingest/watch startup before indexing begins

Merge behavior for exclusions is special:

- if `.bitloops.local.toml` defines either `scope.exclude` or `scope.exclude_from`, local exclusion config replaces shared exclusion config from `.bitloops.toml`
- if local exclusion keys are absent, shared exclusion config applies
- non-exclusion `[scope]` keys keep normal merge behavior

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
- Capability policy plus inference runtimes, profiles, and slot bindings
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
