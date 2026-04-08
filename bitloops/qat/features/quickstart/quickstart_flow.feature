@onboarding @quickstart
Feature: Quickstart end-to-end flow
  As a new user following the quickstart guide,
  I want to go from clone to queryable artefacts with test coverage
  so that I can see complete Bitloops value in one sitting.

  Background:
    Given I run CleanStart for flow "quickstart"
    And   I start the daemon in bitloops
    And   I create a Rust project with tests in bitloops
    And   I run InitCommit for bitloops
    And   I run bitloops init --agent claude-code in bitloops
    And   I run bitloops enable in bitloops

  @quickstart-checkpoint
  Scenario: Checkpoint creation through Claude Code edits
    Given I ensure Claude Code auth in bitloops
    And   I ask Claude Code to "Add a subtract function to src/lib.rs that subtracts two i32 numbers and returns the result, and add a test for it" in bitloops
    And   I committed today in bitloops
    Then  checkpoint mapping exists in bitloops
    And   commit_checkpoints count is at least 1 in bitloops

#  @quickstart-devql
#  Scenario: DevQL ingest materializes production artefacts
#    Given I simulate a claude checkpoint in bitloops
#    And   I run DevQL init in bitloops
#    And   I run DevQL ingest in bitloops
#    Then  DevQL artefacts query returns results in bitloops
#    And   DevQL checkpoints query returns results for "claude" in bitloops
#
#  @quickstart-testlens
#  Scenario: TestHarness ingests tests and links them to artefacts
#    Given I simulate a claude checkpoint in bitloops
#    And   I run DevQL init in bitloops
#    And   I run DevQL ingest in bitloops
#    And   I run TestHarness ingest-tests at HEAD in bitloops
#    Then  TestHarness query for "test_add" at current workspace state with view "tests" returns results in bitloops
#
#  @quickstart-coverage
#  Scenario: Coverage ingestion produces line coverage data
#    Given I simulate a claude checkpoint in bitloops
#    And   I run DevQL init in bitloops
#    And   I run DevQL ingest in bitloops
#    And   I run TestHarness ingest-tests at HEAD in bitloops
#    And   I run TestHarness ingest-coverage at HEAD in bitloops
#    Then  coverage_captures count is at least 1 in bitloops
#    And   coverage_hits count is at least 1 in bitloops
