True Coverage Redesign - Implementation Plan

Context

TestLens currently fans merged LCOV coverage onto every statically linked test (ingest_coverage.rs:58), then the query layer treats those
fanned-out rows as pair-level evidence at 0.95 confidence (query_test_harness.rs:186-196). This misleads agents into believing a specific test
verified a specific artefact when only workspace-level coverage was ingested. The redesign ensures only isolated per-test captures produce
test-scoped evidence.

Alignment with Design Specification

The design spec defines a two-stream ingestion model:

- Stream 1 (static): Tree-sitter parsing → test_links (always available, approximate)
- Stream 2 (coverage): CI/LCOV ingestion → execution-verified linkage (precise, after test run)

This redesign fixes Stream 2 so it doesn't contaminate Stream 1's evidence. The confidence/strength model from the spec is preserved:

- confidence = linkage quality (coverage-verified vs static-only) — now only isolated captures give high confidence
- strength = verification value (fan_out + classification weight) — unchanged

Classification: Stays coverage-derived via rebuild_classifications_from_coverage(). The function is updated to query the new coverage_hits +
coverage_captures tables instead of the old test_coverage. Once true per-test captures flow in, classifications will naturally become accurate
because the underlying coverage data is correct. No classification logic changes needed.

Stage 1: Schema Reset + Domain Types

Replace test_coverage table with coverage_captures + coverage_hits. Add domain enums and structs.

Schema:

- coverage_captures: capture_id PK, repo_id, commit_sha, tool, format, scope_kind (workspace|package|test_scenario|doctest),
  subject_test_artefact_id nullable FK, line_truth, branch_truth, captured_at, status, metadata_json
- coverage_hits: composite PK (capture_id, artefact_id, line, branch_id), file_path, covered, hit_count

Domain types to add:

- ScopeKind enum: Workspace, Package, TestScenario, Doctest
- CoverageFormat enum: Lcov, LlvmJson
- CoverageCaptureRecord struct
- CoverageHitRecord struct

Domain types to remove:

- TestCoverageRecord
- CoverageTarget

Files:

- src/db/schema.rs - replace test_coverage DDL
- src/domain/mod.rs - add/remove types

Verify: cargo check (compile errors expected in ingest_coverage.rs and sqlite.rs - fixed in subsequent stages)

---

Stage 2: Rewrite Coverage Ingest (Single Capture, LCOV)

Rewrite ingest-coverage to create one coverage_captures row + N coverage_hits rows. No fan-out through test_links.

CLI changes:

- Add --scope <workspace|package|test-scenario|doctest> (required)
- Add --tool <string> (optional, default "unknown")
- Add --test-artefact-id <id> (required when scope=test-scenario)
- Reject LCOV when scope=test-scenario (LCOV too lossy for per-test)

Ingest logic:

1.  Create a coverage_captures row with scope metadata
2.  For each LCOV SF section, resolve artefact_ids by matching artefacts WHERE path = lcov_path AND line BETWEEN start_line AND end_line AND
    canonical_kind NOT IN ('test_suite','test_scenario','file')
3.  Insert coverage_hits rows with the capture_id - no join through test_links

Repository changes:

- Replace replace_test_coverage with insert_coverage_capture + insert_coverage_hits
- Remove load_test_links_by_production_artefact and load_coverage_targets_for_file from write trait (only used for fan-out)
- Add load_artefacts_for_file_lines(commit_sha, file_path) -> Vec<(artefact_id, start_line, end_line)> for resolution

Files:

- src/cli.rs - add args to IngestCoverage
- src/app.rs - pass new args
- src/app/commands/ingest_coverage.rs - rewrite handle(), keep parse_lcov_report()
- src/repository/mod.rs - update trait
- src/repository/sqlite.rs - new write methods

Classification adaptation:

- rebuild_classifications_from_coverage() stays but is updated to query coverage_hits JOIN coverage_captures instead of test_coverage
- Only test_scenario captures (which have subject_test_artefact_id) contribute to per-test classification — the function joins on
  scope_kind='test_scenario' to get (test_id, artefact_id, path) tuples, then computes fan_out/boundary_crossings per test as before
- Workspace/package captures have no test attribution, so they produce no per-test classifications (this is correct — the whole point is we
  can't attribute workspace coverage to individual tests)
- The call remains in ingest_coverage.rs after inserting hits
- For workspace-only ingestion, the function simply produces zero classifications (tests remain unclassified until isolated captures arrive)

Verify: Unit test: ingest LCOV with scope=workspace, verify capture row + hit rows, verify no test_artefact_id on capture. Verify
classifications still derived.

---

Stage 3: Query Layer Rewrite

Update confidence scoring so only isolated test_scenario captures produce high confidence.

Confidence changes:

- 0.95 ONLY when coverage_captures with scope_kind='test_scenario' AND subject_test_artefact_id = this_test has hits on the queried artefact
- Workspace/package captures contribute to artefact-level coverage stats but NOT to per-test confidence
- Static-only tests stay at 0.6

New output fields:

- evidence on CoveringTestOutput: "static_only" | "isolated_line_hit"
- coverage_mode on SummaryOutput: "none" | "artefact_only" | "per_test_line" | "per_test_branch"

Branch output:

- Remove covering_test_ids from BranchOutput (empty until true branch attribution)

Repository query changes:

- load_coverage_pair_stats → join coverage_hits + coverage_captures WHERE scope_kind='test_scenario' AND subject_test_artefact_id = ?
- coverage_exists_for_commit → check coverage_captures table
- load_coverage_summary → aggregate from coverage_hits across ALL captures (artefact-level)

Files:

- src/read/query_test_harness.rs - confidence logic, new fields, branch changes
- src/repository/mod.rs - update query trait signatures
- src/repository/sqlite.rs - rewrite SQL queries

Verify: Unit tests: workspace-only coverage gives no test 0.95; isolated capture gives 0.95 + isolated_line_hit

---

Stage 4: LLVM JSON Parser

Add parser for LLVM JSON coverage export format (the canonical format for Rust per-test captures).

LLVM JSON structure:

- data[].files[].segments[] where each segment = [line, col, count, has_count, is_region_entry, is_gap_region]
- Extract per-file, per-line hit counts from segments

Integration:

- Add --format <lcov|llvm-json> to IngestCoverage CLI (auto-detect by extension if omitted)
- LLVM JSON is the ONLY allowed format for scope=test-scenario
- Dispatch to LLVM JSON parser when format matches
- Output feeds into same insert_coverage_capture + insert_coverage_hits path

Files:

- New: src/app/commands/parse_llvm_json.rs
- src/app/commands/mod.rs - add module
- src/app/commands/ingest_coverage.rs - dispatch based on format
- src/cli.rs - add --format arg

Verify: Unit test: parse known LLVM JSON fragment, verify line hits. Integration test: ingest with scope=test-scenario + llvm-json format

---

Stage 5: Batch Manifest Ingestion

Add ingest-coverage-batch --manifest <path> for bulk per-test coverage ingestion.

Manifest format (JSON):
[
{
"format": "llvm-json",
"path": "coverage/test_foo.json",
"scope": "test_scenario",
"test_artefact_id": "test_foo_id",
"tool": "cargo-llvm-cov"
}
]

Files:

- src/cli.rs - add IngestCoverageBatch command
- src/app.rs - add match arm
- New: src/app/commands/ingest_coverage_batch.rs
- src/app/commands/mod.rs - add module
- src/domain/mod.rs - add BatchManifestEntry struct

Verify: Integration test: 2-entry manifest, verify 2 capture rows + corresponding hits

---

Stage 6: E2E Test Updates

Update existing BDD tests and add new scenarios for the true-coverage contract.

Updates to existing tests:

- Rust quickstart: add --scope workspace to ingest-coverage calls
- All e2e tests that check covering_test_ids in branch output: expect empty

New scenarios:

- "workspace coverage does not produce per-test evidence" - ingest workspace LCOV, verify all tests show evidence: "static_only" and confidence
  < 0.95
- "isolated test capture produces per-test evidence" - ingest LLVM JSON with scope=test_scenario, verify that test shows evidence:
  "isolated_line_hit" and confidence = 0.95
- "batch manifest ingestion" - multi-entry manifest, verify captures and query results

Files:

- features/ - new/updated .feature files
- tests/e2e/ - new/updated gherkin test files
- tests/e2e/support/rust_journey.rs - add LLVM JSON fixture generation helper

Verify: cargo test all green

---

Stage 7: Cleanup and Indices

- Add index on coverage_hits(artefact_id, capture_id) for query perf
- Add index on coverage_captures(commit_sha, scope_kind) for filtering
- Update clear_existing_production_data and clear_existing_test_discovery_data in sqlite.rs to reference new tables
- Remove any remaining dead code referencing test_coverage

Files:

- src/db/schema.rs - add indices
- src/repository/sqlite.rs - update cleanup functions

Verify: Full cargo test green. Manual smoke with Ruff fixture.

---

Critical Files Summary

┌────────────────────────────────────────────────┬─────────┐
│ File │ Stages │
├────────────────────────────────────────────────┼─────────┤
│ src/db/schema.rs │ 1, 7 │
├────────────────────────────────────────────────┼─────────┤
│ src/domain/mod.rs │ 1, 5 │
├────────────────────────────────────────────────┼─────────┤
│ src/repository/mod.rs │ 2, 3 │
├────────────────────────────────────────────────┼─────────┤
│ src/repository/sqlite.rs │ 2, 3, 7 │
├────────────────────────────────────────────────┼─────────┤
│ src/app/commands/ingest_coverage.rs │ 2, 4 │
├────────────────────────────────────────────────┼─────────┤
│ src/read/query_test_harness.rs │ 3 │
├────────────────────────────────────────────────┼─────────┤
│ src/cli.rs │ 2, 4, 5 │
├────────────────────────────────────────────────┼─────────┤
│ src/app.rs │ 2, 5 │
├────────────────────────────────────────────────┼─────────┤
│ New: src/app/commands/parse_llvm_json.rs │ 4 │
├────────────────────────────────────────────────┼─────────┤
│ New: src/app/commands/ingest_coverage_batch.rs │ 5 │
└────────────────────────────────────────────────┴─────────┘

Verification End-to-End

After all stages:

1.  cargo test - all unit + e2e tests pass
2.  Ruff F523 quickstart: ingest-coverage --scope workspace produces artefact-level coverage; tests show evidence: "static_only", confidence =
    0.6
3.  Ruff ERA001 quickstart: same behavior for workspace coverage
4.  Simulated per-test capture: one test gets evidence: "isolated_line_hit", confidence = 0.95; others remain static_only
    ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
