Feature: Bitloops Claude Code workflows
  As a Bitloops maintainer
  I want `bitloops ftf` to exercise the main Claude Code integration path
  So that the default FTF command validates real end-to-end Bitloops behavior

  Scenario: Bootstrap Bitloops and persist a Claude Code session
    Given I run CleanStart for flow "ClaudeCodeSession"
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    Then claude-code session exists in bitloops
    And checkpoint mapping exists in bitloops
    And bitloops stores exist in bitloops

  Scenario: Claude Code follow-up edits create multiple checkpoints
    Given I run CleanStart for flow "ClaudeCodeProgression"
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I make a second change using Claude Code to bitloops
    And I committed today in bitloops
    Then claude-code session exists in bitloops
    And checkpoint mapping count is at least 2 in bitloops
    And bitloops stores exist in bitloops
