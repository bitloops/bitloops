# Test Harness Prototype Decisions

Last updated: 2026-03-13

This document records the current prototype decisions that are already reflected in the codebase.
These are implementation-level defaults for the prototype, not final tuned product decisions.

## Query transport

- Prototype transport is CLI JSON.
- The query entrypoint is `testlens query`.
- Supported query views are `full`, `summary`, `tests`, and `coverage`.

## Query-level behavior

- `summary` returns only `artefact` and `summary`.
- `tests` returns `artefact`, `covering_tests`, and `summary`.
- `coverage` returns only `artefact` and `coverage`.
- `full` returns the combined payload.

## Noise control

- Default `min_strength = 0.3` for `tests` and `full` views.
- `summary` and `coverage` do not apply strength filtering.
- `--min-strength 0.0` is the explicit override for the full unfiltered match set.
- Summary counts are computed before the strength filter is applied.
- Summary includes coverage percentages when coverage data exists for the artefact.

## Static linkage strategy

- MVP static linkage is direct-only.
- The current implementation links tests to production artefacts from import-path resolution plus scenario call-site matching.
- Transitive relevance is not added during static-link construction.
- Rationale: keep static links precise and let coverage data represent deeper execution reach.

## Confidence and strength

- `confidence` is prototype-scored from whether the test/artefact pair has coverage rows and whether the pair has covered rows.
- `strength = classification_weight / fan_out`.
- Current classification weights:
  - `unit = 1.0`
  - `integration = 0.7`
  - `e2e = 0.4`
- Call-chain distance is not yet part of strength scoring.

## Classification thresholds

- Current prototype thresholds are:
  - `unit`: fan-out `1..=3` and boundary crossings `<= 1`
  - `integration`: fan-out `4..=10` and boundary crossings `1..=3`
  - `e2e`: fan-out `>= 11` and boundary crossings `>= 3`
- Fallback logic still promotes higher fan-out or higher boundary crossing counts upward when the first-range checks do not match cleanly.
- These thresholds are now explicit code-level constants in `src/domain/mod.rs`.

## Verification-level thresholds

- `untested`: no covering tests after the classification filter
- `partially_tested`: covering tests exist and branch coverage is below `50%`, or no coverage has been ingested yet
- `well_tested`: covering tests exist and branch coverage is at least `50%`
- The `50%` threshold is now a named constant in `src/read/query_test_harness.rs`.

## Query error contract

- `Repository not indexed` is returned when the requested commit has no indexed artefacts yet.
- `Artefact not found` is returned when the commit is indexed but the requested artefact cannot be resolved.
- These error paths are covered in the TypeScript query-layer BDD and the full SQLite-backed TypeScript journey.

## Coverage ingestion and query behavior

- Coverage ingestion is commit-addressed.
- Coverage is joined to artefact spans, so same-file artefacts are queried independently.
- Rust stable coverage is currently generated through `cargo llvm-cov --lcov`.
- Rust stable coverage in this prototype is line-coverage reliable.
- Uncovered branch assertions are currently validated on the TypeScript fixture where branch data is present in LCOV.

## Deferred decision areas

- Framework-agnostic run-outcome ingestion and scoring parity across languages
- Final tuning of `min_strength`
- Final tuning of classification thresholds
- Final tuning of `verification_level` thresholds
- Whether dynamic instrumentation belongs in MVP or post-MVP
- Final taxonomy positioning for `BDD`, `smoke`, and `acceptance` labels beyond the current unit/integration/e2e prototype model
