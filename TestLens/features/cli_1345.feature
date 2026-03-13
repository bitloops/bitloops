Feature: CLI-1345 Stream 1 discovery of test artefacts
  As a developer
  I want ingest-tests behavior validated through executable BDD scenarios
  So commit-addressable test artefact discovery stays correct

  Scenario: P1-1 Parse and persist test artefacts at a specific commit
    Given a fixture repository containing TypeScript and Rust production files and test files
    And a commit C1 where new test files are added
    When the user runs testlens ingest-tests for C1
    Then test_suite and test_scenario artefacts are created for C1
    And both Rust and TypeScript test artefacts are discovered for C1
    And each C1 test artefact includes language, file path, and source span metadata
    And querying C1 test scenarios is reproducible

  Scenario: P1-2 Commit-addressable reproducibility across commits
    Given commits C0 and C1 where C1 introduces additional tests
    When test artefacts are queried at C0 and C1
    Then test artefacts introduced in C1 are absent at C0
    And commit-addressed query results are reproducible for both C0 and C1
