Feature: CLI-1368 Rust inline and parameterized test linkage
  As a developer
  I want Rust inline src tests and #[test_case(...)] harnesses to be queryable
  So Ruff-style rule tests can show up against the rule function they exercise

  Scenario: R1-1 Inline src #[test_case] scenarios are materialized
    Given a Rust fixture repository with inline parameterized tests at commit C1
    When static linkage is ingested for C1
    Then case-specific Rust test scenarios are materialized

  Scenario: R1-2 Ruff-style rule harness cases link to the matching rule function
    Given a Rust fixture repository with inline parameterized tests at commit C1
    When static linkage is ingested for C1
    Then querying string_dot_format_extra_positional_arguments returns the F523 harness case
    And querying string_dot_format_extra_named_arguments returns the F522 harness case
