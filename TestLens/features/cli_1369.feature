Feature: CLI-1369 Ruff-style additional Rust test declarations
  As a developer
  I want Rust wasm-bindgen tests and macro-generated quickcheck tests to be queryable
  So TestLens does not miss common Ruff test scenarios beyond plain #[test] and #[test_case(...)]

  Scenario: R2-1 Additional Ruff-style Rust test scenarios are materialized
    Given a Rust fixture repository with Ruff-style additional test declarations at commit C1
    When static linkage is ingested for C1
    Then Ruff-style Rust test scenarios are materialized

  Scenario: R2-2 Ruff-style additional test declarations link to production artefacts
    Given a Rust fixture repository with Ruff-style additional test declarations at commit C1
    When static linkage is ingested for C1
    Then querying render_message returns the wasm harness case
    And querying is_equivalent_to returns the quickcheck property test
