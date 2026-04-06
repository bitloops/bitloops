# DevQL Getting Started

This guide shows the current daemon-first flow for DevQL.

If you prefer running from source instead of an installed binary, replace `bitloops ...` with `cargo run -- ...`.

## Backend model

DevQL uses the Bitloops daemon plus the shared store backends configured in the global daemon `config.toml`. Repo policy still lives in `.bitloops.toml` and `.bitloops.local.toml`.

Available backends:

- Relational: SQLite by default, optionally Postgres
- Events: DuckDB by default, optionally ClickHouse
- Blob: local filesystem by default, optionally S3 or GCS

Important:

- Default local store paths live under the Bitloops platform data directory, not under `<repo>/.bitloops/stores/...`.
- `bitloops init` bootstraps repo policy and hooks. It can optionally queue an initial current-state sync, but it does not ingest checkpoint history.
- `bitloops devql sync` is queue-based. It returns immediately by default and only follows the task when you pass `--status`.

## 1) Start the daemon

```bash
bitloops start --create-default-config
```

On a fresh machine, this creates the default daemon config and local default store paths. Interactive bootstrap can also ask for telemetry consent unless you pass an explicit telemetry flag.

## 2) Configure stores and semantic provider

Edit the global daemon config (`config.toml`) and set the stores and semantic provider you want. Example:

```toml
[stores.relational]
sqlite_path = "/absolute/path/to/bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = "/absolute/path/to/bitloops/stores/event/events.duckdb"

[stores.blob]
local_path = "/absolute/path/to/bitloops/stores/blob"

[semantic]
provider = "openai_compatible"
model = "gpt-4.1-mini"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1"
```

What this does:

- uses SQLite for relational data
- uses DuckDB for event data
- uses local filesystem blob storage
- keeps all machine-specific backend configuration in the daemon config rather than in the repo

To use ClickHouse for events instead, configure `[stores.events]` with `clickhouse_url` and related settings. To use Postgres for relational data instead, configure `[stores.relational]` with `postgres_dsn`.

## 3) Initialise a repo

From inside the repository or subproject you want DevQL to work against:

```bash
bitloops init --sync=true
```

What this does:

- creates or updates `.bitloops.local.toml`
- adds `.bitloops.local.toml` to `.git/info/exclude`
- installs or reconciles Bitloops git hooks and agent hooks
- queues an initial DevQL current-state sync and waits for it to finish

Other useful variants:

```bash
bitloops init --sync=false
bitloops init --install-default-daemon --sync=true
```

Notes:

- if you omit `--sync` in an interactive terminal, Bitloops asks whether you want to sync the codebase after hook setup
- in non-interactive mode, `bitloops init` requires `--sync=true` or `--sync=false`
- that initial sync only reconciles current workspace state; it does not ingest committed checkpoints or event history

## 4) Check backend connectivity

Run either:

```bash
bitloops --connection-status
```

or:

```bash
bitloops devql connection-status
```

This checks the configured logical backends and exits non-zero if any configured backend is unavailable.

## 5) Explicitly ensure schema (optional)

```bash
bitloops devql init
```

The daemon normally bootstraps the DevQL schema automatically on startup, and queued sync tasks also ensure schema exists before they run. `bitloops devql init` remains useful when you want to force that check explicitly.

## 6) Ingest checkpoint and artefact history

```bash
bitloops devql ingest
```

This reads committed checkpoints from the repo and writes checkpoint/event data plus relational DevQL rows to the configured backends.

Optional flag:

```bash
bitloops devql ingest --max-checkpoints 200
```

Use `--max-checkpoints` when you want to bound ingestion to the newest N checkpoints.

## 7) Sync current workspace state

Use sync when you want `artefacts_current`, `artefact_edges_current`, and related current-state tables reconciled with the present workspace:

```bash
bitloops devql sync
bitloops devql sync --status
```

By default, `bitloops devql sync` queues a sync task and returns immediately after printing the queued task id.

Use `--status` when you want the CLI to hold the terminal open, stream the task state, and print the final summary.

For a read-only drift check:

```bash
bitloops devql sync --validate --status
```

Useful status commands:

```bash
bitloops status
bitloops daemon status
```

These show global sync queue totals, and when you run them inside a repository they also show the active or most recent sync task for that repo.

## 8) Query with DevQL

Checkpoints query:

```bash
bitloops devql query 'repo("bitloops-cli")->checkpoints()->limit(20)'
```

Artefacts query at a ref:

```bash
bitloops devql query 'repo("bitloops-cli")->asOf(ref:"main")->file("bitloops/src/main.rs")->artefacts()->limit(20)'
```

Artefacts changed by a specific agent:

```bash
bitloops devql query 'repo("bitloops-cli")->artefacts(agent:"claude-code")->select(path,canonical_kind,symbol_fqn,start_line,end_line)->limit(50)'
```

Chat history for a specific artefact selection:

```bash
bitloops devql query 'repo("bitloops-cli")->file("index.ts")->artefacts(lines:1..20,kind:"function")->chatHistory()->limit(5)'
```

Raw GraphQL examples:

```bash
bitloops devql query '{ repo(name: "bitloops-cli") { artefacts(first: 5) { edges { node { path symbolFqn canonicalKind } } } } }'
bitloops devql query --graphql --compact '{ health { relational { backend connected } events { backend connected } } }'
```

Notes:

- inputs containing `->` are treated as DevQL DSL and compiled to GraphQL
- inputs without `->` are treated as raw GraphQL by default
- `--graphql` forces raw GraphQL mode explicitly
- `--compact` emits compact JSON

## 9) Optional dashboard

```bash
bitloops dashboard
```

The dashboard uses the same daemon and backend configuration as the CLI.

## 10) Knowledge refs

Knowledge association source and target refs are version-aware:

- `knowledge:<item_id>` resolves to the latest version for that item
- `knowledge:<item_id>:<version_id>` uses the explicit version

Examples:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "knowledge:<target_item_id>"
bitloops devql knowledge associate "knowledge:<source_item_id>:<source_version_id>" --to "knowledge:<target_item_id>"
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "knowledge:<target_item_id>:<target_version_id>"
```
