Feature: Cross-capability change-planning smoke

  This is the integrated acceptance test for the core product promise:
  Bitloops helps developers and agents make safer changes by combining
  blast radius, test harness, semantic clones, and knowledge in a
  coherent workflow.

  @devql @claude-code @testlens @integration
  Scenario: Full change-planning workflow with Claude Code
    # Setup: create a rich project with tests, coverage, and knowledge
    Given I run CleanStart for flow "CrossCapabilitySmoke"
    And I start the daemon in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I ensure Claude Code auth in bitloops
    And I run DevQL init in bitloops
    And I run DevQL ingest in bitloops
    And I run TestLens ingest-tests for latest commit in bitloops
    And I run TestLens ingest-coverage for latest commit in bitloops

    # Step 1: Inspect impact before making a change
    Then DevQL artefacts query returns results in bitloops
    And DevQL deps query for "UserService.createUser" with direction "in" returns at least 1 result in bitloops

    # Step 2: Inspect verification state
    And TestLens query for "createUser" at latest commit with view "summary" returns results in bitloops

    # Step 3: Make the change with Claude Code and commit
    Given I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I run DevQL ingest in bitloops

    # Step 4: Verify all surfaces return coherent answers post-change
    Then bitloops stores exist in bitloops
    And checkpoint mapping exists in bitloops
    And claude-code session exists in bitloops
    And DevQL artefacts query returns results in bitloops
    And DevQL checkpoints query returns results for "claude-code" in bitloops

  @devql @semantic-clones @integration
  Scenario: Capability surfaces are independently queryable in same repo
    Given I run CleanStart for flow "CapabilitySurfaceCoherence"
    And I start the daemon in bitloops
    And I create a TypeScript project with similar implementations in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL ingest in bitloops
    And I run DevQL semantic clones rebuild in bitloops

    # All capability surfaces should work without interfering
    Then DevQL artefacts query returns results in bitloops
    And DevQL clones query for "OrderService.create" returns at least 1 result in bitloops
    And DevQL knowledge query returns 0 items in bitloops
