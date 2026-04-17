@onboarding @agents-checkpoints
Feature: Agent checkpoint capture flow
  As a developer validating checkpoint persistence,
  I want focused coverage around agent interactions and checkpoint materialization
  so that the dedicated suite proves session and checkpoint behavior deterministically.

  Background:
    Given I run CleanStart for flow "agents-checkpoints"
    And   I start the daemon in bitloops
    And   I create a Rust project with tests in bitloops

  @agents-checkpoints
  Scenario: Supported agent can complete bootstrap and create the first checkpoint
    Given I run InitCommit for bitloops
    And   I run bitloops init --agent claude-code in bitloops
    And   I run bitloops enable in bitloops
    And   I ask claude-code to "Add a subtract function to src/lib.rs that subtracts two i32 numbers and returns the result, and add a test for it" in bitloops
    When  I committed today in bitloops
    Then  checkpoint mapping exists in bitloops
    And   commit_checkpoints count is at least 1 in bitloops

  @agents-checkpoints @agents-checkpoints-precommit
  Scenario: Agent interaction exists before the first checkpoint is committed
    Given I run InitCommit for bitloops
    And   I run bitloops init --agent claude-code in bitloops
    And   I run bitloops enable in bitloops
    And   I ask claude-code to "Add a subtract function to src/lib.rs that subtracts two i32 numbers and add a test for it" in bitloops
    Then  claude-code interaction exists before commit in bitloops
    When  I committed today in bitloops
    Then  checkpoint mapping exists in bitloops
    And   commit_checkpoints count is at least 1 in bitloops

  @agents-checkpoints @agents-checkpoints-progression
  Scenario: Single agent checkpoint progression stays ordered across multiple commits
    Given I run InitCommit for bitloops
    And   I run bitloops init --agent claude-code in bitloops
    And   I run bitloops enable in bitloops
    And   I ask claude-code to "Add a subtract function to src/lib.rs that subtracts two i32 numbers and add a test for it" in bitloops
    When  I committed today in bitloops
    And   I ask claude-code to "Add a divide function to src/lib.rs that divides two i32 numbers and add a test for it" in bitloops
    And   I committed today in bitloops
    Then  checkpoint mapping count is at least 2 in bitloops
    And   commit_checkpoints count is at least 2 in bitloops
    And   captured commit history is ordered in bitloops

  @agents-checkpoints @agents-checkpoints-timeline
  Scenario: Single agent checkpoint timeline stays coherent across yesterday and today
    Given I ran InitCommit yesterday for bitloops
    And   I run bitloops init --agent claude-code in bitloops
    And   I run bitloops enable in bitloops
    And   I ask claude-code to "Add a subtract function to src/lib.rs that subtracts two i32 numbers and add a test for it" in bitloops
    When  I committed yesterday in bitloops
    And   I ask claude-code to "Add a divide function to src/lib.rs that divides two i32 numbers and add a test for it" in bitloops
    And   I committed today in bitloops
    Then  checkpoint mapping count is at least 2 in bitloops
    And   captured commit history is ordered in bitloops
    And   commit timeline and contents are correct in bitloops

  @agents-checkpoints @agents-checkpoints-multi
  Scenario: Multiple agents can interleave checkpoint activity without breaking history order
    Given I run InitCommit for bitloops
    And   I run bitloops init with agents claude-code and cursor in bitloops
    And   I run bitloops enable in bitloops
    Then  git hooks exist for the claude-code agent in bitloops
    And   git hooks exist for the cursor agent in bitloops
    Given I ask claude-code to "Add a subtract function to src/lib.rs that subtracts two i32 numbers and add a test for it" in bitloops
    And   I ask cursor to "Add a modulo function to src/lib.rs that returns the remainder of two i32 numbers and add a test for it" in bitloops
    Then  claude-code interaction exists before commit in bitloops
    And   cursor interaction exists before commit in bitloops
    When  I committed today in bitloops
    And   I ask cursor to "Add a divide function to src/lib.rs that divides two i32 numbers and add a test for it" in bitloops
    And   I committed today in bitloops
    Then  checkpoint mapping count is at least 2 in bitloops
    And   commit_checkpoints count is at least 2 in bitloops
    And   captured commit history is ordered in bitloops
    And   claude-code session exists in bitloops
    And   cursor session exists in bitloops
