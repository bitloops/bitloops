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

## 2. Host execution contexts: core vs knowledge split (short term done)

**Update (2026-03-20):** **`CapabilityExecutionContext`** / **`CapabilityIngestContext`** are **core-only** (repo, graph, config, blobs, connectors, DevQL relational helpers, etc.). Knowledge relational + document ports live on **`KnowledgeExecutionContext`** / **`KnowledgeIngestContext`** (supertraits of the core traits). The host registers knowledge contributions via **`register_knowledge_stage`** / **`register_knowledge_ingester`** and dispatches with the knowledge context type; other packs use **`StageHandler`** / **`IngesterHandler`** and never see **`relational()`** / **`documents()`** on the core trait.

**Residual:** **`CapabilityMigrationContext`** still exposes **`relational()`** / **`documents()`** for all pack migrations in one ordered run (only Knowledge migrations use them today). Medium-term: namespaced migration callbacks or split migration context if non-knowledge migrations must not see those ports at compile time.

**Suggested approach (remaining):**

- **Medium term:** Introduce a **`CapabilityStorageGateway`** (or per-capability gateway handles) on the context, **namespaced by capability id**, matching the registry doc, if multiple packs need first-class document/relational ports.

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

**Finding:** Clone-edge build and persistence are orchestrated from **ingestion** with direct **relational** access patterns, while the compass positions Semantic Clones as a pack that owns **Stage 1–3 enrichers**, **vector/embeddings**, **clone edges**, and **pack-scoped migrations**, with model providers behind host-approved access.

**Suggested approach:**

- After item 1’s decision: if Semantic Clones remains partially in ingestion, **draw a clear line**: ingestion may **schedule** or **trigger** pack work, but **business logic and writes** should live in pack modules callable only through **context + gateways**.
- Move **schema creation** for `symbol_clone_edges` (and related tables) toward **pack migrations** or a single host-owned “relational bootstrap” that is explicitly **pack-scoped** in naming and ownership docs.
- Ensure **embedding/model** calls go through the same **approved provider** abstraction the compass describes, not ad hoc clients from deep in ingestion.

---

## 5. Knowledge: strong pattern; guard against store leakage and duplication

**Finding:** Knowledge **services** generally respect **`CapabilityIngestContext` / `CapabilityExecutionContext`** and gateways — aligned with the Knowledge compass. Storage implementations (`capabilities/knowledge/storage/*`) use **rusqlite** / **duckdb** directly, which is acceptable **if** they are only constructed by the host and exposed as **`dyn Knowledge*Gateway`**.

Risk: other packs or core paths opening the **same files** with ad hoc SQL instead of new gateways.

**Suggested approach:**

- Keep **all** relational/DuckDB paths for knowledge behind **`RelationalGateway`** / **`DocumentStoreGateway`** (implemented only by knowledge storage types from the host); reject new direct store usage outside `capabilities/knowledge/storage/` in review.
- When adding **config**, prefer **`CapabilityConfigView`** (already used in health paths) consistently; avoid parsing raw repo config inside pack logic except through the view.
- Track **provenance** fields on writes per compass (capability id/version/run) where not already stamped.

---

## 6. Observability and lifecycle completeness

**Finding:** The registry doc calls for **diagnostics**: which packs loaded, contributions, migrations, health outcomes. `DevqlCapabilityHost` implements registration, collision checks, migration orchestration, and health invocation for **all three** built-in packs (including Semantic Clones), but **CoreExtensionHost** still tracks Knowledge/Test Harness separately, so **unified diagnostics** across both subsystems remain harder until consolidated or explicitly bridged.

**Suggested approach:**

- Expose a **single CLI or debug query** (or structured log at boot) that lists: pack id, lifecycle state, registered stages/ingesters, migration version, health results — **including** extension-registered packs once item 1 is resolved.
- Ensure **failure modes** (validation, collision, migration failure) surface **pack id + contribution type + identifier** as the registry doc requires.

---

## Summary: recommended order

| Order | Topic | Core action |
|------|--------|-------------|
| 1 | Dual pack models | **Done (SC):** Semantic Clones on `DevqlCapabilityHost` only; extension host does not register SC. Residual: K+TH still dual-registered (extension + DevQL). |
| 2 | Context surface area | **Done (stages/ingesters):** core vs **`Knowledge*Context`** + **`register_knowledge_*`**; **open:** migration context still wide |
| 3 | Test Harness depth | **Done:** coverage + linkage + classification + summaries ingesters + `testlens ingest-tests` / coverage via `invoke_ingester`; gated stages remain |
| 4 | Semantic Clones structure | Pack-owned enrichers/storage boundaries vs ingestion triggers |
| 5 | Knowledge hygiene | Gateway-only store access; config/provenance consistency |
| 6 | Observability | One lifecycle/diagnostic story across all packs |

---

*Generated from an internal codebase review; update this file as gaps are closed.*
