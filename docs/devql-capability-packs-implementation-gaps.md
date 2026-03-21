# DevQL capability packs: implementation gaps and resolution order

This document captures an architecture review of first-party capability packs against the internal compass pages, ordered by **recommended sequence of work**. Each item includes a **suggested approach** to close the gap.

**References (Confluence):**

- [DevQL Capability Packs: Architecture and Implementation Compass](https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/452591621/DevQL+Capability+Packs+Architecture+and+Implementation+Compass)
- [DevQL Capability-Pack Registry and Host Contexts](https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/454983699/DevQL+Capability-Pack+Registry+and+Host+Contexts+Architecture+and+Implementation+Compass)
- [Knowledge Capability Pack](https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/455671814/Knowledge+Capability+Pack+Architecture+and+Implementation+Compass)
- [Semantic Clones Capability Pack](https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/455344152/Semantic+Clones+Capability+Pack+Architecture+and+Implementation+Compass)
- [Test Harness Capability Pack](https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/454983721/Test+Harness+Capability+Pack+Architecture+and+Implementation+Compass)

**Related:** [Grep-driven boundary audit](./devql-capability-packs-grep-audit.md) (Knowledge vs Test Harness capability modules, `devql/ingestion` table access).

**Target boundaries (core ↔ packs, timeouts, optional cross-pack grants):** [devql-core-pack-boundaries.md](./devql-core-pack-boundaries.md). Implemented in-repo (2026-03-20): `HostInvocationPolicy` + `with_timeout` on stage/ingester dispatch; `execute_devql_subquery` wall-clock limit via `DevqlSubqueryOptions::subquery_timeout`; `host.cross_pack_access` grants **or** descriptor `dependencies` for registered-stage composition; `devql_relational_scoped` binds DevQL relational to the invoking ingester capability id; default `host` config block in `build_capability_config_root`.

---

## 1. Dual capability-pack integration models (highest leverage)

**Update (2026-03-20):** Semantic Clones is registered on **`DevqlCapabilityHost`** (`semantic_clones` pack: ingester `semantic_clones.rebuild`, SQLite DDL via pack migration, ingest calls `invoke_ingester_with_relational`). It is **no longer** registered on **`CoreExtensionHost`**. Postgres `symbol_clone_edges` DDL remains in relational bootstrap; extension-style descriptor helpers in `engine/capability_packs/builtin/semantic_clones.rs` remain for provider builders and tests.

**Finding (historical):** Two parallel mechanisms existed: **DevQL capability host** vs **Core extension host** capability descriptors, with **Semantic Clones** incorrectly living on the extension path.

**Status:** **Closed for Semantic Clones (option A).** `builtin_packs(repo_root)` registers **Knowledge**, **Test Harness**, and **Semantic Clones** on `DevqlCapabilityHost`. `CoreExtensionHost::bootstrap_builtins()` registers **only** Knowledge and Test Harness descriptors (for compatibility/readiness); it does **not** register Semantic Clones. Descriptor helpers in `engine/capability_packs/builtin/semantic_clones.rs` remain for provider builders and tests, not extension-host registration.

**Residual (acceptable unless you want full unification):** Knowledge and Test Harness still have **two registration surfaces** (extension descriptors + DevQL `CapabilityPack`). That is separate from the original “Semantic Clones in the wrong lane” problem.

**Why address first:** This split determines every other boundary (lifecycle, contexts, migrations, observability). Without alignment, “pack” means different things in different subsystems.

**Suggested approach:**

- **Decide a target:** Either (A) **migrate Semantic Clones** onto `DevqlCapabilityHost` with pack-scoped ingesters, migrations, and storage gateways, or (B) **document and formalise** “extension descriptor + core ingestion orchestration” as an allowed first-party pattern with explicit rules (what must never bypass host policy, how pack-scoped schema is named, how readiness is reported).
- If (A): introduce ingestion entry points that invoke **`IngesterHandler`** with **`CapabilityIngestContext`** and move schema init under **pack migrations** orchestrated by the host.
- If (B): add a short **architecture decision record** in-repo and ensure new packs do not accidentally follow the wrong lane.

---

## 2. Host execution + migration contexts: core vs knowledge split (done)

**Update (2026-03-20):** **`CapabilityExecutionContext`** / **`CapabilityIngestContext`** are **core-only** (repo, graph, config, blobs, connectors, DevQL relational helpers, etc.). Knowledge relational + document ports live on **`KnowledgeExecutionContext`** / **`KnowledgeIngestContext`** (supertraits of the core traits). The host registers knowledge contributions via **`register_knowledge_stage`** / **`register_knowledge_ingester`** and dispatches with the knowledge context type; other packs use **`StageHandler`** / **`IngesterHandler`** and never see **`relational()`** / **`documents()`** on the core trait.

**Update (2026-03-21, migrations):** **`CapabilityMigrationContext`** is **core-only** (repo, repo root, **`apply_devql_sqlite_ddl`**). Knowledge store ports are on **`KnowledgeMigrationContext`** (supertrait). Pack migrations use **`MigrationRunner::Core(...)`** vs **`MigrationRunner::Knowledge(...)`**; **`run_migrations`** requires **`KnowledgeMigrationContext`** so knowledge migrations can run in the same ordered pass as core migrations.

**Residual:** If multiple non-knowledge packs need first-class relational/document ports during migrations, introduce a **`CapabilityStorageGateway`** (or per-capability handles) **namespaced by capability id**, matching the registry doc.

**Suggested approach (remaining):**

- **Medium term:** When cross-pack migration storage is needed, add namespaced gateways rather than widening **`CapabilityMigrationContext`** again.

---

## 3. Test Harness: registration shell vs incomplete ingest/migrate/storage story

**Update (2026-03-21):** **Closed for coverage ingest + migrations + relational surface.** The pack applies **real test-domain DDL** via `CapabilityMigrationContext` (bumped migration version). **`TestHarnessCoverageGateway`** is the narrow write/read surface used by LCOV / LLVM JSON ingest; **`BitloopsTestHarnessRepository`** implements it. **`TestHarnessPack::new(repo_root)`** opens an optional **`Arc<Mutex<BitloopsTestHarnessRepository>>`** when store config resolves; the **`test_harness.coverage`** ingester locks that handle and calls **`ingest_coverage::execute`**. **`bitloops testlens ingest-coverage`** / **`ingest-coverage-batch`** go through **`DevqlCapabilityHost::invoke_ingester`** (not a parallel command-only repo open). If the relational store cannot be opened, the ingester returns a structured failure with reason **`test_harness_relational_store_unavailable`**.

**Update (2026-03-21, follow-up):** **Linkage, classification, and summaries ingesters** use the same **`Option<Arc<Mutex<BitloopsTestHarnessRepository>>>`** pattern. **`test_harness.linkage`** runs **`ingest_tests::execute`** (test discovery + **`replace_test_discovery`**); **`bitloops testlens ingest-tests`** invokes that ingester. **`test_harness.classification`** calls **`rebuild_classifications_from_coverage`**. **`test_harness.summaries`** is a **read-only snapshot** of per-commit row counts (**`TestHarnessQueryRepository::load_test_harness_commit_counts`**) plus **`coverage_exists_for_commit`** (no extra materialized summary tables).

**Finding (historical):** The pack correctly registered stages, ingesters, schema, examples, health, and migrations, but ingest paths and migrations were incomplete relative to the compass.

**Residual:** Stages under **`dependency_gated_stage_response`** remain gated; optional **`TestHarnessIngestContext`** supertrait if you want compile-time separation from the shared mutex handle.

**Suggested approach (if extending further):**

- If a compile-time split is desired, add a **`TestHarnessIngestContext`** supertrait (like Knowledge) instead of passing the repo only through the ingester closure.

---

## 4. Semantic Clones: pipeline location vs “pack owns enrichers and storage”

**Update (2026-03-21):** **Clone-edge rebuild orchestration** moved from **`ingestion/semantic_clones_persistence.rs`** to **`capabilities/semantic_clones/pipeline.rs`**. **`devql ingest`** already triggered rebuild via **`invoke_ingester_with_relational`**; the **ingester** now calls **`pipeline::rebuild_symbol_clone_edges`** directly. **`crate::engine::devql::rebuild_symbol_clone_edges`** is a **`#[cfg(test)] pub(crate)`** re-export for **`devql::tests`**. Postgres relational bootstrap calls **`pipeline::init_postgres_semantic_clones_schema`**. The duplicate/unused **`engine/devql/ingest.rs`** file was removed.

**Update (2026-03-21, follow-up):** **Stage 1–2 persistence** moved from **`ingestion/semantic_*_persistence.rs`** to **`capabilities/semantic_clones/stage_semantic_features.rs`** and **`stage_embeddings.rs`**, re-exported from **`capabilities/semantic_clones/mod.rs`** for **`run_ingest`**. Relational bootstrap calls those modules’ **`init_*_schema`** functions. **Postgres vs SQLite DDL split** is documented in [devql-core-pack-boundaries.md](./devql-core-pack-boundaries.md#relational-ddl-postgres-bootstrap-vs-sqlite-pack-migrations-semantic-stack).

**Finding (historical):** Clone-edge build and persistence lived under **`devql/ingestion`**, while the compass positions Semantic Clones as owning enrichers, embeddings, clone edges, and pack-scoped migrations.

**Residual:** **Persistence** for stages 1–2 now lives under **`capabilities/semantic_clones`**; **embedding/LLM provider construction** still flows through **`semantic_clones_pack`** from core ingest — continue converging orchestration on pack builders where practical.

**Suggested approach (remaining):**

- Optional: narrow **`devql ingest`**’s direct use of **`RelationalStorage`** for semantic stages via a small gateway if you want stricter core/pack compile-time separation.

---

## 5. Knowledge: strong pattern; guard against store leakage and duplication

**Finding:** Knowledge **services** generally respect **`CapabilityIngestContext` / `CapabilityExecutionContext`** and gateways — aligned with the Knowledge compass. Storage implementations (`capabilities/knowledge/storage/*`) use **rusqlite** / **duckdb** directly, which is acceptable **if** they are only constructed by the host and exposed as **`dyn Knowledge*Gateway`**.

Risk: other packs or core paths opening the **same files** with ad hoc SQL instead of new gateways.

**Update (2026-03-21):** **`capabilities/knowledge/mod.rs`** documents the hygiene contract (gateways + **`CapabilityConfigView`** + provenance). Persisted ingestion and association provenance JSON now includes **`capability_version`** and **`api_version`** from **`KNOWLEDGE_DESCRIPTOR`**, and refresh ingests stamp **`knowledge.refresh`** (add path **`knowledge.add`**) instead of reusing the add label.

**Update (2026-03-21, follow-up):** **`CapabilityIngestContext::invoking_ingester_id`** + host wiring populate **`ingester_id`** and **`invoking_capability_id`** on persisted provenance for **`DevqlCapabilityHost::invoke_ingester`** runs. Removed dead **`plugin.rs`**, **`providers/`**, and orphan **`tests.rs`**; knowledge fetch stays on **`engine::adapters::connectors`**.

**Suggested approach:**

- Keep **all** relational/DuckDB paths for knowledge behind **`RelationalGateway`** / **`DocumentStoreGateway`** (implemented only by knowledge storage types from the host); reject new direct store usage outside `capabilities/knowledge/storage/` in review.
- When adding **config**, prefer **`CapabilityConfigView`** (already used in health paths) consistently; avoid parsing raw repo config inside pack logic except through the view.
- **Optional later:** add a per-invocation **trace / run id** (UUID) on the ingest runtime if you need cross-service correlation beyond **`ingester_id`**.

---

## 6. Observability and lifecycle completeness

**Update (2026-03-21):** **`bitloops devql packs`** lists registered packs (descriptor, stages, ingesters, per-pack migrations, schema modules, health check names, query-example counts), host **invocation** timeouts and **`cross_pack_access`** grants, and the ordered **migration plan**. **`--json`** emits **`HostRegistryReport`** (unchanged shape when `--with-extensions` is off); **`--with-extensions`** adds a second section (human) or wraps **`PackLifecycleReport`** in JSON. **`--with-health`** runs DevQL host health checks; **`--apply-migrations`** runs the DevQL pack migration pass before reporting. **`--with-extensions`** builds **`CoreExtensionHost::registry_report`** (language packs, extension capability descriptors, migration plan, readiness, diagnostics). Implementation: **`capability_host::diagnostics`** (`PackLifecycleReport`), **`CoreExtensionHostRegistryReport`** / **`format_core_extension_host_registry_human`**, **`run_capability_packs_report`**.

**Finding (historical):** The registry doc calls for **diagnostics**: which packs loaded, contributions, migrations, health outcomes. `DevqlCapabilityHost` implements registration, collision checks, migration orchestration, and health invocation for **all three** built-in packs (including Semantic Clones), but **CoreExtensionHost** still tracks Knowledge/Test Harness separately, so **unified diagnostics** across both subsystems remain harder until consolidated or explicitly bridged.

**Update (2026-03-21, follow-up):** **`--with-extensions`** surfaces **`CoreExtensionHost`** in the same CLI. **`CoreExtensionHostError`** and several **`DevqlCapabilityHost`** `bail!` paths now use **`[subsystem:operation] … [contribution:id]`**-style prefixes for easier log triage.

**Residual:** Optional **structured log at boot** listing both hosts without a subprocess.

**Suggested approach (remaining):**

- Emit a **single boot diagnostic** (tracing or structured stderr) if you need continuous visibility beyond on-demand CLI.

---

## Summary: recommended order

| Order | Topic | Core action |
|------|--------|-------------|
| 1 | Dual pack models | **Done (SC):** Semantic Clones on `DevqlCapabilityHost` only; extension host does not register SC. Residual: K+TH still dual-registered (extension + DevQL). |
| 2 | Context surface area | **Done:** core vs **`Knowledge*Context`** for stages/ingesters (**`register_knowledge_*`**) and for migrations (**`CapabilityMigrationContext`** vs **`KnowledgeMigrationContext`**, **`MigrationRunner::Core` / `Knowledge`**) |
| 3 | Test Harness depth | **Done:** coverage + linkage + classification + summaries ingesters + `testlens ingest-tests` / coverage via `invoke_ingester`; gated stages remain |
| 4 | Semantic Clones structure | **Done:** stages 1–2 in `stage_semantic_features` / `stage_embeddings`; stage 3 in `pipeline`; ingest triggers `semantic_clones.rebuild`; Postgres bootstrap + SQLite migrations split documented |
| 5 | Knowledge hygiene | **Done (this pass):** module docs; provenance (`capability_version` / `api_version`, add vs refresh, host **`ingester_id`**); removed dead plugin/providers/orphan tests. **Ongoing:** review discipline (gateways + config view); optional trace id |
| 6 | Observability | **Done:** `bitloops devql packs` (+ `--with-extensions` for `CoreExtensionHost`, JSON wrap only with that flag); clearer error prefixes on host paths |

---

*Generated from an internal codebase review; update this file as gaps are closed.*
