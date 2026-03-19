# TestLens Migration into `bitloops_cli`

## Current status
- Migration complete: the supported test-harness surface now lives in `bitloops_cli`.
- DevQL owns production artefacts. The migrated test-harness path only reads production rows and writes test-domain tables.
- SQLite-first schema initialization is aligned with `bitloops devql init`.
- Shared test-harness code, fixtures, acceptance coverage, and the root quickstart now live under `bitloops_cli`.
- The old `TestLens` tree has been removed.
- The operational `claude-code` runbook is now validated end to end on a curated Rust fixture:
  checkpointed production change -> `bitloops devql ingest` -> checkpointed test change -> `bitloops testlens ingest-tests` -> non-zero `test_links`.

## Deletion tranche
1. Move the shared TestLens library code into `bitloops_cli` or a renamed shared crate.
   - Done: the shared engine now lives in `bitloops_cli/src`.
2. Replace test seeding that still depends on the old prototype production-ingest command shape.
   - Done: Bitloops-side acceptance tests seed production through test-only helpers under `bitloops_cli/tests/test_harness_support`, and the old runtime-side `ingest_production_artefacts` command module has been removed.
3. Retire the old `TestLens` e2e harness and stale feature/docs surface.
   - Done: the old standalone tree was removed after migrating the active Gherkin coverage and fixtures.
4. Delete the standalone binary files, and only after that delete the whole `TestLens` crate.
   - Done: the standalone binary layer and the obsolete crate directory are gone.

## Supported surface
- `bitloops testlens init`
- `bitloops testlens ingest-tests`
- `bitloops testlens ingest-coverage`
- `bitloops testlens ingest-coverage-batch`
- `bitloops testlens ingest-results`
- `bitloops testlens query`
- `bitloops testlens list`

## Follow-up work
- Keep expanding `bitloops_cli`-only acceptance coverage as the test harness evolves.
- Keep direct backend coverage on both relational backends:
  - SQLite via acceptance and Gherkin flows
  - Postgres via repository-level tests that exercise schema init, test discovery writes, runtime-signal writes, and query reads
- Keep the Ruff workspace quickstarts under `bitloops_cli/docs/test_harness/` so they do not get dropped during future refactors.
- Keep `runbook.md` aligned with the validated `claude-code` proof flow so checkpoint-scoped DevQL behavior and link creation stay obvious.
- Keep prototype-free boundaries clear: synthetic production seeding belongs in test-only support, while real runtime production ingest belongs to DevQL.
- Keep repository backends split by responsibility:
  - backend entrypoints and trait impls in `mod.rs`
  - SQL write helpers in focused helper modules
  - query/list helpers and backend-specific tests in separate files
- Keep `cargo clippy --all-targets -- -D warnings` green as the effective push gate for the migrated test-harness code, not just `cargo test`.
- Keep Postgres DDL honest:
  - Postgres-only schema strings should use real Postgres types such as `TIMESTAMPTZ` and `BIGINT`
  - Postgres repository queries should not rely on SQLite-specific `DISTINCT ... ORDER BY ...` behavior
- Prune or rewrite historical migration notes such as `compatibility-plan-1.md` when they stop being useful.
