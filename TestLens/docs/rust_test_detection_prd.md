# Rust Test Detection Completeness PRD

Last updated: 2026-03-16

## Summary

TestLens now covers the mainstream Rust declaration matrix in code and executable tests. The remaining work is no longer "add missing declaration styles"; it is broader real-workspace confidence and residual helper-attribution depth on benchmark tasks.

The current implementation is strong on source-structured discovery, hybrid enumeration with degraded fallback, and static linkage. This document defines the desired end-state, records the current implementation truth, and captures the architecture and rollout decisions that got the prototype to mainstream Rust test artefact detection completeness.

## Desired End-State

The target is complete detection for mainstream executable Rust tests:

- `#[test]`
- runtime `::test` variants such as `#[tokio::test]`
- inline `src/**.rs` test modules
- `#[test_case(...)]`
- `#[case(...)]`
- `#[wasm_bindgen_test]`
- `#[quickcheck]`
- `#[quickcheck_macros::quickcheck]`
- `#[rstest]`
- `proptest!`
- rustdoc doctests

Out of scope for this rollout:

- benches such as `criterion`, `divan`, and `cargo bench`
- fuzzers
- custom `harness = false` runners
- cross-framework run outcomes

This is a stronger and more credible target than "all Rust verification surfaces." It is also the level needed to avoid hidden blind spots in typical Rust production repositories.

## Current Coverage

The Rust ingester lives in `src/app/commands/ingest_tests.rs`.

It currently covers:

- plain `#[test]`
- runtime `::test` variants
- inline `src/**.rs` test modules
- `#[test_case(...)]` and `#[case(...)]` on function attributes
- `#[wasm_bindgen_test]`
- `#[quickcheck]`
- macro-generated quickcheck scenarios where a local `macro_rules!` definition emits a recognized test attribute
- `#[rstest]`, `#[case(...)]`, `#[values(...)]`, `#[files(...)]`, `#[template]`, and `#[apply(...)]`
- `proptest!`
- rustdoc doctests discovered from fenced Rust code blocks on public items
- hybrid `cargo test -- --list` and `cargo test --doc -- --list` enumeration with explicit source-only fallback when Cargo listing is unavailable or too slow

### Validation Status Matrix

| Pattern | Covered in code | BDD covered | SQLite e2e covered | Real Ruff workspace validated |
| --- | --- | --- | --- | --- |
| `#[test]` / runtime `::test` | Yes | Yes | Yes | Partial |
| Inline `src/**.rs` tests | Yes | Yes | Yes | Partial |
| `#[test_case(...)]` / `#[case(...)]` rule-harness cases | Yes | Yes | Yes | Partial |
| `#[wasm_bindgen_test]` | Yes | Yes | Yes | No |
| `#[quickcheck]` / macro-generated quickcheck | Yes | Yes | Yes | No |
| `#[rstest]` family | Yes | Yes | Yes | No |
| `proptest!` | Yes | Yes | Yes | No |
| rustdoc doctests | Yes | Yes | Yes | Partial |

### Synthetic vs Real-Workspace Truth

Synthetic coverage is already strong:

- `CLI-1368` proves inline `src` discovery, case-level scenarios, and Ruff-style rule-harness linkage on a synthetic Ruff-like fixture.
- `CLI-1369` proves `#[wasm_bindgen_test]` and macro-generated quickcheck scenarios on a synthetic Ruff-like fixture.
- `CLI-1381` proves `#[rstest]`, `proptest!`, doctests, and hybrid enumeration on a Cargo-backed Rust fixture with both BDD and SQLite-backed end-to-end coverage.

Fresh real-workspace validation on Ruff is materially better, but still not total:

- On March 16, 2026, a fresh Ruff database produced `873` suites, `4859` scenarios, and `64760` links from source discovery.
- `string_dot_format_extra_positional_arguments` now queries as `partially_tested` with linked F523 harness and doctest coverage.
- `remove_unused_positional_arguments_from_format_call` and `transform_expression` still query as `untested`.
- Ruff's `cargo test -- --list` and `cargo test --doc -- --list` paths currently time out in this workspace, so the CLI falls back to explicit `source-only` mode and reports that degraded state.

Conclusion:

- mainstream declaration detection is implemented and validated
- hybrid enumeration and degraded fallback are implemented and validated
- deeper helper attribution on benchmark tasks remains a separate residual gap

## Why Source-Only Is Not Enough

The current design is Tree-sitter and source-syntax based. That is the correct foundation for:

- structural test discovery
- import and call-site extraction
- file and line attribution
- static linkage context

It is not sufficient as the only source of truth for complete Rust test detection.

Observed evidence from the Ruff fixture:

- `cargo test -- --list` surfaced quickcheck-generated tests that do not exist as plain source functions.
- `cargo test --doc -- --list` surfaced rustdoc doctests.
- Normal host-side `cargo test -- --list` did not enumerate Ruff's `#[wasm_bindgen_test]` cases.
- Target-specific wasm enumeration may require additional toolchain installation, so it cannot be a hard prerequisite for normal ingestion.

Conclusion:

- Source-only discovery is too weak to justify a completeness claim.
- Build-first discovery is too dependent on toolchain availability and framework-specific runners.
- The credible end-state is hybrid.

## Recommended Architecture

### 1. Source Discovery Layer

Keep Tree-sitter as the primary parser for:

- locating candidate test sources
- discovering suites and scenarios
- deriving file and line metadata
- extracting imports and call sites
- assigning static linkage context

This remains the authoritative source for location and linkage when source information is available.

### 2. Framework Expansion Layer

Add explicit source-level adapters for patterns that are still missing or underpowered:

- `#[rstest]`, `#[case(...)]`, `#[values(...)]`, `#[files(...)]`, `#[template]`, `#[apply(...)]`
- `proptest!`
- rustdoc fenced Rust code blocks on public items

This layer should prefer concrete scenario expansion when the source makes it statically visible. When full expansion cannot be recovered, it should still materialize a source-discovered fallback scenario instead of silently dropping coverage.

### 3. Build-Assisted Enumeration Layer

Augment source discovery with optional enumeration when a Cargo workspace is present:

- `cargo test -- --list` for libtest-backed tests
- `cargo test --doc -- --list` for rustdoc doctests

This layer is for completeness and reconciliation, not for replacing Tree-sitter.

`#[wasm_bindgen_test]` should remain source-authoritative unless a target-specific enumeration path is available. Wasm target installation must not become a hard dependency for normal `ingest-tests`.

### 4. Reconciliation Layer

Merge source-discovered and enumerated scenarios into one canonical artefact model.

Rules:

- prefer source-discovered location and linkage context when both sources match
- materialize enumerated-only scenarios under synthetic suites so they remain queryable
- keep unmatched enumerated results explicit rather than dropping them
- report whether enumeration ran fully, partially, or not at all

## Data and Interface Direction

The public CLI does not need new flags in this rollout.

`ingest-tests` should remain the main entrypoint, but its output should report:

- source-only mode
- hybrid mode with full enumeration
- hybrid mode with partial enumeration / degraded fallback

Internally, the test artefact model should grow enough metadata to support reconciliation and debugging:

- `test_framework`
- `declaration_style`
- `discovery_source`
- optional runner identity when enumeration came from a specific harness

If these fields are not persisted immediately, they should at least exist in the in-memory ingestion model so the reconciliation logic is not built on ad-hoc string heuristics alone.

## Rollout Plan

### Phase 1: PRD and baseline gap inventory

- Author this document.
- Keep `docs/testlens-prototype-prd.md` as historical context instead of mutating it into a Rust-completeness PRD.
- Keep `docs/roadmap.txt` and `docs/validation/traceability_matrix.md` aligned with the real status.

### Phase 2: Source-level declaration completeness

Status: landed in `CLI-1383`, `CLI-1384`, and `CLI-1385`.

- Add `rstest` discovery and scenario expansion.
- Add `proptest!` discovery and scenario extraction.
- Add source-discovered doctest scenarios on public items.

Each slice must ship with:

- inline unit coverage for parser logic
- BDD coverage under `features/` plus `tests/e2e/`
- a SQLite-backed full CLI acceptance path

### Phase 3: Hybrid enumeration

Status: landed in `CLI-1386`.

- Add `cargo test -- --list` enumeration when Cargo metadata and toolchain are available.
- Add `cargo test --doc -- --list` for doctests.
- Reconcile enumerated tests with source-discovered tests.
- Preserve graceful fallback when enumeration is unavailable or incomplete.

### Phase 4: Ruff acceptance gate

Status: landed in `CLI-1387`, with explicit residual helper-attribution notes.

Use a fresh Ruff fixture database as the required real-workspace gate.

Success criteria:

- nonzero suites, scenarios, and links on a fresh run
- benchmark-relevant pyflakes artefacts no longer all appear `untested` when a matching harness exists
- any residual gaps are documented explicitly in validation docs and Jira

### Phase 5: Broader confidence

- Extend real-workspace validation beyond Ruff before claiming general Rust completeness.
- Keep the unsupported list explicit until those surfaces are actually proven.

## Acceptance Criteria

This rollout is complete only when all of the following are true:

- `cargo test` passes
- each new declaration slice has unit, BDD, and SQLite-backed acceptance coverage
- a fresh Ruff run is re-executed after the new logic lands
- the traceability matrix marks each Rust pattern as either:
  - real-workspace validated
  - synthetic-only validated
  - not yet supported

## Non-Goals

This PRD does not reopen:

- cross-framework run outcomes
- scoring parity and run-health semantics
- benches or fuzzing as first-class test artefacts
- final threshold tuning for query scoring

## Related Jira

- `CLI-1368` inline `src` tests and `#[test_case(...)]` rule-harness linkage
- `CLI-1369` Ruff-present declaration styles: `#[wasm_bindgen_test]` and macro-generated quickcheck
- `CLI-1381` umbrella task for Rust test artefact detection completeness
- `CLI-1382` through `CLI-1388` execution subtasks for the completeness rollout
