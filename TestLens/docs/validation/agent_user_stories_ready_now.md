# Agent-Helpful User Stories We Can Test Right Now

Last updated: 2026-03-16

Source document:
- Confluence page `433717249` - `Test Harness - Design Specification`

This note maps the agent-facing scenarios from the design spec to what the current TestLens prototype can validate in this repository today.

"Ready to test right now" means there is executable coverage in the current local test suite, primarily through:
- `features/cli_1347.feature`
- `features/cli_1349.feature`
- `features/typescript_full_journey_e2e.feature`
- `features/rust_quickstart_e2e.feature`

Current validation references:
- `docs/validation/traceability_matrix.md`
- `target/validation/current_status.md`

## Ready Now

| Design spec scenario | Agent-helpful user story | Why it helps agents | What we can test now | Current evidence |
| --- | --- | --- | --- | --- |
| Scenario 1: Pre-change safety assessment | As an agent planning a refactor, I need a quick verification summary for an artefact so I can decide whether it is safe to proceed. | This supports the first planning step before the agent edits code. | `tests().summary()` returns `verification_level`, counts by classification, and coverage percentages. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs`, `features/typescript_full_journey_e2e.feature`, `tests/e2e/typescript_full_journey_e2e_gherkin.rs` |
| Scenario 2: Understanding what tests exist before making a change | As an agent preparing a change, I need the covering tests, their type, and their relative strength so I know what to update and what to watch. | This directly improves change planning and regression awareness. | `tests()` returns covering tests plus summary, and the default `min_strength` filter can be overridden. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs`, `features/typescript_full_journey_e2e.feature`, `tests/e2e/typescript_full_journey_e2e_gherkin.rs` |
| Scenario 3: Finding untested code paths before fixing a bug | As an agent fixing a bug, I need exact uncovered branches so I can target the missing test and the risky path. | This narrows bug-fix work to the real verification gap instead of broad retesting. | `coverage()` returns branch detail and uncovered branches for the queried artefact. | `features/cli_1349.feature`, `tests/e2e/cli_1349_gherkin.rs`, `features/typescript_full_journey_e2e.feature`, `tests/e2e/typescript_full_journey_e2e_gherkin.rs` |
| Scenario 6: Assessing an unfamiliar artefact | As an agent touching unfamiliar code, I need to know quickly whether the area is effectively untested. | This lets the agent switch to characterization-test mode before making risky changes. | `tests().summary()` is asserted for an untested artefact and returns `verification_level = untested`. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs`, `features/typescript_full_journey_e2e.feature`, `tests/e2e/typescript_full_journey_e2e_gherkin.rs` |
| Scenario 7: PR review adequacy signal | As an agent reviewing a change, I need a compact adequacy signal so I can flag weakly tested edits. | This supports review-time triage without loading the full test landscape first. | Summary output exposes `verification_level`, counts, and coverage percentages. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs` |
| Scenario 8: Red-phase TDD pattern discovery | As an agent writing tests first, I need to see the existing test names, suites, and classifications so I can follow the local testing style. | This improves consistency when the agent adds new tests. | `tests()` exposes test names, suites, and classifications for the target artefact. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs` |
| Scenario 9: Cross-cutting artefact noise management | As an agent changing a cross-cutting artefact, I need the tool to suppress incidental tests by default so I can focus on the strong signals first. | This keeps agent context compact and reduces wasted tokens on irrelevant tests. | The default `min_strength` filter hides weaker tests; override paths are tested. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs`, `features/typescript_full_journey_e2e.feature`, `tests/e2e/typescript_full_journey_e2e_gherkin.rs` |
| Scenario 10: Decide whether to write new tests | As an agent about to implement a change, I need the verification state to determine whether to rely on existing tests, add focused tests, or write characterization tests first. | This is the core planning decision for safe autonomous coding. | The current suite asserts both tested and untested verification paths and supports follow-up drill-down with `coverage()`. | `features/cli_1347.feature`, `tests/e2e/cli_1347_gherkin.rs`, `features/typescript_full_journey_e2e.feature`, `tests/e2e/typescript_full_journey_e2e_gherkin.rs` |

## Partially Testable Today

| Design spec scenario | Current position | Why it is only partial |
| --- | --- | --- |
| Scenario 4: Identifying pre-existing test failures | We can spot-check this on the TypeScript/Jest flow today. | Cross-framework run outcomes and scoring parity are still deferred with `CLI-1350`, so this is not yet a full prototype-wide story. |

## Practical Readout

The strongest agent-improving stories we can exercise right now are the ones that help an agent decide:
- whether an artefact is safe to change
- which existing tests matter
- which branch paths are untested
- whether an area is effectively untested
- how to avoid noisy cross-cutting results
- whether the agent should add tests before changing code

Those stories are already represented in executable coverage and are the best candidates for immediate agent-task evaluation.

## Important Limits

- TypeScript currently has the strongest end-to-end coverage for these agent stories.
- Rust validates the prototype flow and LCOV ingestion, but not every query-level story is covered as deeply as TypeScript.
- Pre-existing failing test stories are not fully closed until run outcomes are implemented beyond the current Jest-only path.
