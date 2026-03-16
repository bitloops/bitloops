# DevQL Getting Started

This guide shows the minimum commands to get DevQL working in the CLI.

DevQL now uses the shared Bitloops store backends. These stores also hold checkpoint and session runtime data, so they are not DevQL-only.

## Provider model

Backend configuration is defined in `<repo>/.bitloops/config.json` under `stores`:

- `stores.relational.provider`: `sqlite` | `postgres`
- `stores.event.provider`: `duckdb` | `clickhouse`
- `stores.blob.provider`: `local` | `s3` | `gcs`

Current runtime adapters are:

- Relational: `sqlite` (default) and `postgres`
- Events: `duckdb` (default) and `clickhouse`
- Blob: `local` (default), `s3`, and `gcs`

Important:
- The configuration shape is non-backwards-compatible with legacy `devql.*` keys.
- Store paths default to repo-local `.bitloops/stores/*` locations.
- Database files are expected to be created during `bitloops init`.

Current command support matrix:

- `devql connection-status` and dashboard DB health checks support all configured providers.
- `devql init` supports configured providers, including defaults (`relational.provider=sqlite`, `event.provider=duckdb`).
- `devql ingest` supports configured events and relational providers (`duckdb`/`clickhouse` + `sqlite`/`postgres`).
- `devql query` supports:
  - `checkpoints()`/`telemetry()` on `event.provider=duckdb` or `event.provider=clickhouse`
  - `artefacts()`/`deps()`/`chatHistory()` on `relational.provider=sqlite` or `relational.provider=postgres`

## 0) (Optional) Run Postgres and ClickHouse with Docker

```bash
docker run -d --name mypostgres -p 5432:5432 \
  -e POSTGRES_USER=bitloops \
  -e POSTGRES_PASSWORD=bitloops \
  -e POSTGRES_DB=bitloops \
  postgres
```

## 1) Configure stores and semantic provider

Create `<repo>/.bitloops/config.json`:

```json
{
  "stores": {
    "relational": {
      "provider": "sqlite",
      "sqlite_path": ".bitloops/stores/relational/relational.db"
    },
    "event": {
      "provider": "duckdb",
      "duckdb_path": ".bitloops/stores/event/events.duckdb"
    },
    "blob": {
      "provider": "local",
      "local_path": ".bitloops/stores/blob"
    }
  },
  "semantic": {
    "provider": "openai",
    "model": "gpt-4.1-mini",
    "api_key": "YOUR_KEY"
  }
}
```

What this does:
- Uses `sqlite` for relational data and `duckdb` for event data.
- Uses local filesystem blob storage.
- Uses repo-local paths for all stores.

If you omit file paths:
- SQLite defaults to `<repo>/.bitloops/stores/relational/relational.db`
- DuckDB defaults to `<repo>/.bitloops/stores/event/events.duckdb`
- Local blob store defaults to `<repo>/.bitloops/stores/blob`

To use ClickHouse for events instead:

```json
{
  "stores": {
    "event": {
      "provider": "clickhouse",
      "clickhouse_url": "http://localhost:8123",
      "clickhouse_database": "bitloops",
      "clickhouse_user": "bitloops",
      "clickhouse_password": "bitloops"
    }
  }
}
```

## 2) Initialise stores in the repo

Run:

```bash
cargo run -- init --agent claude-code
```

What this does:
- Creates and initialises local store files/directories for configured providers.
- For default local stores, this creates:
  - `.bitloops/stores/relational/relational.db`
  - `.bitloops/stores/event/events.duckdb`
  - `.bitloops/stores/blob`

If store DB files are missing later, runtime commands fail with an error instructing you to run `bitloops init`.

## 3) Check backend connectivity

Run either:

```bash
cargo run -- --connection-status
```

or:

```bash
cargo run -- devql connection-status
```

What this does:
- Runs a connectivity check against configured logical backends.
- Prints a `DB Status` table with statuses:
  - `Connected`
  - `Could not authenticate`
  - `Could not reach DB`
  - `Not configured`
- Exits non-zero if any configured backend fails.

Example output:

```text

+------------+-----------------------+
| DB         | Status                |
+------------+-----------------------+
| Relational | Connected             |
| Events     | Could not reach DB    |
+------------+-----------------------+
```

## 4) Initialise DevQL schema

```bash
cargo run -- devql init
```

What this does:
- Creates DevQL schema for configured providers.
- With defaults, this initialises SQLite relational DevQL tables and DuckDB `checkpoint_events` table.

## 5) Ingest checkpoint + artefact data

```bash
cargo run -- devql ingest
```

What this does:
- Reads committed checkpoints from the repo.
- Writes checkpoint events to the configured event backend (`duckdb` by default, or `clickhouse`).
- Writes repository/commit/file/artefact rows to the configured relational backend (`sqlite` by default, or `postgres`).

Optional flags:

```bash
cargo run -- devql ingest --init=false --max-checkpoints 200
```

- `--init=false`: skip schema bootstrap step.
- `--max-checkpoints N`: limit how many checkpoints are ingested.

## 6) Query with DevQL

Checkpoints query:

```bash
cargo run -- devql query 'repo("bitloops-cli")->checkpoints()->limit(20)'
```

Artefacts query at a ref:

```bash
cargo run -- devql query 'repo("bitloops-cli")->asOf(ref:"main")->file("bitloops_cli/src/main.rs")->artefacts()->limit(20)'
```

Artefacts changed by a specific agent:

```bash
cargo run -- devql query 'repo("bitloops-cli")->artefacts(agent:"claude-code")->select(path,canonical_kind,symbol_fqn,start_line,end_line)->limit(50)'
```

Same, constrained to recent events:

```bash
cargo run -- devql query 'repo("bitloops-cli")->artefacts(agent:"claude-code",since:"2026-03-01")->select(path,canonical_kind,symbol_fqn,start_line,end_line)->limit(50)'
```

Chat history for a specific artefact selection:

```bash
cargo run -- devql query 'repo("bitloops-cli")->file("index.ts")->artefacts(lines:1..20,kind:"function")->chatHistory()->limit(5)'
```

Chat histories for all artefacts in a file line range:

```bash
cargo run -- devql query 'repo("bitloops-cli")->file("index.ts")->artefacts(lines:1..200)->chatHistory()->select(path,symbol_fqn,start_line,end_line,chat_history)->limit(50)'
```

What this does:
- Parses the DevQL pipeline.
- Routes checkpoint/telemetry stages to configured event backend (`duckdb` or `clickhouse`).
- Routes artefact stages to configured relational backend (`sqlite` or `postgres`).
- `chatHistory()` enriches artefact rows with related checkpoint/session chat context.
- Prints JSON output.

## 7) (Optional) Use dashboard with DB startup health

```bash
cargo run -- dashboard --no-open
```

What this does:
- On startup, checks DB health for configured backends.
- Uses the same `DB Status` table/status semantics as `--connection-status`.
- Keeps live health checks for configured adapters while dashboard runs.
- Exposes live health at:

```text
http://127.0.0.1:5667/api/db/health
```

## Installed binary equivalents

If `bitloops` is already installed, replace `cargo run -- ...` with:

```bash
bitloops --connection-status
bitloops init --agent claude-code
bitloops devql init
bitloops devql ingest
bitloops devql query 'repo("bitloops-cli")->checkpoints()->limit(20)'
bitloops dashboard --no-open
```
