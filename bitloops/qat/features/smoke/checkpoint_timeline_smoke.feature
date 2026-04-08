Feature: Bitloops checkpoint timeline smoke suite
  As a Bitloops maintainer
  I want deterministic smoke coverage for relative-day checkpoint history
  So that `cargo qat-smoke` and `cargo qat` validate timeline persistence in CI

  @smoke
  Scenario: Preserve relative-day commit timeline
    Given I run CleanStart for flow "SmokeCommitTimeline"
    And I start the daemon in bitloops
    And I ran InitCommit yesterday for bitloops
    And I create a Vite app project in bitloops
    And I committed yesterday in bitloops
    And I committed today in bitloops
    Then commit timeline and contents are correct in bitloops
