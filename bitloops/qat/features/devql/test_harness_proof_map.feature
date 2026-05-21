Feature: TestHarness proof-map for pre-change safety assessment

  The TestHarness must help developers and agents assess whether a code
  area is well-tested before making changes. It must surface covering tests,
  classification, strength, and coverage gaps.

  Background:
    Given I run CleanStart for flow "TestHarnessProofMap"
    And I start the daemon in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I run TestHarness ingest-tests for latest commit in bitloops
    Then daemon capability-event status shows TestHarness sync handler completed in bitloops

  @devql @testharness
  Scenario: Test summary returns counts for `UserService.createUser`
    Then TestHarness query for "UserService.createUser" at latest commit with view "summary" returns results in bitloops
    And TestHarness summary shows non-zero test count in bitloops

  @devql @testharness
  Scenario: Tests query returns individual covering tests for `UserService.createUser`
    Then TestHarness query for "UserService.createUser" at latest commit with view "tests" returns results in bitloops
    And TestHarness tests include at least 1 test with a classification in bitloops

  @devql @testharness @testharness-coverage
  Scenario: Current workspace coverage query uses current artefacts for `UserService.createUser`
    Given I run TestHarness ingest-coverage for current workspace in bitloops
    Then TestHarness query for "UserService.createUser" at current workspace state with view "coverage" returns results in bitloops
    And TestHarness coverage shows line coverage percentage in bitloops

  @devql @testharness @testharness-coverage
  Scenario: Historical coverage query uses historical artefacts for `UserService.createUser`
    Given I run TestHarness ingest-coverage for latest commit in bitloops
    Then TestHarness query for "UserService.createUser" at latest commit with view "coverage" returns results in bitloops
    And TestHarness coverage shows line coverage percentage in bitloops

  @devql @testharness
  Scenario: Untested artefact is clearly identified for `UntestableSingleton.getInstance`
    Then TestHarness query for "UntestableSingleton.getInstance" at latest commit with view "summary" returns empty or zero-count in bitloops

  @devql @testharness
  Scenario: Failing test is distinguishable from passing test
    Given I run TestHarness ingest-results with a failing test for latest commit in bitloops
    Then TestHarness query for "UserService.createUser" at latest commit with view "tests" includes a failing test in bitloops
