# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **DevQL GraphQL bootstrap** (`CLI-1513`): added `async-graphql` and `async-graphql-axum`, introduced the initial `src/graphql/` schema scaffold, and mounted `/devql`, `/devql/playground`, `/devql/ws`, and `/devql/sdl` on the existing Axum server. The minimal schema now builds and executes in-process with a stub `repo(name)` query while the current REST dashboard routes remain available during the migration.
- **DevQL GraphQL shared context and health bootstrap** (`CLI-1514`): expanded the GraphQL foundation with a shared request context that resolves repository identity, backend configuration, blob storage, capability-host bootstrap state, and loader scaffolding. Added a `health` query that reports relational, events, and blob backend connectivity through the existing dashboard health probes, with smoke coverage for both in-process execution and the `/devql` HTTP endpoint.
- **DevQL GraphQL repository and commit read resolvers** (`CLI-1515`): added repository-level GraphQL reads for `defaultBranch`, `commits`, `branches`, `users`, and `agents`, together with typed `Commit`, `Checkpoint`, `Branch`, and connection/page-info objects. Commit reads now expose `filesChanged` and nested checkpoint access, cursor pagination returns structured GraphQL error codes for invalid cursors or arguments, and the in-process schema tests now cover happy-path, pagination, empty-state, and resolver-error behaviour.
- **DevQL GraphQL artefact, file, and dependency resolvers** (`CLI-1516`): added GraphQL `Artefact`, `FileContext`, and `DependencyEdge` types, repository-level `file`, `files`, and `artefacts` reads, and nested artefact tree and dependency-edge resolvers for current-code DevQL queries. The new GraphQL surface validates repository-relative paths and line-range filters, supports unresolved dependency targets and cursor pagination, and is covered by in-process tests for repository, file, artefact, and dependency query flows.

### Changed

- **DevQL test-harness stage boundary cleanup:** `coverage()` now requires `symbol_id` from upstream core artefact rows and no longer resolves coverage by falling back through `artefacts_current`. The direct `replace_production_artefacts` test-harness repository path and the dead `ProductionIngestionBatch` model were removed, with test-only production seeding moved to direct SQLite setup helpers.

### Fixed

## [0.0.11] - 2026-03-25

### Added

- **Unified multi-scope configuration** (`CLI-1459`): One configuration domain replaces the legacy `settings.json` / `config.json` split. All settings (hooks, strategy, stores, knowledge, semantic, dashboard, watch) now live in a single `config.json` / `config.local.json` file pair using a versioned envelope format (`version` / `scope` / `settings`). Layer precedence: code defaults → global `~/.bitloops/config.json` → project shared → project local → environment variables. Deep-merge semantics for objects; arrays replace entirely; explicit `null` clears keys from lower layers.
- **Monorepo project-root discovery** (`CLI-1463`): `bitloops_project_root()` walks upward from `cwd` to find the nearest `.bitloops/` directory marker. Falls back to git root when no marker exists. Git hooks still install at git root; config, stores, and agent directories resolve from the Bitloops project root.
- **Provider-less store backend model** (`CLI-1481`–`CLI-1484`): Removed `provider` enum fields from `RelationalBackendConfig`, `EventsBackendConfig`, and `BlobStorageConfig`. Local backends (SQLite, DuckDB, local blob) are always present; remote backends (Postgres, ClickHouse, S3, GCS) activate when their connection string or bucket is configured. Consumer dispatch sites use capability checks (`has_postgres()`, `has_clickhouse()`, `s3_bucket.is_some()`) instead of `match provider`.

- **`bitloops devql packs`**: inspect `DevqlCapabilityHost` registry (packs, stages, ingesters, migration plan, schema modules, health check names, query-example counts, invocation policy, cross-pack grants); **`--json`** (unchanged top-level **`HostRegistryReport`** unless **`--with-extensions`**), **`--with-health`**, **`--apply-migrations`**, **`--with-extensions`** (append **`CoreExtensionHost`** snapshot via **`CoreExtensionHostRegistryReport`**; JSON becomes **`PackLifecycleReport`**). **`CoreExtensionHost::registry_report`**, **`CoreExtensionHostError`** / DevQL capability host errors: **`[subsystem:…]`**-prefixed messages for triage.
- DevQL capability host: documented core↔pack boundaries ([`docs/devql-core-pack-boundaries.md`](docs/devql-core-pack-boundaries.md)); configurable `host.invocation` timeouts for stages, ingesters, and composition subqueries; optional `host.cross_pack_access` read grants for registered-stage composition alongside descriptor dependencies; `devql_relational_scoped` for ingester-bound DevQL relational access.
- DevQL now fully indexes code artefacts (functions, methods, classes, interfaces, structs, enums, traits, modules) for Rust and JS/TS, capturing rich metadata: fully-qualified symbol names, parent hierarchy, byte-precise location, signature, modifiers (async/static/visibility), and docstrings.
- DevQL tracks dependency edges between artefacts (exports, inheritance, references, calls) for both Rust and JS/TS, enabling cross-symbol graph queries.
- DevQL maintains both a **current snapshot** and full **historical record** of artefacts and edges, allowing point-in-time queries over the evolution of a codebase.
- Tree-sitter is now used as the parsing backend for all DevQL code extraction, providing accurate language-aware symbol resolution.
- Checkpoint migration is complete (`CLI-1357` and `CLI-1358` to `CLI-1367`): checkpoint/session persistence now uses relational storage (SQLite with optional PostgreSQL) plus blob storage backends (local filesystem, S3, and GCS) for transcripts, prompts, and context.
- Introduced Layered Extension Architecture foundations (`CLI-1426`) in the extension host: host compatibility contracts, lifecycle states, readiness reporting, and diagnostics are now first-class extension primitives.
- Added host-managed Language Pack registration and resolution (`CLI-1426`), including profile normalisation, alias resolution, source-version constraints, and extension-based profile matching.
- Added Capability Pack registry ownership validation and migration orchestration (`CLI-1426`) for stage, ingester, schema module, and query-example contributions.
- Updated DevQL Getting Started documentation with expanded field references and query examples.
- Improved the version command and added a `bitloops --version --check` flag to check for the latest version.
- Cut down the `bitloops dashboard` loading time by moving the host name detection from the DNS probe to the user-home config file (`~/.bitloops/config.json`).
- Updated Readme documentation
- Add documetnation around Contributing, Security & Code of Conduct

### Fixed

- **Global config fallback**: `load_effective_config` no longer falls back to `PathBuf::from(".")` when HOME is unset; the global layer is skipped entirely, preventing scope-validation failures that silently dropped all configuration.
- **Conflicting blob backend detection**: `create_blob_store_with_backend` now returns an error when both `s3_bucket` and `gcs_bucket` are set, instead of silently picking S3.

### Changed

- **Breaking — Test Harness (`CLI-1454`):** Removed the read-only **`test_harness.summaries`** ingester. Per-commit test-harness row counts and coverage presence are available from the DevQL stage **`test_harness_tests_summary()`** when the query resolves a commit (e.g. **`asOf(ref:...)`** / **`asOf(commit:...)`**). Ingesters remain for state-changing ingest only (linkage, coverage, classification).

- **Layered Extension Architecture restructuring (`CLI-1426`)**: The entire codebase has been restructured to make the four-layer architecture visible at the `src/` top level. The `engine/` mega-module (30 sub-modules, 350+ files) has been decomposed into `host/`, `capability_packs/`, `adapters/`, `storage/`, `telemetry/`, `config/`, `utils/`, `models/`, `git/`, `cli/`, and `api/`. Eliminated modules: `engine/`, `repository/`, `app/`, `read/`, `domain/`, `store_config/`, `terminal/`. All modules now use modern Rust 2018+ sibling-file convention (`name.rs` + `name/`) instead of legacy `mod.rs`.
- **Host is now pack-agnostic**: Removed all Knowledge-specific code from the capability host — `KnowledgeExecutionContext`, `KnowledgeIngestContext`, `KnowledgeMigrationContext`, `KnowledgeStage`, `KnowledgeIngester`, `register_knowledge_stage`, `register_knowledge_ingester`, `RegisteredStage::Knowledge`, `RegisteredIngester::Knowledge`, and `MigrationRunner::Knowledge` have been eliminated. Generic context traits now include optional `relational()` and `documents()` ports. All packs use the same `IngesterHandler` / `StageHandler` with `CapabilityIngestContext` / `CapabilityExecutionContext`.
- **Test Harness pack owns its storage**: `TestHarnessCoverageGateway` removed from host gateways. Repository traits, SQLite implementation, Postgres implementation, and the dispatch enum all moved into `capability_packs/test_harness/storage/`. Test mapping logic (`app/test_mapping/`) and ingestion commands (`app/commands/ingest_*`) consolidated into the pack.
- **Semantic Clones pack owns its analysis pipeline**: `engine/semantic_clones/`, `engine/semantic_embeddings/`, and `engine/semantic_features/` consolidated into `capability_packs/semantic_clones/{scoring,embeddings,features}/`. Extension descriptor helpers moved from `host/capability_packs/builtin/` into the pack.
- **Checkpoint-to-commit mapping is now fully DB-driven**: Auto-commit strategy no longer writes `Bitloops-Checkpoint`, `Bitloops-Session`, `Bitloops-Strategy`, `Bitloops-Metadata`, or `Bitloops-Agent` trailers into commit messages. Instead, `insert_commit_checkpoint_mapping()` records the mapping in the `commit_checkpoints` relational table after each commit. The rewind module uses `lookup_session_id_for_commit()` (DB join) instead of parsing session trailers from commit messages. `host/checkpoints/trailers.rs` has been deleted; only `checkpoint_id.rs` (validation utilities) remains.
- **Host internal restructuring**: `session/`, `strategy/`, `lifecycle/`, `transcript/`, `history/`, `summarize`, and `trailers` grouped under `host/checkpoints/`. `extensions/` renamed to `host/extension_host/`. `devql/capability_host/` promoted to `host/capability_host/`.
- **`include!` macro cleanup**: Converted `include!`-composed files in `host/devql/`, `host/checkpoints/strategy/`, `capability_packs/semantic_clones/`, `cli/explain/`, and `host/extension_host/` to proper Rust modules with explicit imports and visibility annotations.
- DevQL Semantic Clones: stages **1–2** persistence in `capability_packs/semantic_clones/stage_semantic_features.rs` and `stage_embeddings.rs`; stage **3** in `capability_packs/semantic_clones/pipeline.rs`. Relational bootstrap calls pack `init_*_schema` for semantic tables.
- DevQL Test Harness pack: `builtin_packs(repo_root)` opens an optional relational handle shared by test-harness ingesters; linkage / classification ingesters use the same pattern as coverage. `bitloops testlens ingest-tests` / `ingest-coverage` / `ingest-coverage-batch` invoke the host ingesters.
- Added self-hosted runners
- Manual commit checkpoint flows are now fully DB-driven and trailer-free (`CLI-1357`), including temporary/committed checkpoint writes, checkpoint read paths, and `post_commit()` mapping via `commit_checkpoints`; legacy git-based checkpoint/shadow-branch storage paths and commit hook side effects have been removed.
- Artefacts are now updated in real time whenever someone changes them and saved in the artefacts_current and artefact_edges_current tables. CLI-1391 is complete and enums are used instead of strings.
- Implemented watch to reuse existing runtime in DevQL
- Fixed DevQL interface to query from the correct table depending on the query.
- Fixed workspace revisions being persisted twice for a single change.
- DB schema definitions now include a content hash so the schema watcher detects changes automatically, removing the need for a manual restart.
- Edges deduplication in case of references

## [0.0.10] - 2026-03-12

- Added first-class Codex support (current hook parity: `SessionStart` and `Stop`), including `bitloops init --agent codex`, lifecycle/runtime dispatch wiring, and managed Codex hook installation in `.codex/hooks.json` (Codex matcher format) with idempotent install/uninstall that preserves user-defined hook entries.
- Dashboard `/api/commits` now returns all checkpoint session agents via `checkpoint.agents` and no longer exposes a singular `checkpoint.agent` value.
- Dashboard `/api/commits` now includes `checkpoint.first_prompt_preview` with the first 160 characters from the first prompt of the first checkpoint session, after stripping leading `<tag>...</tag>` blocks and trimming leading whitespace.
- Dashboard agent filtering/aggregation now evaluates all session agents per checkpoint, so `/api/commits` agent filters, `/api/agents`, and KPI agent counts reflect multi-session checkpoints correctly.
- Dashboard `files_touched` payloads in `/api/commits` (`commit.files_touched` and `checkpoint.files_touched`) and `/api/checkpoints/{checkpoint_id}` now return arrays of objects (`[{ filepath, additionsCount, deletionsCount }]`) instead of path-keyed maps or plain path arrays.
- DevQL database support has been extended with provider-based backends: relational storage now supports `sqlite` (default) or `postgres`, and events storage now supports `duckdb` (default) or `clickhouse`.
- Local DevQL setup now works out of the box with file-based defaults (`~/.bitloops/DevQL/relational.db` and `~/.bitloops/DevQL/events.duckdb`), reducing external database dependencies for local development.
- Existing PostgreSQL/ClickHouse configurations remain backward compatible via legacy `postgres_dsn` / `clickhouse_*` config keys and `BITLOOPS_DevQL_*` environment variables.

## [0.0.9] - 2026-03-09

- Reissued release after rollback of v0.0.8

## [0.0.8] - 2026-03-09

### Added

- Native Windows CMD installer at `scripts/install.cmd` with GitHub Releases download and SHA256 verification.
- Windows ARM64 (`aarch64-pc-windows-msvc`) release artifacts and installer support.
- DevQL query history injest in agent pre-hook.
- Added workflow to protect main branch from merges of other than develop branches.
