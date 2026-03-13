# TestLens Agent Guide

Keep the codebase aligned with a compact CQRS structure.

## Boundaries

- `src/main.rs`, `src/lib.rs`, `src/cli.rs`, `src/app.rs` are boundary files.
- `src/app/commands/*.rs` is the write side. Handlers own use-case orchestration, parsing, and traversal.
- `src/app/queries/*.rs` is the query entry layer. Handlers delegate to the read side.
- `src/read/*.rs` contains read-model composition and response shaping.
- `src/domain/mod.rs` contains records shared across app and persistence boundaries.
- `src/repository/mod.rs` defines repository traits for both write and query access.
- `src/repository/sqlite.rs` contains the SQLite implementation. Keep SQL, `rusqlite`, transactions, and row mapping there.
- `src/db/*.rs` contains schema and DB bootstrap/setup helpers.

## Rules

- Keep command and query concerns separate. Reads do not mutate state. Writes do not own read-model formatting.
- Do not put raw SQL, `rusqlite`, or transaction logic in command or query handlers.
- Persistence details stay behind repository traits and concrete implementations such as SQLite.
- Read-side query composition may aggregate repository results, but it should not reach into SQLite directly.
- Repository methods should accept and return domain records from `src/domain/mod.rs`.
- Tree-sitter traversal, path resolution, fixture parsing, and linkage logic stay in handlers, not repositories.

## Naming

- Name files after the use case they implement.
- Prefer explicit names such as `ingest_production_artefacts.rs` and `query_test_harness.rs`.
- Avoid generic names like `utils` or `helpers`.

## Direction

- Rust remains the primary target fixture and should stay first-class in workflow decisions.
- Multi-language support should stay adapter-based rather than branching the whole pipeline.
- Unit tests stay inline under `src/`. Acceptance tests live under `tests/e2e/` with shared helpers in `tests/e2e/support/`.

## Hygiene

- If architectural boundaries change, update `docs/architecture/overview.md`.
- If the developer or fixture flow changes, update `DEVELOPMENT.md`.
- Keep the structure compact and explicit.
