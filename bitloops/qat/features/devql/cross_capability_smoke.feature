Feature: Cross-capability deterministic smoke

  This is the integrated offline smoke for the hardened DevQL surfaces:
  agent queryability, blast radius, TestHarness, and semantic clones
  should all remain queryable in the same repository after a real
  agent-driven change.

  Background:
    Given I run CleanStart for flow "CrossCapabilitySmoke"
    And I start the daemon in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I add semantic clone fixtures in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I configure semantic clones with fake embeddings runtime in bitloops
    And I run DevQL init in bitloops
    And DevQL pack health for semantic clones is ready in bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I run DevQL semantic clones rebuild in bitloops
    And I run TestHarness ingest-tests for latest commit in bitloops
    And I run TestHarness ingest-coverage for latest commit in bitloops

  @devql @integration
  Scenario: Hardened DevQL capability surfaces compose in one offline workflow
    Then checkpoint mapping exists in bitloops
    And claude-code session exists in bitloops
    And DevQL checkpoints query returns results for "claude-code" in bitloops
    And DevQL artefacts query returns results in bitloops
    And DevQL deps query for "UserService.createUser" with direction "in" and asOf latest commit returns at least 1 result in bitloops
    And TestHarness query for "UserService.createUser" at current workspace state with view "summary" returns results in bitloops
    And DevQL clones query for "UserService.createUser" returns at least 1 result in bitloops
