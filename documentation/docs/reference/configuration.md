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

`bitloops configure`, `bitloops start`, and `bitloops daemon start` use this file.

- `bitloops configure --web` creates the default file and local stores when needed, starts or reuses the daemon, and opens the dashboard configuration page.
- `bitloops configure --file <path>` validates a complete daemon `config.toml`, installs it at the default daemon config path, ensures local stores exist, then restarts or starts the daemon.
- `bitloops configure --default-config` installs the generated default daemon config directly and starts or reuses the daemon.
- Installers support a default-config flag for scripted setup. On macOS, Linux, and WSL pass `--default-config` to `install.sh`; on Windows PowerShell pass `-DefaultConfig`; on Windows CMD pass `--default-config` or `/default-config`. The installers write the generated default daemon TOML and run `bitloops configure --file`.
- The manual alternative is to install without the default-config flag, run `bitloops configure --web`, then run `bitloops init` inside each repo.
- In interactive mode, plain `bitloops start` prompts to create the default file when it is missing.
- `bitloops start --create-default-config` creates the default file and the matching default local SQLite, DuckDB, and blob-store paths.
- `bitloops embeddings install --runtime platform` installs the managed `bitloops-platform-embeddings` runtime and writes the hosted runtime args into the daemon config. Add `--gateway-url https://gateway.example/v1/embeddings` only when you want an explicit gateway override.
- `--config /path/to/config.toml` uses an explicit daemon config file. If that explicit path is missing, `start` fails instead of creating it.
- `bitloops start --config /path/to/config.toml --bootstrap-local-stores` keeps that explicit config path and creates the matching local SQLite, DuckDB, and blob-store artefacts before startup.
- `bitloops start` and `bitloops enable` accept `--telemetry`, `--telemetry=false`, and `--no-telemetry` to resolve telemetry consent explicitly.
- `bitloops enable --install-embeddings` and `bitloops daemon enable --install-embeddings` can also update the effective daemon config when they add the default local embeddings profile. When that profile uses the default local Bitloops-managed runtime, Bitloops also installs or updates the managed `bitloops-local-embeddings` binary.
- `bitloops enable --install-embeddings --embeddings-runtime platform` follows the hosted platform path instead. Add `--embeddings-gateway-url https://gateway.example/v1/embeddings` or set `BITLOOPS_PLATFORM_GATEWAY_URL` only when you want to override the platform default. The bearer token environment variable defaults to `BITLOOPS_PLATFORM_GATEWAY_TOKEN` and can be overridden with `--embeddings-api-key-env`.
- `bitloops enable --capture --install-context-guidance --context-guidance-runtime platform` configures hosted context guidance text generation. Add `--context-guidance-gateway-url https://gateway.example/v1/chat/completions` only when you want an explicit chat completions endpoint override. The bearer token environment variable defaults to `BITLOOPS_PLATFORM_GATEWAY_TOKEN` and can be overridden with `--context-guidance-api-key-env`.

The daemon config owns:

- Store backends and custom store paths
- Provider credentials
- Inference runtimes and profiles
- Daemon-owned capability bindings such as semantic summary generation
- Dashboard defaults
- Daemon runtime defaults such as `local_dev`, logging, and telemetry

Repo semantic embedding intent is project policy, not daemon policy. The daemon may define a `local_code` or `platform_code` profile, but each repo opts into or out of using that profile in `.bitloops.local.toml` or `.bitloops.toml`.

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
ann_neighbors = 5
enrichment_workers = 1

[semantic_clones.inference]
summary_generation = "summary_llm"

[context_guidance.inference]
guidance_generation = "guidance_llm"

[inference.runtimes.bitloops_inference]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-inference/bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.runtimes.bitloops_local_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-local-embeddings/bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"

[inference.profiles.summary_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "openai_chat_completions"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200

[inference.profiles.guidance_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "bitloops_platform_chat"
model = "ministral-3-3b-instruct"
api_key = "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}"
temperature = "0.1"
max_output_tokens = 4096

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
- `context_guidance`
- `architecture`
- `inference`
- `dashboard`

### CLI Auth

Bitloops CLI auth uses WorkOS AuthKit’s device flow.

Notes:

- CLI auth is not configured through `config.toml`.
- `bitloops login` works out of the box with the built-in WorkOS client id.
- `BITLOOPS_WORKOS_CLIENT_ID` overrides that built-in client id when you need a non-default WorkOS application.
- `BITLOOPS_WORKOS_BASE_URL` overrides the default `https://api.workos.com` base URL when you need a non-default WorkOS environment.
- Tokens are stored in the platform secure credential store, not in `config.toml`.
- Session metadata is stored in the daemon runtime store under the platform state directory.

### Text-Generation Profiles

- `task = "text_generation"` profiles must declare `runtime`.
- `task = "text_generation"` profiles must also declare `temperature` and `max_output_tokens`.
- Bitloops always routes text generation through the configured runtime, typically `bitloops_inference`.
- `driver` on a text-generation profile is interpreted by `bitloops-inference`, not by Bitloops itself.
- Local summary bootstrap uses Ollama by default when interactive `bitloops enable` can detect it, and writes `base_url = "http://127.0.0.1:11434/api/chat"`.
- Local context guidance setup uses the same Ollama chat profile shape and writes `max_output_tokens = 4096`.
- `thinking_level` is an optional profile property for local CLI-agent drivers only. Bitloops preserves the configured value and includes it in runtime identity; when absent, Bitloops leaves it unset and does not synthesize a default.

### Structured-Generation Profiles

`task = "structured_generation"` profiles use the same generic `[inference.profiles.<name>]` profile shape as text generation. The `thinking_level` property is optional on any profile, but only local CLI-agent drivers use it.

For local CLI-agent drivers, Bitloops launches the managed `bitloops_inference` runtime, and `bitloops-inference` launches the selected agent runtime from the profile. Configure these profiles manually when you need structured generation backed by a local CLI agent.

Supported `thinking_level` values:

- `codex_exec`: `low`, `medium`, `high`, `extra_high`, `xhigh`. `extra_high` and `xhigh` both run Codex with `model_reasoning_effort = "xhigh"`.
- `claude_code_print`: `low`, `medium`, `high`, `xhigh`, `max`.

When `thinking_level` is absent, Bitloops sends no default and emits no warning. The local inference runtime and driver decide their own default behavior.

The `architecture_graph` capability currently exposes two structured-generation slots:

- `[architecture.inference].fact_synthesis`: optional profile for architecture graph synthesis and architecture role seed generation.
- `[architecture.inference].role_adjudication`: optional profile for queued architecture role adjudication.

Example Codex-backed role adjudication profile:

```toml
[architecture.inference]
role_adjudication = "architecture_role_adjudication_codex"

[inference.runtimes.bitloops_inference]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-inference/bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.runtimes.codex]
command = "codex"
args = ["--ask-for-approval", "never"]
startup_timeout_secs = 5
request_timeout_secs = 900

[inference.profiles.architecture_role_adjudication_codex]
task = "structured_generation"
driver = "codex_exec"
runtime = "codex"
model = "gpt-5.4-mini"
temperature = "0.1"
max_output_tokens = 1024
thinking_level = "high"
```

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

`bitloops enable --install-embeddings`, `bitloops daemon enable --install-embeddings`, and `bitloops configure --file` all use daemon config paths explicitly when deciding which daemon config to read, mutate, and bootstrap against.

That means:

- a repo-local `config.toml` is updated when it is the effective config
- the default global config is only updated when no nearer config applies
- the override environment variable is honoured consistently by both config mutation and runtime bootstrap

### Default Embeddings Enablement

When Bitloops auto-enables the default local embeddings profile through `bitloops enable --install-embeddings`, interactive `bitloops enable`, or `bitloops embeddings install`, it creates the minimum daemon config needed for that local profile and writes the repo opt-in to project policy:

```toml
[inference.runtimes.bitloops_local_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-local-embeddings/bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"
```

```toml title=".bitloops.local.toml"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "local_code"
summary_embeddings = "local_code"
```

Notes:

- `local_code` is the default auto-created local embeddings profile name.
- `bitloops_embeddings_ipc` is the default auto-created local embeddings driver.
- `bitloops_local_embeddings` is the default auto-created runtime id.
- `bge-m3` is the default auto-created local model.
- When Bitloops installs the managed runtime, it writes an absolute path under the Bitloops data directory, as shown above.
- Use `command = "bitloops-local-embeddings"` only when you are managing that standalone binary yourself on `PATH`.
- Existing legacy daemon embedding bindings are preserved by migrating their profile names into repo policy; new installs do not write repo opt-in into daemon config.
- The same runtime warm/bootstrap path used by `bitloops embeddings pull local_code` is reused for local-profile setup.

### Platform Embeddings Enablement

When you select the hosted gateway path, Bitloops writes a separate managed runtime/profile and repo opt-in:

```toml
[inference.runtimes.bitloops_platform_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-platform-embeddings/bitloops-platform-embeddings"
args = ["--gateway-url", "https://gateway.example/v1/embeddings", "--api-key-env", "BITLOOPS_PLATFORM_GATEWAY_TOKEN"]
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.platform_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_platform_embeddings"
model = "bge-m3"
```

```toml title=".bitloops.local.toml"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "platform_code"
summary_embeddings = "platform_code"
```

Notes:

- `bitloops_platform_embeddings` is the hosted runtime id.
- The managed platform runtime never downloads a local model bundle.
- Hosted gateway credentials stay in runtime args and the referenced environment variable, not in the profile itself.

### Context Guidance Generation

When you configure hosted context guidance, Bitloops writes a text-generation binding and profile:

```toml
[context_guidance.inference]
guidance_generation = "guidance_llm"

[inference.runtimes.bitloops_inference]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-inference/bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.guidance_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "bitloops_platform_chat"
model = "ministral-3-3b-instruct"
api_key = "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}"
temperature = "0.1"
max_output_tokens = 4096
```

Local context guidance uses Ollama through the same `bitloops_inference` runtime:

```toml
[context_guidance.inference]
guidance_generation = "guidance_local"

[inference.runtimes.bitloops_inference]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-inference/bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.guidance_local]
task = "text_generation"
runtime = "bitloops_inference"
driver = "ollama_chat"
model = "ministral-3:3b"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 4096
```

You can also override the active guidance generation profile with `BITLOOPS_CONTEXT_GUIDANCE_GUIDANCE_GENERATION`.

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

The effective storage authority is split by data family:

| Data family | Authority | Backend selection |
| --- | --- | --- |
| `runtime` | workspace-local | always SQLite |
| `relational current` | workspace-local | always SQLite |
| `relational shared` | workspace-local or shared | SQLite by default, Postgres when `[stores.relational].postgres_dsn` is configured |
| `events` | workspace-local or shared | DuckDB by default, ClickHouse when `[stores.events].clickhouse_url` is configured |
| `blob runtime/session` | workspace-local | always local disk |
| `blob project/knowledge` | workspace-local or shared | local disk by default, S3 or GCS when `[stores.blob]` is configured for a remote object store |

Notes:

- Runtime/session state always stays workspace-local in runtime SQLite.
- Relational `*_current` and other current/projection tables always stay workspace-local in SQLite, even when Postgres is configured.
- Shared relational historical tables use Postgres when `[stores.relational].postgres_dsn` is configured; otherwise they stay local in SQLite.
- Canonical interaction event rows use the selected event backend from `[stores.events]`. The local interaction spool is runtime-local staging, not a second canonical event store.
- Runtime/session blob payloads always stay workspace-local on disk.
- Project/knowledge blob payloads follow `[stores.blob]`, using local disk when no remote object store is configured and S3 or GCS when one is configured.

### Multi-Workspace Behavior

Bitloops treats each workspace or worktree as having its own local runtime and current-projection state:

- Local runtime SQLite and local current/projection relational state are derived from the active config root and workspace path, so one worktree’s current state does not overwrite another worktree’s local current state.
- Shared historical relational data, canonical event data, and project/knowledge blob payloads may point at shared remote backends when configured.
- This means different worktrees can keep divergent local runtime and `*_current` views while still sharing the same historical or project-level backing stores.
- The detailed `bitloops status` view and the GraphQL `health` surface report this split explicitly so you can see which families stay local and which resolve to shared infrastructure.

## Project Policy

`bitloops init` bootstraps the current directory as a Bitloops project by creating or updating `.bitloops.local.toml`, adding it to `.git/info/exclude`, and installing hooks.

`bitloops enable` and `bitloops disable` now operate on two repo-scoped targets:

- `Capture`, which toggles `[capture].enabled`
- `DevQL Guidance`, which toggles `[agents].devql_guidance_enabled` and the managed repo-local DevQL guidance surfaces

Interactive `bitloops init` asks whether you want to queue an initial DevQL current-state sync after hook setup and whether you want to run initial commit-history ingest. Use `--sync=true|false` and `--ingest=true|false` when you want to make those choices explicit; non-interactive runs require those flags.

Daemon-only settings such as telemetry, inference profiles, capability packs, context guidance, semantic clone defaults, store backends, logging, and dashboard options belong to `bitloops configure`, not `bitloops init`.

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
- `semantic_clones`
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
devql_guidance_enabled = true

[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "local_code"
summary_embeddings = "local_code"
```

Example local override layered on top of a shared project file:

```toml title=".bitloops.local.toml"
[capture]
enabled = false

[watch]
watch_debounce_ms = 1500

[agents]
devql_guidance_enabled = false
```

Notes:

- `devql_guidance_enabled` defaults to `true` when the key is omitted.
- `bitloops init --disable-devql-guidance` writes `devql_guidance_enabled = false` and skips installing repo-local DevQL guidance surfaces.
- `bitloops enable --devql-guidance` reinstalls those repo-local DevQL guidance surfaces without changing capture state.
- `bitloops disable --devql-guidance` removes those repo-local DevQL guidance surfaces without changing capture state.
- `[semantic_clones].embedding_mode = "off"` disables repo code embeddings, identity embeddings, summary embeddings, and clone rebuild work even when the daemon defines embedding profiles.
- When `[semantic_clones]` is present, `code_embeddings` and `summary_embeddings` are repo-owned profile bindings. The named profiles must still be defined under `[inference.profiles]` in the effective daemon config.

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
