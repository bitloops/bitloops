# DevQL core ↔ capability pack boundaries

This document records the **target architecture** for keeping the DevQL **core** independent of concrete packs, while giving packs safe access to storage and a **controlled** path for cross-pack reuse.

**Related:** [Capability pack implementation gaps](./devql-capability-packs-implementation-gaps.md) (items 2–3), Confluence compass links there.

---

## Goals

1. **Low coupling** — Core defines stable traits (`CapabilityPack`, handlers, contexts, ports). Core does not import `knowledge`, `semantic_clones`, etc. A **composition root** registers `dyn CapabilityPack` implementations.
2. **Pack data access** — Each pack owns a **namespace** (tables / prefixes / migrations). The host provides **scoped ports** so handlers see only what the invocation allows (e.g. DevQL relational only for the **invoking** capability during ingest).
3. **Third-party safety** — **Timeouts** on stage, ingester, and composition subqueries; **depth limits** on DevQL composition (existing); optional **strict** cross-pack rules. Stronger isolation (WASM / separate process) remains a future adapter.
4. **Optional cross-pack reads** — User-configured **grants** allow capability `B` to invoke registered stages owned by `A` **without** a static `CapabilityDependency` edge, when explicitly listed in config.

---

## Typed host ports vs generic storage kinds

Prefer **operation-scoped or capability-scoped ports** over exposing raw “Relational” / “Events” buckets to every handler. Generic datasource kinds are acceptable **only** as **policy-bound implementations** (namespace checks, allowlisted tables), not as unconstrained SQL.

**Direction:** neutral **`relational()`** / **`documents()`** on **`KnowledgeIngestContext`** / **`KnowledgeExecutionContext`** only; core **`Capability*Context`** traits omit those ports. **`devql_relational` / `devql_relational_scoped`** remain on ingest for pack-scoped DevQL relational access. Migrations still use a single **`CapabilityMigrationContext`** that includes knowledge stores until further split.

---

## Cross-pack access: dependencies vs config grants

- **Descriptor `dependencies`** — Maintainer declares that pack `X` may depend on stages owned by `Y` (compile-time / ship-time contract).
- **`host.cross_pack_access` (config)** — User explicitly allows `from_capability` → `to_capability` for `resource: "devql_registered_stage"` and `mode: "read"`.

**Validation rule (implemented):** For a composed registered stage, allow invocation if **caller == stage owner**, or **caller declares a dependency on the stage owner**, or a **matching config grant** exists.

Example (merged into the same JSON root as `knowledge`, under `host`):

```json
{
  "host": {
    "invocation": {
      "stage_timeout_secs": 120,
      "ingester_timeout_secs": 300,
      "subquery_timeout_secs": 60
    },
    "cross_pack_access": [
      {
        "from_capability": "test_harness",
        "to_capability": "knowledge",
        "resource": "devql_registered_stage",
        "mode": "read"
      }
    ]
  }
}
```

*Note:* Today the host builds a default `host` object in code; wiring **user file merge** into `config_root` is a follow-up so overrides land in `LocalCapabilityRuntimeResources` without hand-editing Rust.

---

## Invocation policy (timeouts)

Configured under `host.invocation` in the merged capability config root (see defaults in `runtime_contexts::build_capability_config_root`):

| Field | Role |
|--------|------|
| `stage_timeout_secs` | Wall-clock limit for `invoke_stage`. |
| `ingester_timeout_secs` | Wall-clock limit for `invoke_ingester` / `invoke_ingester_with_relational`. |
| `subquery_timeout_secs` | Wall-clock limit for `execute_devql_subquery` (composition). |

Failures surface as structured errors (`… timed out after …`).

---

## Scoped DevQL relational (ingest)

During ingester dispatch, the host sets **`invoking_capability_id`**. Handlers that need DevQL relational storage must call **`devql_relational_scoped(expected_capability_id)`** so the storage handle is only usable when the active invocation matches that pack (reduces accidental cross-pack use when multiple ingesters share code paths).

---

## Pack lifecycle diagnostics (CLI)

From a configured repo, **`bitloops devql packs`** prints a snapshot of the **`DevqlCapabilityHost`**: registered pack ids, stages, ingesters, migration steps, schema modules, health check names, query-example counts, invocation timeouts, and **`host.cross_pack_access`** grants. Use **`--json`** for machine-readable output (same top-level shape as **`HostRegistryReport`** unless **`--with-extensions`** is set, which emits **`PackLifecycleReport`** including **`core_extension_host`**). **`--with-extensions`** appends **`CoreExtensionHost`** registry data (language packs, extension capability descriptors, readiness, diagnostics). **`--with-health`** runs DevQL host health checks; **`--apply-migrations`** runs DevQL pack migrations before reporting. Implementation: **`capability_host::diagnostics`**, **`DevqlCapabilityHost::registry_report`**, **`CoreExtensionHost::registry_report`** (+ **`CoreExtensionHostRegistryReport`**), **`run_capability_packs_report`**.

---

## Relational DDL: Postgres bootstrap vs SQLite pack migrations (semantic stack)

DevQL uses **two complementary mechanisms** for first-party capability tables on the relational backend:

| Mechanism | When it runs | Semantic clones stack |
|-----------|----------------|------------------------|
| **Relational bootstrap** (`ingestion/schema/relational_initialisation.rs` + related SQL helpers) | `init` / first connection for **Postgres** (and shared SQLite base tables) | Creates **`symbol_semantics`**, **`symbol_features`**, **`symbol_embeddings`**, and **`symbol_clone_edges`** (and extensions such as `vector` where required) so a fresh Postgres DSN is consistent before ingest. Init functions live in **`capabilities/semantic_clones/{stage_semantic_features,stage_embeddings,pipeline}`** and are invoked from bootstrap. |
| **Capability pack migrations** (`DevqlCapabilityHost::ensure_migrations_applied_sync`, per-pack `migrations/`) | After bootstrap, for **all** backends the host runs registered pack migrations | **SQLite** paths apply **`semantic_clones`** pack DDL (e.g. **`symbol_clone_edges`**) via migrations; Postgres clone tables are still created early in bootstrap for operational parity. |

**Rule of thumb:** if a table must exist **before** any pack migration runs on a given backend, it belongs in bootstrap for that backend; pack migrations remain the **versioned** path for pack-owned schema and for SQLite-first flows.

---

## Future work

- Replace pack-branded context accessors with **neutral gateway traits** + capability-bound handles.
- **Versioned** stage/subquery input/output schemas and **byte/row budgets** beyond timeouts.
- Enforce **read-only** composition paths where applicable.
- **Audit log** for cross-pack grants.

---

*Last updated: 2026-03-21 (packs diagnostics CLI).*
