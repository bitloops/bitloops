Feature: Workspace-aware blast radius and temporal correctness
  Bitloops must provide correct dependency graphs for both the current
  workspace state and committed historical state. The dependency graph
  must reflect real code relationships, and historical queries must
  remain stable after new commits.

  Background:
    Given I run CleanStart for flow "BlastRadiusTemporal"
    And I create a TypeScript project with known dependencies in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL ingest in bitloops

  @devql @claude-code @integration
  Scenario: Dependency query returns correct incoming edges for a known callee
    Then DevQL deps query for "UserService.createUser" with direction "out" returns at least 1 result in bitloops

  @devql @claude-code @integration
  Scenario: Dependency query returns correct outgoing edges for a known caller
    Then DevQL deps query for "UserController.handleCreate" with direction "out" returns at least 1 result in bitloops

  @devql @claude-code @integration
  Scenario: Current workspace edit changes the dependency graph before commit
    Given I add a new caller of "UserService.createUser" in bitloops
    And I committed today in bitloops
    And I run DevQL ingest in bitloops
    Then DevQL deps query for "callCreateUser" with direction "out" returns at least 1 result in bitloops

  @devql @claude-code @integration
  Scenario: Historical query returns the pre-edit graph after ingest
    Given I add a new caller of "UserService.createUser" in bitloops
    And I committed today in bitloops
    And I run DevQL ingest in bitloops
    Then DevQL deps query for "callCreateUser" with direction "out" and asOf previous commit returns exactly 0 results in bitloops
    And DevQL deps query for "callCreateUser" with direction "out" and asOf latest commit returns at least 1 result in bitloops

  @devql @claude-code @integration
  Scenario: Repeated ingest does not duplicate artefacts or edges
    Given I run DevQL ingest in bitloops
    Then DevQL artefacts query result count is stable across ingests in bitloops
