# Context

> This plan was moved from the repo root into the Test Harness capability-pack docs set. Historical `TestLens` ownership has been translated to the current in-repo `bitloops` layout where direct equivalents exist.

Check the test-harness-stage-1 branch. There is a TestHarness implementation (functional prototype), based on this document https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/450265089/Feature+Spec+Draft+Structural+Test+Mapping Full Test harness document: https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/433717249/Test+Harness+-+Design+Specification?atl_f=PAGETREE bitloops is the existing CLI, which contains stuff for getBlastRadius, devQL. The TestLens has an ingest_production_artefacts functionality which is not going to be needed, since this is implemented by the main bitloops (a mock simple impl was done, so testHarness could be developed unblocked). We are going probably to have schema compatibility issues that we need to figure out, (regarding production artefacts, the bitloops should be the source of truth). So in a high level flow, i think test artefacts should be on an independant table probably, and not mix them with production artefacts.

---

I think the correct plan of course, is to bring testlens in a suite closer to the bitloops.
So as 1st stage we could update Testlens, to store production and test artefact seperatelly.
Make the production artefacts table look exactly like the bitloops's production artefacts schema.

---

Below is a codex-ready implementation plan for Stage 1.

This plan is based on the current TestLens prototype architecture and schema, where production and test artefacts are mixed in one `artefacts` table and test discovery writes `test_suite` / `test_scenario` rows into that same table , while `bitloops` already has the richer production schema and identity model built around `repositories`, `commits`, `file_state`, `artefacts`, `artefacts_current`, and edge tables . The current TestLens behavior also explicitly uses direct-only static linkage and a separate query/read layer, which should be preserved

# Stage 1 objective

Refactor TestLens so that:

1. production artefacts are stored separately from test artefacts
2. the production artefact storage schema in TestLens matches `bitloops` exactly for the production domain
3. test suites, test scenarios, test links, test runs, classifications, coverage, and diagnostics live in dedicated test-domain tables
4. existing TestLens behavior is preserved as much as possible:
   - Rust-first structural mapping stays intact
   - direct-only linkage stays intact
   - current CLI commands still work
   - query results remain functionally equivalent for current prototype use cases

This stage does **not** make `bitloops` the runtime source of truth yet. It only makes TestLens structurally compatible with that future integration.

# Non-goals for Stage 1

Do not do these in this stage:

- do not make TestLens read production artefacts from `bitloops`
- do not merge TestLens into `bitloops`
- do not redesign DevQL transport
- do not add new product capabilities
- do not change static linkage from direct-only to transitive
- do not add dynamic instrumentation
- do not expand scope beyond current TS/Rust prototype support

# Hard requirements

Codex must satisfy all of these:

1. Production tables in TestLens must use the exact same schema as `bitloops`’s production storage.
2. Test artefacts must no longer be stored in production `artefacts` or `artefacts_current`.
3. Test links must reference production artefacts by `production_artefact_id`, and should also store `production_symbol_id` when available.
4. Query paths must resolve the target artefact only from the production tables.
5. Coverage and run ingestion must reference test scenarios via dedicated test tables, not via mixed artefact rows.
6. Existing e2e behaviors for discovery, linkage, coverage, and querying must still pass after updates.

# Implementation strategy

Implement this in six phases, in order.

## Phase 1: replace TestLens production schema with the `bitloops` production schema

### 1.1 Copy production schema exactly

In `bitloops/src/capability_packs/test_harness/storage/schema.rs` (and the shared SQLite bootstrap in `bitloops/src/storage/init/schema.rs`), replace the current prototype production table definitions with the exact SQLite DDL from:

- `bitloops/src/host/devql/ingestion/schema/relational_sqlite_schema.rs` for:
  - `repositories`
  - `commits`
  - `file_state`
  - `current_file_state`
  - `artefacts`
  - `artefacts_current`
  - `artefact_edges`
  - `artefact_edges_current`

Do not rename columns.
Do not simplify columns.
Do not drop indexes.
Do not reinterpret meanings.

### 1.2 Keep test-domain tables separate

Add new test-domain tables to `bitloops/src/capability_packs/test_harness/storage/schema.rs`:

`test_suites`

- `suite_id TEXT PRIMARY KEY`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `language TEXT NOT NULL`
- `path TEXT NOT NULL`
- `name TEXT NOT NULL`
- `symbol_fqn TEXT`
- `start_line INTEGER NOT NULL`
- `end_line INTEGER NOT NULL`
- `start_byte INTEGER`
- `end_byte INTEGER`
- `signature TEXT`
- `discovery_source TEXT NOT NULL`
- `created_at TEXT DEFAULT (datetime('now'))`

Indexes:

- `(repo_id, commit_sha)`
- `(repo_id, commit_sha, path)`

`test_scenarios`

- `scenario_id TEXT PRIMARY KEY`
- `suite_id TEXT REFERENCES test_suites(suite_id) ON DELETE CASCADE`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `language TEXT NOT NULL`
- `path TEXT NOT NULL`
- `name TEXT NOT NULL`
- `symbol_fqn TEXT`
- `start_line INTEGER NOT NULL`
- `end_line INTEGER NOT NULL`
- `start_byte INTEGER`
- `end_byte INTEGER`
- `signature TEXT`
- `discovery_source TEXT NOT NULL`
- `created_at TEXT DEFAULT (datetime('now'))`

Indexes:

- `(repo_id, commit_sha)`
- `(repo_id, commit_sha, suite_id)`
- `(repo_id, commit_sha, path)`

`test_links`

- `test_link_id TEXT PRIMARY KEY`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE`
- `production_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id) ON DELETE CASCADE`
- `production_symbol_id TEXT`
- `link_source TEXT NOT NULL DEFAULT 'static_analysis'`
- `evidence_json TEXT DEFAULT '{}'`
- `created_at TEXT DEFAULT (datetime('now'))`

Indexes:

- `(repo_id, commit_sha, production_artefact_id)`
- `(repo_id, commit_sha, test_scenario_id)`
- unique natural key on `(commit_sha, test_scenario_id, production_artefact_id, link_source)`

`test_runs`

- `run_id TEXT PRIMARY KEY`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE`
- `status TEXT NOT NULL`
- `duration_ms INTEGER`
- `ran_at TEXT NOT NULL`

Indexes:

- `(repo_id, commit_sha, test_scenario_id)`
- `(repo_id, test_scenario_id, ran_at)`

`test_classifications`

- `classification_id TEXT PRIMARY KEY`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE`
- `classification TEXT NOT NULL`
- `classification_source TEXT NOT NULL DEFAULT 'coverage_derived'`
- `fan_out INTEGER NOT NULL`
- `boundary_crossings INTEGER NOT NULL DEFAULT 0`

Indexes:

- `(repo_id, commit_sha, test_scenario_id)`

`coverage_captures`

- `capture_id TEXT PRIMARY KEY`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `tool TEXT NOT NULL DEFAULT 'unknown'`
- `format TEXT NOT NULL DEFAULT 'lcov'`
- `scope_kind TEXT NOT NULL DEFAULT 'workspace'`
- `subject_test_scenario_id TEXT REFERENCES test_scenarios(scenario_id) ON DELETE SET NULL`
- `line_truth INTEGER NOT NULL DEFAULT 1`
- `branch_truth INTEGER NOT NULL DEFAULT 0`
- `captured_at TEXT NOT NULL`
- `status TEXT NOT NULL DEFAULT 'complete'`
- `metadata_json TEXT`

Indexes:

- `(repo_id, commit_sha, scope_kind)`

`coverage_hits`

- `capture_id TEXT NOT NULL REFERENCES coverage_captures(capture_id) ON DELETE CASCADE`
- `production_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id) ON DELETE CASCADE`
- `file_path TEXT NOT NULL`
- `line INTEGER NOT NULL`
- `branch_id INTEGER NOT NULL DEFAULT -1`
- `covered INTEGER NOT NULL`
- `hit_count INTEGER DEFAULT 0`
- primary key `(capture_id, production_artefact_id, line, branch_id)`

Indexes:

- `(production_artefact_id, capture_id)`

`test_discovery_runs`

- `discovery_run_id TEXT PRIMARY KEY`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `language TEXT`
- `started_at TEXT NOT NULL`
- `finished_at TEXT`
- `status TEXT NOT NULL`
- `enumeration_status TEXT`
- `notes_json TEXT`
- `stats_json TEXT`

Indexes:

- `(repo_id, commit_sha)`

`test_discovery_diagnostics`

- `diagnostic_id TEXT PRIMARY KEY`
- `discovery_run_id TEXT REFERENCES test_discovery_runs(discovery_run_id) ON DELETE CASCADE`
- `repo_id TEXT NOT NULL`
- `commit_sha TEXT NOT NULL`
- `path TEXT`
- `line INTEGER`
- `severity TEXT NOT NULL`
- `code TEXT NOT NULL`
- `message TEXT NOT NULL`
- `metadata_json TEXT`

Indexes:

- `(repo_id, commit_sha)`
- `(discovery_run_id)`

### 1.3 Migration policy

Do not implement a complex migration from the old prototype DB.

Instead:

- bump schema version behavior implicitly by changing `testlens init`
- document that old prototype DBs must be recreated
- fail fast if an old DB is opened with the new code and required tables are missing

This is acceptable for the prototype stage.

## Phase 2: split the domain model into production and test domains

### 2.1 Replace the current mixed artefact record model

In `bitloops/src/models.rs`, split the current mixed types into:

### Production domain records

These must mirror `bitloops` semantics:

- `RepositoryRecord`
- `CommitRecord`
- `FileStateRecord`
- `CurrentFileStateRecord`
- `ProductionArtefactRecord`
- `CurrentProductionArtefactRecord`
- `ProductionEdgeRecord`
- `CurrentProductionEdgeRecord`

These should be modeled to match the production schema copied from `bitloops`

### Test domain records

- `TestSuiteRecord`
- `TestScenarioRecord`
- `TestLinkRecord`
- `TestRunRecord`
- `TestClassificationRecord`
- `CoverageCaptureRecord`
- `CoverageHitRecord`
- `TestDiscoveryRunRecord`
- `TestDiscoveryDiagnosticRecord`

### 2.2 Keep classification logic where it is

Keep the current classification threshold constants and derivation logic in `bitloops/src/models.rs`, but update them to operate on `test_scenario_id`-based records instead of mixed artefact IDs

## Phase 3: rewrite production ingestion to match `bitloops` semantics

## 3.1 Production ingestion still exists in Stage 1

Keep the production-ingest entrypoints in `bitloops/src/cli/devql.rs`, but rewrite the underlying materialisation so it produces the same kind of persisted production state as `bitloops`, not the current thin prototype rows

## 3.2 Reuse `bitloops` identity semantics

Implement or copy the identity helpers from:

- `bitloops/src/host/devql/ingestion/artefact_identity.rs`
- `bitloops/src/host/devql/ingestion/artefact_persistence.rs`

Specifically align these semantics:

- `symbol_id`
- file symbol identity
- revision `artefact_id`
- content hash semantics
- parent symbol / parent artefact relationships

The exact requirement is:

- `symbol_id` must represent semantic identity
- `artefact_id` must represent revision identity
- `blob_sha` must be present
- `artefacts_current` must represent current semantic resolution
- `artefacts` must represent historical per-revision rows

## 3.3 Minimum production fields that must now be populated

For every production artefact row:

- `artefact_id`
- `symbol_id`
- `repo_id`
- `blob_sha`
- `path`
- `language`
- `canonical_kind`
- `language_kind`
- `symbol_fqn`
- `parent_artefact_id`
- `start_line`
- `end_line`
- `start_byte`
- `end_byte`
- `signature`
- `modifiers`
- `docstring`
- `content_hash`

Where data is unavailable:

- `modifiers` defaults to `[]`
- `docstring` may be `NULL`
- still populate byte spans from tree-sitter nodes
- still populate `blob_sha`

## 3.4 Also persist repository and commit state

Production ingestion must now write:

- `repositories`
- `commits`
- `file_state`
- `current_file_state`

for the analyzed repository and commit.

## 3.5 Do not persist tests as production artefacts

Production ingestion must only write source-of-truth production/file/edge data.
It must not write:

- test file rows
- `test_suite`
- `test_scenario`

## Phase 4: rewrite test discovery to dedicated test tables

## 4.1 Preserve the current discovery subsystem

Keep the current structural mapping subsystem in:

- `bitloops/src/capability_packs/test_harness/mapping/*`
- `bitloops/src/capability_packs/test_harness/mapping/languages/rust/*`
- `bitloops/src/capability_packs/test_harness/mapping/languages/typescript.rs`
- `bitloops/src/capability_packs/test_harness/mapping/linker.rs`

Do not redesign the discovery algorithm.
Do not change direct-only linking behavior

## 4.2 Change only the materialization target

In `bitloops/src/capability_packs/test_harness/mapping/materialize.rs`, stop creating `ArtefactRecord` rows for:

- test files
- test suites
- test scenarios

Replace that with:

- `TestSuiteRecord` rows into `test_suites`
- `TestScenarioRecord` rows into `test_scenarios`
- `TestLinkRecord` rows into `test_links`

## 4.3 Linking rules

For every discovered test scenario:

- resolve matching production artefacts using the existing linker
- insert one `test_links` row per direct match
- always write `production_artefact_id`
- when possible, also write `production_symbol_id`

How to get `production_symbol_id`:

- load it from production `artefacts` or `artefacts_current` when resolving the matched production artefact
- if not available from the current query path, add a production lookup repository method to fetch it

## 4.4 Test files are not first-class production artefacts anymore

Do not create synthetic test file artefact rows in production tables.
The path on `test_suites` and `test_scenarios` is enough for Stage 1.

## 4.5 Diagnostics

Create a `test_discovery_run` row for each `ingest-tests` execution.
Persist all current discovery issues into `test_discovery_diagnostics` instead of keeping them only in memory or printing them.

## Phase 5: update coverage and results ingestion

## 5.1 Coverage ingestion

In `bitloops/src/capability_packs/test_harness/ingest/coverage.rs`:

Change coverage ingestion so that:

- `coverage_captures.subject_test_artefact_id` becomes `subject_test_scenario_id`
- `coverage_hits.artefact_id` becomes `production_artefact_id`
- all joins resolve against `test_scenarios` and production `artefacts`, not mixed artefact rows

Keep current behavior around LCOV / LLVM JSON handling and coverage mode semantics as much as possible

## 5.2 Results ingestion

In `bitloops/src/capability_packs/test_harness/ingest/results.rs`:

Change all result/run mapping to resolve against `test_scenarios.scenario_id`, not `artefacts.artefact_id` for mixed test rows.

## 5.3 Rebuild classifications from dedicated test tables

In `bitloops/src/capability_packs/test_harness/storage/sqlite.rs`, rewrite `rebuild_classifications_from_coverage` so it:

- groups by `subject_test_scenario_id`
- counts distinct `production_artefact_id`
- computes boundary crossings from covered production file paths
- writes `test_classifications.test_scenario_id`

Do not read test identity from mixed `artefacts` anymore.

## Phase 6: rewrite repository and query layers

## 6.1 Split repository traits

In `bitloops/src/capability_packs/test_harness/storage.rs`, split the current repository contracts into clearer domains:

`ProductionRepository`

- load repo / commit / file state
- load production artefacts
- load production edges
- resolve production artefacts by query
- upsert production state

`TestHarnessRepository`

- upsert test suites/scenarios/links
- upsert discovery runs/diagnostics
- upsert coverage
- upsert test runs
- rebuild classifications

`TestHarnessQueryRepository`

- find production artefact by query
- load covering tests for production artefact
- load latest test run
- load coverage summary
- load linked fan-out
- list production artefacts

## 6.2 Remove cleanup heuristics based on mixed-table semantics

The current cleanup logic deletes test or production rows from the same `artefacts` table using `canonical_kind` and path-pattern heuristics

Delete that logic.

Replace it with domain-specific cleanup:

- production ingestion clears and rewrites only production tables/state for the given commit/path/blob
- test ingestion clears and rewrites only test-domain rows for the given commit
- coverage/results only clear their own domain rows

## 6.3 Query layer must resolve only production artefacts as query targets

In `bitloops/src/capability_packs/test_harness/query.rs` and the storage backends under `bitloops/src/capability_packs/test_harness/storage/`:

`find_artefact(...)` must resolve only production artefacts from production tables, never test tables.

Then:

- `load_covering_tests` must join:
  - `test_links`
  - `test_scenarios`
  - `test_suites`
  - `test_classifications`

- `load_latest_test_run` must use `test_runs.test_scenario_id`
- `load_coverage_pair_stats` must use:
  - `coverage_captures.subject_test_scenario_id`
  - `coverage_hits.production_artefact_id`

The current query behavior around `summary`, `tests`, `coverage`, min-strength filtering, confidence scoring, and verification-level logic should remain unchanged unless a schema update requires a small refactor

# File-by-file work list

Implement changes in this order.

## Must change

`bitloops/src/capability_packs/test_harness/storage/schema.rs`

- replace production schema with exact `bitloops` production schema
- add dedicated test-domain tables

`bitloops/src/models.rs`

- split production and test domain records
- keep classification logic, adapt IDs

`bitloops/src/capability_packs/test_harness/storage.rs`

- split repository traits

`bitloops/src/capability_packs/test_harness/storage/sqlite.rs`

- rewrite persistence and query logic for separated domains
- remove mixed-table cleanup logic
- update classification rebuild
- update covering-tests query joins

`bitloops/src/host/devql/ingestion/artefact_persistence.rs`

- rewrite to emit `bitloops`-compatible production rows and state

`bitloops/src/capability_packs/test_harness/mapping/materialize.rs`

- emit `test_suites`, `test_scenarios`, `test_links`
- stop creating test rows in production tables

`bitloops/src/capability_packs/test_harness/ingest/tests.rs`

- write discovery runs and diagnostics
- call new test-domain persistence path

`bitloops/src/capability_packs/test_harness/ingest/coverage.rs`

- update to `test_scenario_id` and `production_artefact_id`

`bitloops/src/capability_packs/test_harness/ingest/results.rs`

- update to dedicated test scenario tables

`bitloops/src/capability_packs/test_harness/query.rs`

- query only production artefacts as targets
- join into test tables for harness output

## Likely change

`bitloops/src/capability_packs/test_harness/mapping/model.rs`

- keep discovery model
- adjust output types so they are no longer built around `ArtefactRecord` for test entities

`bitloops/src/capability_packs/test_harness/mapping/linker.rs`

- possibly add lookup support for `production_symbol_id`

`bitloops/src/cli/testlens.rs`

- wire new repository functions

`docs/layered-extension-architecture.md`

- update architecture diagram and wording

`docs/layered-extension-architecture-capability-packs.md`

- document split-domain storage

# Exact behavioral rules codex must preserve

1. Static test linkage remains direct-only.
2. Existing test discovery logic remains the same unless required by schema separation.
3. Query views remain:
   - `full`
   - `summary`
   - `tests`
   - `coverage`

4. Default `min_strength` remains `0.3` for `tests` and `full`.
5. Verification-level thresholds remain unchanged.
6. Classification thresholds remain unchanged.
7. Confidence scoring behavior remains unchanged.

These behaviors are part of the current prototype contract and should not drift in this stage

# Acceptance criteria

The implementation is done only when all of these are true.

## Schema acceptance

- TestLens production tables match `bitloops` production schema exactly.
- No test suite or test scenario rows are stored in production `artefacts` or `artefacts_current`.
- All test-domain entities live in dedicated test tables.

## Ingestion acceptance

- `ingest-production-artefacts` writes:
  - repositories
  - commits
  - file_state
  - current_file_state
  - artefacts
  - artefacts_current
  - artefact_edges
  - artefact_edges_current

- `ingest-tests` writes:
  - test_suites
  - test_scenarios
  - test_links
  - test_discovery_runs
  - test_discovery_diagnostics

- `ingest-coverage` writes:
  - coverage_captures
  - coverage_hits
  - test_classifications

- `ingest-results` writes:
  - test_runs

## Query acceptance

- `query --artefact ...` resolves only production artefacts
- `tests()` results still include covering tests, classifications, confidence, strength, and summary
- coverage summaries still work
- latest run data still works

## Integrity acceptance

- `test_links.production_artefact_id` always points to a valid production artefact row
- deleting/replacing test discovery for a commit does not delete production artefacts
- deleting/replacing production artefacts for a commit does not delete test tables except where explicitly intended through a full commit reset flow

## Behavioral acceptance

All existing relevant tests still pass, plus new assertions are added to prove separation.

# Required test updates

Add or update tests for these cases.

## Unit tests

1. production ingestion writes `artefacts` with `symbol_id`, `blob_sha`, byte spans, and content hash
2. test discovery no longer writes test rows into production `artefacts`
3. classification rebuild uses `test_scenario_id`
4. coverage pair stats use `production_artefact_id`
5. query target resolution excludes test tables

## Integration / e2e tests

Update current e2e flows so they assert:

1. after `ingest-production-artefacts`, production tables contain rows and test tables are empty
2. after `ingest-tests`, test tables contain suites/scenarios/links, but production `artefacts` does not gain `test_suite` or `test_scenario`
3. after `ingest-coverage`, classifications and coverage rows refer to test scenarios and production artefacts correctly
4. `query` returns the same functional output shape as before

Use the existing e2e harness and fixture flow already present in TestLens

# Implementation notes for codex

Use these rules while coding:

- Prefer copying exact production schema and identity semantics from `bitloops` over re-implementing approximations.
- Do not preserve backward compatibility with the old TestLens DB layout.
- Do not invent a new production identity model.
- Do not leave half-migrated mixed-table logic behind.
- Do not write test artefacts into production tables “temporarily”.
- Keep Stage 1 focused on storage separation and schema compatibility only.

# Suggested execution order for codex

1. update `capability_packs/test_harness/storage/schema.rs`
2. refactor `models.rs`
3. split repository traits in `capability_packs/test_harness/storage.rs`
4. rewrite `capability_packs/test_harness/storage/sqlite.rs` schema-dependent code
5. rewrite the production-ingest path in `cli/devql.rs` + `host/devql/ingestion/`
6. rewrite `capability_packs/test_harness/mapping/materialize.rs` and `capability_packs/test_harness/ingest/tests.rs`
7. rewrite `capability_packs/test_harness/ingest/coverage.rs`
8. rewrite `capability_packs/test_harness/ingest/results.rs`
9. rewrite `capability_packs/test_harness/query.rs` and storage backends
10. fix tests
11. update docs

# Final instruction block to give codex

Implement Stage 1 of TestLens storage refactoring.

Requirements:

- separate production and test persistence
- make TestLens production schema identical to `bitloops` production schema
- keep test-domain entities in dedicated tables
- preserve current discovery, linkage, query, confidence, strength, and classification behavior
- do not keep mixed `artefacts` storage for tests
- do not implement backward DB migration
- update tests and docs accordingly

If you want, I can also turn this into a shorter “Codex task prompt” version with imperative steps only, ready to paste directly.

# Post-Stage-1 next steps

As of March 18, 2026, the Stage 1 objective above is complete:

- production and test storage are separated
- the production schema matches `bitloops`’s production schema
- the current CLI flow still works on the real Ruff fixture
- the Stage 1 docs and validation notes have been updated to the split schema

The next milestone is no longer another Stage 1 refactor. The next milestone is to move TestLens closer to the intended runtime architecture where `bitloops` is the production source of truth.

## Immediate follow-up before Stage 2

There is one Stage 1 loose end worth closing first:

1. trace the remaining `3` normalized duplicate production artefact identities reported by Ruff production ingest
2. determine whether those `3` cases are:
   - legitimate identity collisions that still need a semantic-identity fix
   - acceptable normalization cases that should be documented
   - ingest/reporting mismatches that should be removed
3. record the outcome in `progress.txt` before starting the integration work below

This should be treated as the first next step.

Current status from the March 18, 2026 Ruff investigation:

- the remaining `3` normalized duplicate artefact identities are all cfg-gated alternate definitions, not another broad semantic-identity bug
- the current Ruff cases are:
  - `find_user_settings_toml` in `crates/ruff_workspace/src/pyproject.rs`
  - `SarifResult::from_message` in `crates/ruff_linter/src/message/sarif.rs`
  - `now` in `crates/ruff_db/src/system/memory_fs.rs`
- in all three cases, Ruff defines separate `wasm32` and non-`wasm32` implementations with the same semantic name/signature
- for now, treat these as acceptable normalization cases and keep the CLI note honest about them
- revisit cfg-aware identity only if Stage 2 integration with `bitloops` requires stricter parity

Current status from the March 18, 2026 fresh-DB `bitloops` ingest check:

- the real production-ingest entrypoints are `bitloops devql init` and `bitloops devql ingest`
- the default repo-local stores are:
  - `.bitloops/stores/relational/relational.db`
  - `.bitloops/stores/event/events.duckdb`
- on a brand-new DevQL DB for this repo, `bitloops devql ingest` inserts the `repositories` row but processes `0` checkpoints and writes `0` commits, file-state rows, artefacts, or edges
- a second clean repro on a disposable git repo confirmed the same boundary with a normal git commit but no Bitloops checkpoint state:
  - `bitloops devql ingest` completed with `checkpoints_processed=0` and `artefacts_upserted=0`
  - relational counts after ingest were `repositories=1`, `checkpoints=0`, `commit_checkpoints=0`, `commits=0`, `artefacts_current=0`
- that means a plain git commit is not sufficient for DevQL production materialization
- a disposable `rust-lang/cfg-if` run also validated the positive path:
  - a shell-driven Codex session-start / edit / stop / commit cycle produced committed Bitloops checkpoint rows
  - `bitloops devql ingest` then materialized production rows in the repo-local SQLite DB
  - `testlens init` and `testlens ingest-tests` successfully continued on that same DB
- the reason is structural, not a new bug: this repo currently has no committed checkpoint metadata for DevQL to replay, so `checkpoints`, `commit_checkpoints`, and `checkpoint_blobs` remain empty
- `bitloops` production materialization is checkpoint-driven:
  - read committed checkpoint summaries from the relational store
  - map checkpoint IDs to commit SHAs through `commit_checkpoints`
  - resolve `files_touched`
  - read git blob state at each mapped commit
  - upsert `commits`, `file_state`, `artefacts`, `artefacts_current`, and edge tables
- the contrast case also holds: once committed checkpoint rows exist, `bitloops devql ingest` does materialize production rows normally
- this is not equivalent to the current TestLens prototype ingest path, which scans a repo and commit directly
- default embedding behavior is also relevant: when not disabled, `bitloops` boots a local Jina embedding model under `.bitloops/embeddings/models` before checkpoint processing starts
- `testlens init` already succeeds against a Bitloops-created relational DB, so the same SQLite file can hold both the DevQL production tables and the TestLens test-domain tables
- `testlens ingest-tests` still fails fast when no production artefacts exist for the requested commit; the current error text still says to run `testlens ingest-production-artefacts` first, which should be corrected once the Bitloops-backed production path is the intended flow

## Stage 2 objective

Make `bitloops` the runtime source of truth for the production domain, while TestLens continues to own the test-domain tables and query behavior.

In practical terms, that means:

1. TestLens should stop being the primary producer of production artefacts for the real workflow.
2. TestLens should read production state from the `bitloops`-owned schema:
   - `repositories`
   - `commits`
   - `file_state`
   - `current_file_state`
   - `artefacts`
   - `artefacts_current`
   - `artefact_edges`
   - `artefact_edges_current`
3. TestLens should keep owning only:
   - `test_suites`
   - `test_scenarios`
   - `test_links`
   - `test_runs`
   - `test_classifications`
   - `coverage_captures`
   - `coverage_hits`
   - `test_discovery_runs`
   - `test_discovery_diagnostics`

## Stage 2 implementation order

Take Stage 2 in these steps, in order.

### Step 1. Close the duplicate-identity investigation

- identify the exact Ruff artefacts behind the remaining `3` normalized duplicate identities
- fix them if they reflect broken semantic identity
- otherwise document them explicitly and make sure the CLI output is honest about what is being normalized

### Step 2. Define the integration boundary

- decide how TestLens will discover and open the `bitloops` production database
- decide whether TestLens will:
  - read the production schema directly in place, or
  - attach/import production state into a TestLens-owned DB
- decide whether TestLens will require already-materialized production rows, or whether it will treat missing DevQL state as a verification/bootstrap failure that points the user at `bitloops devql ingest`
- document failure behavior when production state for a repo/commit is missing
- explicitly account for the fact that `bitloops` production ingest is checkpoint-driven and may legitimately produce only a repository row on a fresh repo with no committed checkpoint history
- document a Ruff quickstart for this boundary:
  - Bitloops checkpoint creation in the Ruff repo
  - `bitloops devql ingest` to materialize production rows
  - `testlens init` and `testlens ingest-tests` against the same relational DB

The preferred direction is direct read compatibility, not schema-copying again.

### Step 3. Add a production-read path

Implement a production repository path that reads production artefacts from `bitloops` storage instead of relying on TestLens’s own production-ingest command.

At minimum:

- resolve repo and commit state from the production DB
- resolve production query targets from `artefacts_current` / `artefacts`
- resolve production metadata needed by test linking and coverage joins

### Step 4. Narrow or deprecate `ingest-production-artefacts`

Once the production-read path exists:

- stop treating `testlens ingest-production-artefacts` as the real production path
- either:
  - keep it only as a local prototype/bootstrap tool, clearly marked as such, or
  - replace it with a thin compatibility/verification command that checks production state already exists

### Step 5. Revalidate the real Ruff flow

Run the same Ruff validation again, but with production data supplied by `bitloops` semantics instead of TestLens’s internal production ingest.

Acceptance for this step:

1. `ingest-tests` still works against the production source of truth
2. `query` still resolves production artefacts correctly
3. coverage ingestion still joins against production artefacts correctly
4. the F523 and ERA001 validation stories still produce the expected functional results

## Stage 2 non-goals

Do not do these while starting Stage 2:

- do not redesign query output or scoring
- do not add transitive linkage
- do not add new coverage modes
- do not merge all TestLens tables into `bitloops`
- do not expand language scope

## Current recommended next action

Start with the duplicate-identity investigation, then move directly into defining the production DB integration boundary.
