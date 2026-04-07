Feature: Bitloops unified agent smoke suite
  As a Bitloops maintainer
  I want deterministic smoke coverage for the supported agent integrations
  So that `cargo qat-smoke` and `cargo qat` validate the Bitloops golden path in CI

  @smoke
  Scenario Outline: First agent-driven Bitloops session is captured
    Given I run CleanStart for flow "<first_flow>"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent <agent> --sync=false in bitloops
    And I run bitloops enable in bitloops
    And I make a first change using <agent> to bitloops
    And I committed today in bitloops
    Then bitloops stores exist in bitloops
    And <agent> session exists in bitloops
    And checkpoint mapping exists in bitloops

    Examples:
      | agent       | first_flow               |
      | claude-code | SmokeClaudeFirstSession  |
      | cursor      | SmokeCursorFirstSession  |
      | gemini      | SmokeGeminiFirstSession  |
      | copilot     | SmokeCopilotFirstSession |
      | codex       | SmokeCodexFirstSession   |
      | opencode    | SmokeOpenCodeFirstSession |

  @smoke
  Scenario Outline: Follow-up agent edits create progression
    Given I run CleanStart for flow "<followup_flow>"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent <agent> --sync=false in bitloops
    And I run bitloops enable in bitloops
    And I make a first change using <agent> to bitloops
    And I committed today in bitloops
    And I make a second change using <agent> to bitloops
    And I committed today in bitloops
    Then bitloops stores exist in bitloops
    And <agent> session exists in bitloops
    And checkpoint mapping count is at least 2 in bitloops

    Examples:
      | agent       | followup_flow               |
      | claude-code | SmokeClaudeProgression      |
      | cursor      | SmokeCursorProgression      |
      | gemini      | SmokeGeminiProgression      |
      | copilot     | SmokeCopilotProgression     |
      | codex       | SmokeCodexProgression       |
      | opencode    | SmokeOpenCodeProgression    |

  @smoke
  Scenario: Preserve relative-day commit timeline
    Given I run CleanStart for flow "SmokeCommitTimeline"
    And I start the daemon in bitloops
    And I ran InitCommit yesterday for bitloops
    And I create a Vite app project in bitloops
    And I committed yesterday in bitloops
    And I committed today in bitloops
    Then commit timeline and contents are correct in bitloops
