# DevQL Getting Started

This guide shows the minimum commands to get DevQL working in the CLI.

## Provider model (foundation contract)

DevQL backend configuration is now split into logical providers:

- `devql.relational.provider`: `sqlite` | `postgres`
- `devql.events.provider`: `duckdb` | `clickhouse`

Current runtime adapters are:

- Relational: `sqlite` (default) and `postgres`
- Events: `duckdb` (default) and `clickhouse`

Config precedence is:

1. Environment variables
2. `~/.bitloops/config.json`
3. Defaults

Legacy keys remain supported for backward compatibility:

- `postgres_dsn`, `clickhouse_url`, `clickhouse_user`, `clickhouse_password`, `clickhouse_database`
- `BITLOOPS_DEVQL_PG_DSN`, `BITLOOPS_DEVQL_CH_*`

## 0) (Optional) Run Postgres and ClickHouse with docker

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
      "provider": "sqlite",
      "sqlite_path": "~/.bitloops/devql/relational.db"
    },
    "events": {
      "provider": "duckdb",
      "duckdb_path": "~/.bitloops/devql/events.duckdb"
    }
  }
}
```

What this does:
- Tells DevQL to use `sqlite` for relational data and `duckdb` for events.
- These can be overridden by `BITLOOPS_DEVQL_RELATIONAL_PROVIDER`, `BITLOOPS_DEVQL_EVENTS_PROVIDER`, and legacy/new `BITLOOPS_DEVQL_*` backend settings.
- If `sqlite_path` is omitted, DevQL defaults to `~/.bitloops/devql/relational.db`.
- Postgres remains available with `relational.provider=postgres` plus `postgres_dsn`.

To use ClickHouse for events instead, set `devql.events` in your config:

```json
{
  "devql": {
    "events": {
      "provider": "clickhouse",
      "clickhouse_url": "http://localhost:8123",
      "clickhouse_database": "bitloops",
      "clickhouse_user": "bitloops",
      "clickhouse_password": "bitloops"
    }
  }
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
- Creates required tables for the configured providers.
- With defaults, this initializes the local SQLite relational DB.

## 4) Ingest checkpoint + artefact data

```bash
cargo run -- devql ingest
```

What this does:
- Reads committed checkpoints from the repo.
- Writes checkpoint events to the events backend (`duckdb` by default, or `clickhouse` if configured).
- Writes repository/commit/file/artefact rows to the relational backend (`sqlite` by default, or `postgres`).

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
- Routes artefact stages to the relational backend (`sqlite` by default, or `postgres`).
- `chatHistory()` enriches artefact rows with related checkpoint/session chat context.
- Prints JSON output.

## 6) (Optional) Use dashboard with DB startup health

```bash
cargo run -- dashboard --no-open
```

What this does:
- On startup, checks DB health for configured backends.
- Uses the same `DB Status` table/status semantics as `--connection-status`.
- Keeps live health checks for configured adapters (`sqlite`/`postgres` relational and `duckdb`/`clickhouse` events) while dashboard runs.
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


## Artefact Completion and Blast Radius Docs

### 1. DevQL architecture was refactored into a dedicated engine module

The older `commands/devql/*` implementation was broken apart and moved into `engine/devql/*`. This is the largest structural change in the branch.

What changed:

- Ingestion logic was split into focused modules such as schema setup, identity generation, persistence, extraction, and edge building.
- Query parsing and execution were moved into dedicated query modules.
- Shared database utilities were moved under the engine module.
- The CLI command layer now delegates to the engine instead of carrying the full implementation inline.

### 2. The Postgres schema was expanded for richer artefact tracking

The branch substantially extends the DevQL Postgres model.

### New or changed artefact storage

- `artefacts` now stores `symbol_id`, `start_byte`, `end_byte`, and `signature`.
- `canonical_kind` was relaxed to nullable so language constructs without a cross-language mapping can still be stored.
- `artefacts_current` was added to represent the latest known state of each symbol in the repository.
- `current_file_state` was added to track the latest blob for each file path.

### New dependency-edge storage

- `artefact_edges` was added for historical dependency edges.
- `artefact_edges_current` was added for the current graph view.
- Constraints enforce valid targets and valid line ranges.
- Natural unique indexes were added to keep edge ingestion idempotent.

Why it matters:

- DevQL can now answer both "what existed at a given revision?" and "what is true right now?" without rebuilding the graph every time.
- Symbols and dependencies are stored with enough fidelity for blast-radius analysis and repeatable re-ingestion.

### 3. Artefact identity and persistence were redesigned

The branch introduces a clearer separation between semantic identity and revision identity.

Key behavior changes:

- `symbol_id` identifies the logical symbol.
- `artefact_id` identifies the symbol within a specific blob revision.
- Parent-child relationships are tracked at both symbol and artefact levels.
- File artefacts are persisted explicitly and become the parent container for nested symbols.
- Current-state tables are updated through upserts keyed by semantic identity.

Why it matters:

- A symbol can evolve across commits while still being traceable as the "same" logical artefact.
- Historical queries remain revision-accurate, while current queries stay fast and stable.

### 4. JS/TS artefact extraction was upgraded from regex-driven to AST-driven

JS/TS extraction now uses tree-sitter instead of relying only on regex matching.

### JS/TS artefacts extracted in this branch

| Language kind | Canonical kind |
| --- | --- |
| `function_declaration` | `function` |
| `method_definition` | `method` |
| `interface_declaration` | `interface` |
| `type_alias_declaration` | `type` |
| `variable_declarator` | `variable` |
| `import_statement` | `import` |
| `class_declaration` | `null` |
| `constructor` | `null` |

Important modeling decision:

- Not every language-specific construct receives a canonical cross-language kind.
- For example, `class_declaration` and `constructor` are stored but their `canonical_kind` remains `null`.


### 5. Rust artefact extraction was added

Rust now participates in artefact extraction instead of being effectively file-only.

### Rust artefacts extracted in this branch

| Language kind | Canonical kind |
| --- | --- |
| `function_item` outside `impl` | `function` |
| `function_item` inside `impl` | `method` |
| `trait_item` | `interface` |
| `type_item` | `type` |
| `enum_item` | `enum` |
| `use_declaration` | `import` |
| `mod_item` | `module` |
| `struct_item` | `null` |
| `impl_item` | `null` |
| `const_item` | `null` |
| `static_item` | `null` |

### 6. Dependency-edge extraction was added for both JS/TS and Rust

This branch introduces persisted dependency edges, which is the main enabler for blast-radius and reverse-dependency workflows.

### Edge kinds introduced

| Edge kind | Meaning |
| --- | --- |
| `imports` | File or symbol depends on an imported module or path |
| `calls` | Callable invokes another callable |
| `references` | Symbol references another symbol, especially type/value references |
| `inherits` | Type extends or inherits from another type |
| `implements` | Rust `impl Trait for Type` relationship |
| `exports` | File re-exports or publicly exports another symbol |

### Resolution behavior

- If a dependency resolves to a known local artefact, the edge stores the target artefact ID.
- If it cannot be resolved locally, the edge falls back to `to_symbol_ref`.
- Call edges carry metadata such as resolution mode and call form.
- Export edges are deduplicated so repeated exports do not create duplicate logical edges.

### Language-specific behavior worth calling out

- JS/TS supports imports, local/import/unresolved calls, reference edges, inheritance edges, and export edges.
- Rust supports `use` imports, calls, trait implementation edges, inheritance/reference edges, and export edges.
- Rust macro invocations are inspected as calls, but unresolved macro calls are dropped rather than persisted as bad edges.

Why it matters:

- DevQL can now answer both outgoing dependency questions and incoming blast-radius questions from stored graph data.

### 7. Query capabilities were extended to support dependency traversal

The branch adds dependency traversal on top of artefact selection.

### Query capabilities added

- `deps(kind:"...", direction:"out" | "in" | "both")`
- `include_unresolved:true|false`
- `asOf(commit:"...")` or `asOf(ref:"...")` to switch from current-state tables to historical tables

### Behavior changes

- Current queries read from `artefacts_current` and `artefact_edges_current`.
- Historical queries read from `artefacts` and `artefact_edges`.
- Reverse dependency queries join against the edge target side.
- Bidirectional queries union inbound and outbound edges.
- Invalid stage combinations such as `deps()->chatHistory()` are explicitly rejected.

Why it matters:

- Blast-radius analysis is now a first-class query path rather than an inferred or post-processed capabilit