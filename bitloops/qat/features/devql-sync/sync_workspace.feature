Feature: DevQL sync workspace reconciliation
  The sync command scans the current workspace state, compares it against
  stored state, and materializes current-state artefact tables.

  @devql @sync
  Scenario: Sync validate detects drift on a workspace that has not been synced
    Given I run CleanStart for flow "SyncValidateDrift"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync validate in bitloops
    Then DevQL sync validation reports drift in bitloops
    And DevQL sync validation shows expected greater than 0 in bitloops

  @devql @sync
  Scenario: Sync validate reports clean after a full sync
    Given I run CleanStart for flow "SyncValidateClean"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync in bitloops
    And I run DevQL sync validate in bitloops
    Then DevQL sync validation reports clean in bitloops
