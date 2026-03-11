# DevQL Getting Started

This guide shows the minimum commands to get DevQL working in the CLI.

## Provider model (foundation contract)

DevQL backend configuration is now split into logical providers:

- `devql.relational.provider`: `sqlite` | `postgres`
- `devql.events.provider`: `duckdb` | `clickhouse`

Current runtime adapters are `postgres` + `duckdb` (default events backend). `clickhouse` remains available as an explicit events provider. `sqlite` is still a provider contract placeholder for follow-up implementation tasks.

Config precedence is:

1. Environment variables
2. `~/.bitloops/config.json`
3. Defaults

Legacy keys remain supported for backward compatibility:

- `postgres_dsn`, `clickhouse_url`, `clickhouse_user`, `clickhouse_password`, `clickhouse_database`
- `BITLOOPS_DEVQL_PG_DSN`, `BITLOOPS_DEVQL_CH_*`

## 0) Run Postgres with docker (required for current relational adapter)

```bash
docker run -d --name mypostgres -p 5432:5432 \
  -e POSTGRES_USER=bitloops \
  -e POSTGRES_PASSWORD=bitloops \
  -e POSTGRES_DB=bitloops \
  postgres
```

## 1) Configure database connections

Create `~/.bitloops/config.json`:

```json
{
  "devql": {
    "relational": {
      "provider": "postgres",
      "postgres_dsn": "postgres://bitloops:bitloops@localhost:5432/bitloops"
    },
    "events": {
      "provider": "duckdb",
      "duckdb_path": "~/.bitloops/devql/events.duckdb"
    }
  }
}
```

What this does:
- Tells DevQL to use `postgres` for relational data and `duckdb` for events.
- These can be overridden by `BITLOOPS_DEVQL_RELATIONAL_PROVIDER`, `BITLOOPS_DEVQL_EVENTS_PROVIDER`, and legacy/new `BITLOOPS_DEVQL_*` backend settings.
- Postgres is used via `psql` with a 10s connect timeout and 30s statement timeout for health checks and queries; you can override with `PGCONNECT_TIMEOUT` and `PGOPTIONS` if needed.

To use ClickHouse for events instead, set:

```json
"events": {
  "provider": "clickhouse",
  "clickhouse_url": "http://localhost:8123",
  "clickhouse_database": "bitloops",
  "clickhouse_user": "bitloops",
  "clickhouse_password": "bitloops"
}
```

## 2) Check backend connectivity

Run either:

```bash
cargo run -- --connection-status
```

or:

```bash
cargo run -- devql connection-status
```

What this does:
- Runs a connectivity check against the configured logical backends.
- Prints a `DB Status` table with human-friendly statuses:
  - `Connected` (green)
  - `Could not authenticate` (red)
  - `Could not reach DB` (red)
  - `Not configured` (yellow)
- Exits non-zero if any configured backend fails.

Example output:

```text
DB Status
+------------+-----------------------+
| DB         | Status                |
+------------+-----------------------+
| Relational | Connected             |
| Events     | Could not reach DB    |
+------------+-----------------------+
```

## 3) Initialize DevQL schema

```bash
cargo run -- devql init
```

What this does:
- Creates required tables in the currently supported adapters (`duckdb`/`clickhouse` events + `postgres` relational).

## 4) Ingest checkpoint + artefact data

```bash
cargo run -- devql ingest
```

What this does:
- Reads committed checkpoints from the repo.
- Writes checkpoint events to the events backend (`duckdb` by default, or `clickhouse` if configured).
- Writes repository/commit/file/artefact rows to the relational backend (`postgres`).

Optional flags:

```bash
cargo run -- devql ingest --init=false --max-checkpoints 200
```

- `--init=false`: skip schema bootstrap step.
- `--max-checkpoints N`: limit how many checkpoints are ingested.

## 5) Query with DevQL

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
- Routes checkpoint/telemetry stages to the configured events backend (`duckdb` or `clickhouse`).
- Routes artefact stages to the relational backend (`postgres`).
- `chatHistory()` enriches artefact rows with related checkpoint/session chat context.
- Prints JSON output.

## 6) (Optional) Use dashboard with DB startup health

```bash
cargo run -- dashboard --no-open
```

What this does:
- On startup, checks DB health for configured backends.
- Uses the same `DB Status` table/status semantics as `--connection-status`.
- Keeps backend clients for configured adapters (`postgres` plus `duckdb` or `clickhouse`) while dashboard runs.
- Exposes live health at:

```text
http://127.0.0.1:5667/api/db/health
```

## Installed binary equivalents

If `bitloops` is already installed, replace `cargo run -- ...` with:

```bash
bitloops --connection-status
bitloops devql init
bitloops devql ingest
bitloops devql query 'repo("bitloops-cli")->checkpoints()->limit(20)'
bitloops dashboard --no-open
```
