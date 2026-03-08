# DevQL Getting Started

This guide shows the minimum commands to get DevQL working in the CLI.

## 0) Run Postgres and ClickHouse with docker

```bash
docker run -d --name mypostgres -p 5432:5432 \
  -e POSTGRES_USER=bitloops \
  -e POSTGRES_PASSWORD=bitloops \
  -e POSTGRES_DB=bitloops \
  postgres
```

```bash
docker run -d --name bitloops-clickhouse \
  -p 8123:8123 -p 9000:9000 \
  -e CLICKHOUSE_USER=bitloops \
  -e CLICKHOUSE_PASSWORD=bitloops \
  -e CLICKHOUSE_DB=bitloops \
  clickhouse/clickhouse-server
```


## 1) Configure database connections

Create `~/.bitloops/config.json`:

```json
{
  "devql": {
    "postgres_dsn": "postgres://bitloops:bitloops@localhost:5432/bitloops",
    "clickhouse_url": "http://localhost:8123",
    "clickhouse_database": "bitloops",
    "clickhouse_user": "bitloops",
    "clickhouse_password": "bitloops"
  }
}
```

What this does:
- Tells DevQL where Postgres (artefacts) and ClickHouse (checkpoint events) live.
- These can also be overridden by `BITLOOPS_DEVQL_*` environment variables.
- Postgres is used via `psql` with a 10s connect timeout and 30s statement timeout for health checks and queries; you can override with `PGCONNECT_TIMEOUT` and `PGOPTIONS` if needed.

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
- Runs a connectivity check against Postgres and ClickHouse.
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
| Postgres   | Connected             |
| ClickHouse | Could not reach DB    |
+------------+-----------------------+
```

## 3) Initialize DevQL schema

```bash
cargo run -- devql init
```

What this does:
- Creates required tables in ClickHouse and Postgres for DevQL MVP.

## 4) Ingest checkpoint + artefact data

```bash
cargo run -- devql ingest
```

What this does:
- Reads committed checkpoints from the repo.
- Writes checkpoint events to ClickHouse.
- Writes repository/commit/file/artefact rows to Postgres.

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
- Routes checkpoint/telemetry stages to ClickHouse.
- Routes artefact stages to Postgres.
- `chatHistory()` enriches artefact rows with related checkpoint/session chat context.
- Prints JSON output.

## 6) (Optional) Use dashboard with DB startup health

```bash
cargo run -- dashboard --no-open
```

What this does:
- On startup, checks DB health for configured backends.
- Uses the same `DB Status` table/status semantics as `--connection-status`.
- Keeps pooled Postgres and ClickHouse clients while dashboard runs.
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
