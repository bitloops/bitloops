# Native Semantic Vector Backend Abstraction for SQLite and Postgres

## The final goal is to make faster the devql query by searchMode: AUTO, because it takes time with hnsw_rs and that's why we need to leverage the use of vectors in the db. A plan is written below.

## Summary
- Introduce a backend abstraction for semantic vector persistence and nearest-neighbor retrieval so the search/domain code does not know whether it is using local SQLite or remote Postgres.
- Keep `symbol_embeddings` / `symbol_embeddings_current` as the canonical embedding source of truth. The vector backend is an infrastructure concern that is fed from those rows and queried for candidate IDs.
- Implement both backends in the first delivery:
  - SQLite repos use `sqlite-vec`
  - Postgres repos use `pgvector`
- Support mixed embedding dimensions:
  - SQLite adapter manages dimension-scoped `vec0` tables
  - Postgres adapter stores vectors in regular table columns and builds per-dimension ANN indexes
- Remove request-time Rust HNSW from the semantic search path only. Keep current score thresholds, reranking, and GraphQL behavior unchanged.
- Replace request-time HNSW building in [semantic_artefact_query.rs](/Users/elli/Projects/Bitloops/bitloops/bitloops/src/graphql/semantic_artefact_query.rs) with local `sqlite-vec` or remote `pg-vector` KNN retrieval for all semantic search lanes: `CODE`, `IDENTITY`, `SUMMARY`, and `AUTO`.

## Key Changes
- Add a new infra-facing interface, e.g. `SemanticVectorBackend`, consumed by semantic search and semantic embedding persistence. It should expose:
  - `ensure_schema()`
  - `upsert_current_rows(...)`
  - `upsert_historical_rows(...)`
  - `delete_current_rows_for_paths(...)`
  - `clear_repo_rows(...)`
  - `set_active_setup(...)`
  - `nearest_candidates(repo_id, representation_kind, setup_fingerprint, dimension, query_embedding, limit) -> candidate IDs + raw distance`
- Add a backend selector derived from configured relational backend, not from incidental local SQLite availability. Extend the relational/storage layer with an explicit “primary backend” concept so semantic search can choose:
  - SQLite adapter when the repo is SQLite-only
  - Postgres adapter when the repo is configured with remote Postgres
- Refactor semantic search in [semantic_artefact_query.rs](/Users/elli/Projects/Bitloops/bitloops/bitloops/src/graphql/semantic_artefact_query.rs):
  - stop calling `query_devql_sqlite_rows(...)` and local-only store helpers directly
  - resolve the vector backend from the relational abstraction
  - embed the query as today
  - ask the backend for the top semantic candidate artefact IDs per representation/setup
  - hydrate only those candidates from the primary relational backend
  - keep existing exact rerank, score normalization, thresholding, and dedupe logic in the domain layer
- Refactor semantic embedding persistence in the stage-embeddings pipeline so writes feed both the canonical embedding rows and the selected vector backend through the abstraction instead of embedding backend-specific SQL in the domain flow.

## Backend Implementations
- SQLite adapter:
  - Keep canonical rows in `symbol_embeddings` and `symbol_embeddings_current`.
  - Add dimension-scoped `vec0` tables inside the same SQLite DB, one per dimension actually encountered.
  - create required dimension tables lazily for every distinct `dimension` present in `symbol_embeddings_current`
  - Auto-create/backfill missing vec tables from existing `symbol_embeddings_current` rows during schema/init.
  - Mirror current-row upserts/deletes/clears into the vec tables.
  - Query `vec0` with partition filters (`repo_id`, `representation_kind`, `setup_fingerprint`) and return candidate artefact IDs for exact rerank.
  - when a dimension table is newly created, backfill it immediately from existing current rows using `vec_f32(embedding)` from the stored JSON text
  - after persisting a current embedding row, upsert the matching vec row into the correct dimension table using bound float32 BLOB data
- Postgres adapter:
  - Extend `symbol_embeddings` and `symbol_embeddings_current` with a native `pgvector` column, using plain `vector` to allow mixed dimensions.
  - Initialize `pgvector` with `CREATE EXTENSION IF NOT EXISTS vector`.
  - Mirror semantic embedding writes to remote Postgres as part of the same semantic persistence flow, including current rows, historical rows, and active setup state.
  - Build ANN indexes on `symbol_embeddings_current` per dimension via partial HNSW indexes so mixed dimensions remain searchable without separate physical tables.
  - Query Postgres natively for nearest candidates from the chosen backend; do not fall back to local SQLite for Postgres-configured repos.
- Cross-cutting:
  - Candidate hydration for semantic search must read from the primary backend only, not merged local+remote rows, to avoid duplication and backend leakage.
  - Keep `hnsw_rs` only where clone scoring still uses it today; semantic search should no longer build per-request ANN structures.
- keep deletes/clears/setup changes in sync there too
- pgvector index maintenance is then handled by Postgres automatically once the row is updated

## Test Plan
- Backend abstraction tests:
  - selector chooses SQLite vs Postgres from backend config, not from presence of a local cache DB
  - semantic search uses backend candidate retrieval plus exact rerank with unchanged ordering semantics
- SQLite tests:
  - schema/init creates and backfills dimension-scoped vec tables from pre-existing current rows
  - current-row upserts and delete/clear paths keep vec tables in sync
  - existing GraphQL semantic search tests continue to pass unchanged for `IDENTITY`, `CODE`, `SUMMARY`, and `AUTO`
- Postgres tests:
  - unconditionally test SQL/schema builders and backend dispatch
  - add opt-in integration tests under existing Postgres-gated suites for `pgvector` schema init, mirrored writes, ANN candidate retrieval, and semantic query parity
- Manual acceptance:
  - on `/Users/elli/Projects/axum`, warm `searchMode: CODE` for `"part"` preserves the validated top-hit order
  - warm `AUTO` queries no longer spend multi-second time rebuilding semantic indexes
  - Postgres-configured repos execute semantic search through Postgres, not local SQLite

## Assumptions
- Both backends are delivered in the first implementation, behind one shared abstraction.
- Mixed dimensions are supported immediately.
- SQLite physical layout is backend-specific and may use multiple vec tables; Postgres physical layout remains one canonical table with extra vector columns and dimension-partial indexes.
- Canonical embedding metadata remains in `symbol_embeddings` / `symbol_embeddings_current`; the vector backend is derived/search infrastructure.
- Postgres users must have a database where `pgvector` can be enabled; if `CREATE EXTENSION vector` is not permitted, Bitloops should surface a clear setup error rather than silently falling back to another backend.
- `sqlite-vec` is embedded and registered by the application process, so SQLite users do not need separate manual extension installation.
- Implementation references: [sqlite-vec vec0 docs](https://alexgarcia.xyz/sqlite-vec/features/vec0.html), [sqlite-vec KNN docs](https://alexgarcia.xyz/sqlite-vec/features/knn.html), [pgvector README](https://github.com/pgvector/pgvector)

## TODOs
[x] Define `SemanticVectorBackend` and the backend selector on top of the existing relational abstraction, without leaking SQLite/Postgres details into the search/domain layer.
[x] Audit the current semantic search path and replace local-only SQLite entry points with backend-agnostic vector candidate lookup plus backend-agnostic artefact hydration.
[x] Implement the SQLite adapter with dimension-scoped `vec0` tables, lazy table creation, backfill from `symbol_embeddings_current`, and synchronized upsert/delete/clear behavior.
[x] Implement the Postgres adapter with `pgvector` schema initialization, mirrored vector writes for current and historical embeddings, partial HNSW indexes per dimension, and native nearest-neighbor queries.
[x] Wire semantic embedding persistence so canonical embedding writes also update the selected vector backend and keep setup changes, stale-row deletion, and repo/path clears in sync.
[x] Remove request-time `hnsw_rs` usage from semantic search while keeping current exact rerank, score thresholds, and dedupe behavior unchanged.
[x] Add backend-focused tests for schema/init, write synchronization, candidate retrieval, and GraphQL semantic search parity across `CODE`, `IDENTITY`, `SUMMARY`, and `AUTO`.
[ ] Run manual verification on `axum` for warm `CODE` and `AUTO` queries and confirm the result ordering remains stable while latency drops materially.
