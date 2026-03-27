Feature: Test Harness proof-map for pre-change safety assessment

  The Test Harness must help developers and agents assess whether a code
  area is well-tested before making changes. It must surface covering tests,
  classification, strength, and coverage gaps.

  Background:
    Given I run CleanStart for flow "TestHarnessProofMap"
    And I create a TypeScript project with tests and coverage in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL ingest in bitloops
    And I run TestLens ingest-tests for latest commit in bitloops
    And I run TestLens ingest-coverage for latest commit in bitloops

  @devql @testlens
  Scenario: Test summary returns counts for a tested artefact
    Then TestLens query for "createUser" at latest commit with view "summary" returns results in bitloops
    And TestLens summary shows non-zero test count in bitloops

  @devql @testlens
  Scenario: Tests query returns individual covering tests
    Then TestLens query for "createUser" at latest commit with view "tests" returns results in bitloops
    And TestLens tests include at least 1 test with a classification in bitloops

  @devql @testlens
  Scenario: Coverage query returns line coverage data
    Then TestLens query for "createUser" at latest commit with view "coverage" returns results in bitloops
    And TestLens coverage shows line coverage percentage in bitloops

  @devql @testlens
  Scenario: Untested artefact is clearly identified
    Then TestLens query for "UntestableSingleton" at latest commit with view "summary" returns empty or zero-count in bitloops

  @devql @testlens
  Scenario: Failing test is distinguishable from passing test
    Given I run TestLens ingest-results with a failing test for latest commit in bitloops
    Then TestLens query for "createUser" at latest commit with view "tests" includes a failing test in bitloops
