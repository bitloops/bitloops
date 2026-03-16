# Test Harness Traceability Matrix

Last updated: 2026-03-16

This matrix tracks the current automated verification coverage in the prototype.
It is intentionally scoped to what is implemented in this repository today.
`cargo test` also emits the current run snapshot to `target/validation/current_status.md`.

## Status

| Jira | Scope | Executable spec / test | Rust-first pass | TypeScript parity pass | Notes |
| --- | --- | --- | --- | --- | --- |
| `CLI-1345` | Test artefact discovery | `features/cli_1345.feature` / `tests/e2e/cli_1345_gherkin.rs` | Yes | Yes | Synthetic mixed-language fixture |
| `CLI-1346` | Static linkage | `features/cli_1346.feature` / `tests/e2e/cli_1346_gherkin.rs` | Yes | Yes | Synthetic mixed-language fixture |
| `CLI-1347` | Query levels + noise control + query errors | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs` | Partial | Yes | TypeScript fixture covers summary percentages, untested artefacts, and `Repository not indexed` vs `Artefact not found`; Rust quickstart covers the static query path |
| `CLI-1348` | LCOV ingestion | `features/cli_1348.feature` / `tests/e2e/cli_1348_gherkin.rs` | Yes | Yes | Rust-first via `cargo llvm-cov`; TypeScript full journey also ingests LCOV |
| `CLI-1349` | Coverage query behavior | `features/cli_1349.feature` / `tests/e2e/cli_1349_gherkin.rs` | Partial | Yes | Rust stable toolchain validates line coverage; TypeScript fixture validates uncovered branch behavior |
| `CLI-1368` | Rust inline `src` tests + `#[test_case(...)]` rule-harness linkage | `features/cli_1368.feature` / `tests/e2e/cli_1368_gherkin.rs` and `features/rust_parameterized_linkage_e2e.feature` / `tests/e2e/rust_parameterized_linkage_e2e_gherkin.rs` | Yes | N/A | Synthetic Ruff-style fixture validates inline `src` discovery, case-level scenarios, and rule-function linkage; fresh Ruff validation now resolves `string_dot_format_extra_positional_arguments` to linked tests |
| `CLI-1369` | Rust `#[wasm_bindgen_test]` + macro-generated quickcheck scenarios | `features/cli_1369.feature` / `tests/e2e/cli_1369_gherkin.rs` and `features/rust_parameterized_linkage_e2e.feature` / `tests/e2e/rust_parameterized_linkage_e2e_gherkin.rs` | Yes | N/A | Synthetic Ruff-style fixture validates wasm-bindgen test discovery and macro-generated quickcheck scenario discovery/linkage |
| `CLI-1381` | Rust `#[rstest]`, `proptest!`, rustdoc doctests, and hybrid enumeration fallback | `features/cli_1381.feature` / `tests/e2e/cli_1381_gherkin.rs` and `features/rust_hybrid_detection_e2e.feature` / `tests/e2e/rust_hybrid_detection_e2e_gherkin.rs` | Yes | N/A | Cargo-backed synthetic fixture validates source expansion plus hybrid enumeration; fresh Ruff run from a clean DB produced `873` suites, `4859` scenarios, and `64760` links with explicit `source-only` fallback after Cargo list timeouts |
| `CLI-1351` | End-to-end acceptance matrix | `features/rust_quickstart_e2e.feature` / `tests/e2e/rust_quickstart_e2e_gherkin.rs` and `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Yes | Yes | Full SQLite-backed CLI journeys |
| `CLI-1352` | Prototype defaults + boundary behavior | `src/domain/mod.rs` inline unit tests and `src/read/query_test_harness.rs` inline unit tests | Yes | Yes | Thresholds, scoring bands, and query defaults are explicit and tested |

## Design-spec scenario coverage

| Design spec scenario | Executable spec / test | Status | Notes |
| --- | --- | --- | --- |
| Scenario 1: Pre-change safety assessment | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs`; `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Covered | Summary view now asserts coverage percentages |
| Scenario 2: Understanding what tests exist before making a change | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs`; `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Covered | Tests view, min-strength default, and explicit override |
| Scenario 3: Finding untested code paths before fixing a bug | `features/cli_1349.feature` / `tests/e2e/cli_1349_gherkin.rs`; `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Covered | Coverage view and uncovered branches are exercised end to end |
| Scenario 4: Identifying pre-existing test failures | `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Deferred | TypeScript-only spot check exists; cross-framework parity is deferred with `CLI-1350` |
| Scenario 6: Assessing an unfamiliar artefact | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs`; `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Covered | Untested `hashPassword` summary path is asserted |
| Scenario 7: PR review adequacy signal | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs` | Covered | Summary exposes verification level, counts, and coverage percentages |
| Scenario 8: Red-phase TDD pattern discovery | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs` | Covered | Tests view exposes suite/test names and classifications |
| Scenario 9: Cross-cutting artefact noise management | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs`; `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Covered | Min-strength filtering is exercised on the real TypeScript fixture |
| Scenario 10: Agent decides whether to write new tests | `features/cli_1347.feature` / `tests/e2e/cli_1347_gherkin.rs`; `features/typescript_full_journey_e2e.feature` / `tests/e2e/typescript_full_journey_e2e_gherkin.rs` | Covered | Both tested and untested verification levels are asserted |

## Full-journey acceptance coverage

| Journey | Real DB | Real fixture repo | Covers |
| --- | --- | --- | --- |
| Rust quickstart | Yes | Copied from `testlens-fixture-rust` | Init, production ingest, test ingest, static query, Rust LCOV generation, coverage ingest, coverage query |
| Rust hybrid detection | Yes | Generated Cargo-backed Rust fixture | Init, production ingest, test ingest, hybrid enumeration reporting, `#[rstest]`, `proptest!`, rustdoc doctests, and query validation against linked production artefacts |
| TypeScript full journey | Yes | Copied from `testlens-fixture` | Init, production ingest, test ingest, Jest coverage run, coverage ingest, results ingest, summary/tests/coverage query views, untested artefact summary, indexed vs missing query errors |

## Known gaps

- `CLI-1350` is intentionally deferred. Cross-framework run-outcome ingestion and scoring are not considered closed yet.
- Ruff hybrid enumeration currently degrades to `source-only` on the full workspace because `cargo test -- --list` and `cargo test --doc -- --list` time out. The CLI reports that degraded mode and still materializes source-discovered tests and links.
- Benchmark-relevant helper functions such as `remove_unused_positional_arguments_from_format_call` and `transform_expression` still query as `untested` on Ruff. This is a residual helper-attribution gap, not a missing test-declaration detection gap.
- The broader feature-spec matrix (`S1-S12`, `E1-E4`, `ERR1-ERR2`) is still only partially reconstructable because the feature-spec page linked from Jira could not be resolved from the current Atlassian page id.
- Rust stable tooling currently gives a reliable LCOV line-coverage flow. Branch-gap assertions are currently validated on the TypeScript fixture where branch data is present.
