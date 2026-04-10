Feature: Agent enablement produces a queryable repository
  After a developer sets up Bitloops with Claude Code, makes a change,
  and commits, the repository should be fully queryable through DevQL.
  This validates the complete top-of-funnel product journey: from clean
  repo to first useful retrieval.

  @devql @claude-code @integration
  Scenario: First Claude Code change is queryable through DevQL
    Given I run CleanStart for flow "AgentEnablementQueryable"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I run DevQL init in bitloops
    And I run DevQL ingest in bitloops
    Then bitloops stores exist in bitloops
    And checkpoint mapping exists in bitloops
    And DevQL artefacts query returns results in bitloops
    And DevQL checkpoints query returns results for "claude-code" in bitloops

  @devql @claude-code @integration
  Scenario: Claude Code chat history is retrievable after edit and commit
    Given I run CleanStart for flow "AgentChatHistory"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I run DevQL init in bitloops
    Then DevQL chatHistory query returns results in bitloops
