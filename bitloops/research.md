# Storage Authority Cleanup

## Summary

Adopt an explicit storage-authority model and remove ambiguous dual-target behavior:

- `RuntimeStore` stays local SQLite only.
- `RelationalStore` splits into:
  - local workspace projections: all `*_current` relational tables
  - shared relational truth: non-current relational tables in remote relational like `Postgres` when configured, otherwise local relational like `SQLite`
- `Events` stays single-backend: remote event store like `ClickHouse` xor local event store like `DuckDB`
- `Blob` splits by ownership:
  - runtime/session blob payloads stay local
  - project/knowledge blob payloads use the configured backend

## Implementation Changes

### 1. Codify the storage contract in code, not just docs

- Introduce role-based storage boundaries instead of today’s “local + optional remote” abstraction.
- Keep backend choice behind generic abstractions so the code targets storage roles, not concrete engines:
  - `RuntimeStore` -> local only
  - `CurrentProjectionStore` -> local only
  - `SharedRelationalStore` -> remote if configured, otherwise local
  - `EventStore` -> remote if configured, otherwise local
  - runtime/session blob storage -> local only
  - project/knowledge blob storage -> configured backend
- Replace generic relational dual-write helpers with explicit role-based APIs for:
  - local workspace projection reads/writes
  - shared relational reads/writes
- Do not infer routing by parsing SQL strings.
- Route each subsystem intentionally at the call site.

### 2. Make relational ownership explicit

- Treat these as local-only workspace projections when remote relational db like `Postgres` is configured:
  - every relational table with a `*_current` suffix
  - current-projection bookkeeping tables such as `semantic_clone_embedding_setup_state`
  - local current-vector/index helper tables derived from current embeddings
- Treat all other relational tables as shared authoritative data:
  - repo metadata
  - committed/history-oriented DevQL tables
  - non-current semantic features, embeddings, clone edges, test-harness history, and similar pack-owned historical state
- Stop dual-writing relational tables.
- Stop mixed reads where progress/status code reads local while authoritative writes happen remote, unless the table is explicitly classified as local current state.

### 3. Change schema/bootstrap behavior to match ownership

- When shared relational authority is remote, the remote relational schema/bootstrap should create only shared relational tables.
- When shared relational authority is remote, local relational like `SQLite` schema/bootstrap should create current/projection tables and their helper/index tables.
- When shared relational authority is local-only, local relational like `SQLite` schema/bootstrap should continue to create both shared and current/projection relational tables.
- Existing legacy mirrored tables/rows should not be auto-deleted during upgrade.
- Legacy mirrored tables may be detected and warned about, but the first implementation should make them inert by stopping reads/writes to the disallowed side.

### 4. Harden event single-backend behavior and blob ownership routing

- Events:
  - Keep canonical event data (`checkpoint_events`, interaction sessions/turns/events, telemetry-like event history) on the selected event backend only.
  - Preserve local spool/queue state only as transient runtime staging; it must not become a second canonical event store.
  - Ensure no DuckDB fallback or mirror writes occur when remote event store like `ClickHouse` is configured.
- Blob:
  - Runtime/session blob payloads always stay local, even when remote blob store like `S3`/`GCS` is configured for project/knowledge blobs.
  - Local runtime `SQLite` continues to store blob references and metadata for those local runtime/session payloads.
  - Project/knowledge blob payloads use the configured blob backend only: when remote blob store like `S3`/`GCS` is configured, local when local blob storage is configured.
  - No blob payload family should be mirrored across local disk and remote object storage.

### 5. Clarify status, health, and docs

- Update storage docs to describe authority by ownership:
  - runtime local
  - relational current local
  - relational shared remote/local depending on config
  - events single-backend
  - runtime/session blob payloads local
  - project/knowledge blob payloads remote/local depending on config
- Update status/diagnostic surfaces so users can see the effective authority split instead of inferring it from file presence.
- Keep queue/workplane progress ownership explicit in docs and diagnostics:
  - runtime queue and mailbox status remain runtime-local
  - coverage/freshness counts follow the authority of the underlying relational tables
- Clarify that colleagues may share remote historical relational data, remote event data, and remote project/knowledge blob data while each workspace keeps its own local current relational projections and local runtime/session blob payloads.

## Test Plan

- Relational with `Postgres` configured:
  - `artefacts_current` and pack-owned current tables are written/read only from local SQLite.
  - non-current relational tables are written/read only from Postgres.
  - `semantic_clone_embedding_setup_state` is local-only.
  - init progress, freshness, and current-state consumers read the correct side for each table family.
- Queue/progress safety:
  - sync queue state, enrichment queue state, and mailbox/workplane status continue to live in runtime SQLite
  - progress bars that combine queue state with coverage counts continue to read each signal from its owning store instead of forcing all progress reads through Postgres
- Relational without `Postgres`:
  - both current and non-current relational tables behave correctly in local SQLite-only mode.
- Team/workspace scenario:
  - two worktrees on different branches keep different local `*_current` state without overwriting each other.
  - both still share the same Postgres historical/non-current state.
- Events with `ClickHouse` configured:
  - canonical event rows exist only in ClickHouse.
  - local interaction spool/runtime state may exist transiently, but DuckDB is not used as a second event authority.
- Events without `ClickHouse`:
  - canonical event rows continue to live only in the local event backend.
- Blob with remote backend configured:
  - project/knowledge payload writes/readbacks succeed through `S3`/`GCS`
  - runtime/session payload files are still created and read locally
  - runtime `SQLite` reference rows resolve local runtime/session payloads correctly
  - only runtime/session payload files are created under the local blob path; project/knowledge payload files are not
- Blob with local-only backend configured:
  - both runtime/session and project/knowledge payloads are created and read locally
  - no remote blob writes occur
- Regression coverage:
  - add explicit tests for “no duplicate canonical writes” across relational and event backends when remote is selected.
  - add explicit tests that blob payloads are routed by ownership without cross-backend mirroring: runtime/session stays local, project/knowledge follows the configured blob backend.

## Public / Interface Changes

- No new user-facing config keys are required.
- Existing config semantics become explicit:
  - `stores.relational.postgres_dsn` = shared relational authority is Postgres, while workspace-current relational projections remain local
  - `stores.events.clickhouse_*` = event authority is ClickHouse only
  - `stores.blob.local_path` = always provides the local runtime/session blob root, and also provides the project/knowledge blob root when no remote blob backend is configured
  - `stores.blob.s3_*` / `stores.blob.gcs_*` = project/knowledge blob authority is remote, while runtime/session blob payloads remain local
- Internal storage interfaces should be split by storage role and authority, while backend selection stays behind those abstractions so future engines like MySQL can fit without changing call sites.

## Assumptions

- The chosen relational policy is: local-only `*_current` projections, shared non-current relational data in the configured remote relational backend when present, otherwise local relational storage.
- The chosen event policy is: canonical event rows live only in the selected event backend; local spool/queue state is transient runtime state only.
- The chosen blob policy is: runtime/session blob payloads are always local; project/knowledge blob payloads follow the configured blob backend.
- The chosen progress policy is: queue/workplane state remains runtime-local, while progress coverage/freshness reads follow the authority of the data being measured.
- Local runtime queues/spools are allowed to exist, but only as transient operational state, not as second canonical stores.
- Existing duplicated legacy data is not auto-purged in the first pass; the fix stops future duplication and stale reads first.

## TODOs

### Storage role abstractions

- [ ] Introduce explicit internal storage roles for runtime, current projection, shared relational, events, runtime/session blobs, and project/knowledge blobs.
- [ ] Keep backend selection behind those abstractions so call sites do not depend on concrete engines like `Postgres`, `ClickHouse`, `S3`, or `GCS`.
- [ ] Remove or deprecate generic relational dual-write helpers that blur local and remote authority.

### Relational ownership split

- [ ] Audit relational tables and classify each one as local current/projection or shared relational authority.
- [x] Treat all `*_current` relational tables as local-only workspace projection tables.
- [x] Treat `semantic_clone_embedding_setup_state` and current-vector/index helper tables as local-only current/projection state.
- [ ] Route all non-current relational tables through shared relational authority only.
- [ ] Stop dual-writing relational data to local SQLite and remote relational backends at the same time.
- [ ] Update relational readers so progress, freshness, and feature logic read from the owning relational side instead of whichever side happens to contain data.

### Schema and bootstrap

- [x] Change remote relational bootstrap so it creates only shared relational tables when a remote relational backend is configured.
- [x] Change local relational bootstrap so it creates only current/projection tables when a remote relational backend is configured.
- [x] Preserve the existing local-only SQLite bootstrap path that creates both shared and current/projection tables when no remote relational backend is configured.
- [x] Detect legacy mirrored tables/rows and surface warnings without auto-deleting data in the first pass.
- [ ] Make legacy mirrored tables inert by stopping further reads/writes on the disallowed side.

### Events and blobs

- [ ] Audit event-store write paths to ensure canonical event rows go only to the selected event backend.
- [ ] Ensure no DuckDB fallback or mirror writes occur when a remote event backend is configured.
- [x] Preserve local interaction spool, sync queue, and enrichment queue state only as transient runtime staging.
- [x] Split blob routing by ownership so runtime/session payloads always stay local.
- [x] Route project/knowledge blob payloads through the configured blob backend only.
- [x] Ensure no blob payload family is mirrored across local disk and remote object storage.

### Progress, diagnostics, and docs

- [x] Keep sync queue state, enrichment queue state, and mailbox/workplane status in runtime SQLite.
- [x] Ensure progress bars combine runtime-local queue state with coverage/freshness counts from the owning data store.
- [x] Update status and diagnostics to show effective storage authority by data family.
- [x] Update storage docs to explain the ownership split for runtime, relational current, relational shared, events, and blob families.
- [x] Document the multi-workspace behavior: local current projections and runtime/session payloads remain local, while shared historical/event/project payload data may be remote.

### Tests and regression coverage

- [ ] Add relational tests for remote relational mode: current tables local-only, non-current tables remote-only.
- [ ] Add relational tests for local-only mode: both current and non-current relational data remain local.
- [ ] Add multi-workspace tests proving different branches/worktrees do not overwrite each other’s local current state.
- [ ] Add event tests proving canonical event rows exist only in the selected event backend.
- [x] Add blob tests proving runtime/session payloads stay local while project/knowledge payloads follow the configured blob backend.
- [ ] Add regression tests ensuring no duplicate canonical writes remain across relational and event backends when a remote backend is configured.
