# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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

### Changed

- DevQL Semantic Clones: stages **1–2** persistence moved to **`capabilities/semantic_clones/stage_semantic_features.rs`** and **`stage_embeddings.rs`** (replacing **`ingestion/semantic_*_persistence.rs`**); stage **3** remains **`capabilities/semantic_clones/pipeline.rs`**. **`rebuild_symbol_clone_edges`** is re-exported on **`crate::engine::devql`** under **`cfg(test)`** for **`devql::tests`**. Relational bootstrap calls pack **`init_*_schema`** for semantic tables; Postgres clone init uses **`pipeline::init_postgres_semantic_clones_schema`**. Documented Postgres bootstrap vs SQLite pack migrations in **`docs/devql-core-pack-boundaries.md`**. Removed unused **`engine/devql/ingest.rs`** and **`ingestion/semantic_clones_persistence.rs`**.
- DevQL Test Harness pack: **`builtin_packs(repo_root)`** opens an optional relational handle shared by all test-harness ingesters; **`test_harness.linkage`** / **`classification`** / **`summaries`** mirror the coverage pattern (store unavailable → **`test_harness_relational_store_unavailable`**). Linkage runs **`ingest_tests::execute`**; classification runs **`rebuild_classifications_from_coverage`**; summaries returns **`load_test_harness_commit_counts`** (+ coverage presence). **`bitloops testlens ingest-tests`** / **`ingest-coverage`** / **`ingest-coverage-batch`** invoke the host ingesters.
- DevQL capability host: split **core** vs **knowledge** execution/ingest contexts (`KnowledgeExecutionContext`, `KnowledgeIngestContext`); knowledge stages/ingesters register via `register_knowledge_stage` / `register_knowledge_ingester` and traits `KnowledgeStage` / `KnowledgeIngester` so non-knowledge packs no longer have `relational()` / `documents()` on the base context traits.
- Added self-hosted runners
- Manual commit checkpoint flows are now fully DB-driven and trailer-free (`CLI-1357`), including temporary/committed checkpoint writes, checkpoint read paths, and `post_commit()` mapping via `commit_checkpoints`; legacy git-based checkpoint/shadow-branch storage paths and commit hook side effects have been removed.
- Artefacts are now updated in real time whenever someone changes them and saved in the artefacts_current and artefact_edges_current tables. CLI-1391 is complete and enums are used instead of strings.
- Implemented watch to reuse existing runtime in DevQL
- Fixed DevQL interface to query from the correct table depending on the query.

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
