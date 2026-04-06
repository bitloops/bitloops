Feature: DevQL sync workspace reconciliation
  The sync command scans the current workspace state, compares it against
  stored state, and materializes current-state artefact tables.

  @devql @sync
  Scenario: Full sync indexes workspace source files into queryable artefacts
    Given I run CleanStart for flow "SyncFullIndex"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows added greater than 0 in bitloops
    And DevQL sync summary shows 0 parse errors in bitloops
    And DevQL artefacts query returns results in bitloops

  @devql @sync
  Scenario: Sync detects and indexes newly added source files
    Given I run CleanStart for flow "SyncNewFiles"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows added greater than 0 in bitloops

  @devql @sync
  Scenario: Sync detects and re-indexes modified source files
    Given I run CleanStart for flow "SyncModifiedFiles"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I modify an existing source file in bitloops
    And I commit changes without hooks in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows changed greater than 0 in bitloops

  @devql @sync
  Scenario: Sync removes artefacts for deleted source files
    Given I run CleanStart for flow "SyncDeletedFiles"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I add a new source file in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I delete a source file in bitloops
    And I commit changes without hooks in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows removed greater than 0 in bitloops

  @devql @sync
  Scenario: No-op sync reports zero changes
    Given I run CleanStart for flow "SyncNoop"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows 0 added in bitloops
    And DevQL sync summary shows 0 changed in bitloops
    And DevQL sync summary shows 0 removed in bitloops
    And DevQL sync summary shows unchanged greater than 0 in bitloops

  @devql @sync
  Scenario: Sync after branch checkout reflects the new branch state
    Given I run CleanStart for flow "SyncBranchCheckout"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I create a new branch with additional source files in bitloops
    And I commit changes without hooks in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows added greater than 0 in bitloops

  @devql @sync
  Scenario: Sync catches up after daemon downtime with accumulated changes
    Given I run CleanStart for flow "SyncDaemonDowntime"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I stop the daemon in bitloops
    And I modify an existing source file in bitloops
    And I commit changes without hooks in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I start the daemon in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows added greater than 0 in bitloops
    And DevQL sync summary shows changed greater than 0 in bitloops

  @devql @sync
  Scenario: Sync indexes changes introduced by git pull
    Given I run CleanStart for flow "SyncGitPull"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I simulate a git pull with new changes in bitloops
    And I run DevQL sync --status in bitloops
    Then DevQL sync summary shows added greater than 0 in bitloops

  @devql @sync
  Scenario: Sync validate detects drift on a workspace that has not been synced
    Given I run CleanStart for flow "SyncValidateDrift"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync validate --status in bitloops
    Then DevQL sync validation reports drift in bitloops
    And DevQL sync validation shows expected greater than 0 in bitloops

  @devql @sync
  Scenario: Sync validate reports clean after a full sync
    Given I run CleanStart for flow "SyncValidateClean"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I run DevQL sync validate --status in bitloops
    Then DevQL sync validation reports clean in bitloops

  @devql @sync
  Scenario: Sync validate detects drift when workspace has changed without re-sync
    Given I run CleanStart for flow "SyncValidateDriftAfterChange"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I run DevQL sync validate --status in bitloops
    Then DevQL sync validation reports drift in bitloops

  @devql @sync
  Scenario: Sync repair restores clean state after drift
    Given I run CleanStart for flow "SyncRepair"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL sync --status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I run DevQL sync repair --status in bitloops
    And I run DevQL sync validate --status in bitloops
    Then DevQL sync validation reports clean in bitloops
