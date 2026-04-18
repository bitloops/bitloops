Feature: Bitloops checkpoint timeline Agent Smoke suite
  As a Bitloops maintainer
  I want deterministic Agent Smoke coverage for relative-day checkpoint history
  So that `cargo qat-agent-smoke` and `cargo qat` validate timeline persistence in CI

  @agent_smoke
  Scenario: Preserve relative-day commit timeline
    Given I run CleanStart for flow "SmokeCommitTimeline"
    And I start the daemon in bitloops
    And I ran InitCommit yesterday for bitloops
    And I create a Vite app project in bitloops
    And I committed yesterday in bitloops
    And I committed today in bitloops
    Then git timeline and contents are correct in bitloops
