Feature: Workspace-aware blast radius and temporal correctness
  Bitloops must provide correct outgoing dependency graphs for both the
  current workspace state and committed historical state. The dependency
  graph must reflect real code relationships, and historical queries must
  remain stable after new commits.

  Background:
    Given I run CleanStart for flow "BlastRadiusTemporal"
    And I start the daemon in bitloops
    And I create a TypeScript project with known dependencies in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops

  @devql @deps
  Scenario: Dependency query returns outgoing edges for a known caller
    Then DevQL deps query for "UserService.createUser" with direction "out" and asOf latest commit returns at least 1 result in bitloops

  @devql @deps
  Scenario: Dependency query returns incoming edges for a known callee
    Then DevQL deps query for "UserService.createUser" with direction "in" and asOf latest commit returns at least 1 result in bitloops

  @devql @deps
  Scenario: Current workspace edit changes the outgoing dependency graph before commit
    Given I add a new caller of "UserService.createUser" in bitloops
    And I committed today in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then DevQL deps query for "callCreateUser" with direction "out" and asOf latest commit returns at least 1 result in bitloops

  @devql @deps
  Scenario: Historical query returns the pre-edit outgoing graph after ingest
    Given I add a new caller of "UserService.createUser" in bitloops
    And I committed today in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then DevQL deps query for "callCreateUser" with direction "out" and asOf previous commit returns exactly 0 results in bitloops
    And DevQL deps query for "callCreateUser" with direction "out" and asOf latest commit returns at least 1 result in bitloops

  @devql @deps
  Scenario: Repeated ingest does not duplicate artefacts or edges
    Given I enqueue DevQL ingest task with status in bitloops
    Then DevQL artefacts query result count is stable across ingests in bitloops
