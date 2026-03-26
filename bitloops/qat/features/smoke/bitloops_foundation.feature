Feature: Bitloops QAT foundation
  As a Bitloops maintainer
  I want the Bitloops-only BDD harness implemented in Rust
  So that foundation scenarios can run from `bitloops qat`

  @smoke
  Scenario: Bootstrap a fresh repo and create Bitloops stores
    Given I run CleanStart for flow "FoundationStores"
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    Then bitloops stores exist in bitloops

  @smoke
  Scenario: Preserve relative-day commit timeline
    Given I run CleanStart for flow "FoundationCommitTimeline"
    And I ran InitCommit yesterday for bitloops
    And I create a Vite app project in bitloops
    And I committed yesterday in bitloops
    And I committed today in bitloops
    Then commit timeline and contents are correct in bitloops
