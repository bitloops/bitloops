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
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows artefacts indexed for current HEAD in bitloops
    And DevQL sync summary shows 0 parse errors in bitloops
    And DevQL artefacts query returns results in bitloops

  @devql @sync @test_harness_sync @develop_gate
  Scenario: Sync materializes test-harness coverage for discovered tests
    Given I run CleanStart for flow "SyncTestHarnessPopulate"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then daemon capability-event status shows TestHarness sync handler completed in bitloops
    Then TestHarness query for "createUser" at current workspace state with view "tests" returns results in bitloops

  @devql @sync @test_harness_sync
  Scenario: Sync removes test-harness coverage when test files are deleted
    Given I run CleanStart for flow "SyncTestHarnessDeleteTestFile"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then daemon capability-event status shows TestHarness sync handler completed in bitloops
    Then TestHarness query for "createUser" at current workspace state with view "tests" returns results in bitloops
    Given I delete a test file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then daemon capability-event status shows TestHarness sync handler completed in bitloops
    Then TestHarness query for "createUser" at current workspace state with view "tests" returns empty or zero-count in bitloops

  @devql @sync
  Scenario: Sync detects and indexes newly added source files
    Given I run CleanStart for flow "SyncNewFiles"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows added greater than 0 for current HEAD in bitloops

  @devql @sync
  Scenario: Sync detects and re-indexes modified source files
    Given I run CleanStart for flow "SyncModifiedFiles"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I modify an existing source file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows changed greater than 0 for current HEAD in bitloops

  @devql @sync
  Scenario: Sync removes artefacts for deleted source files
    Given I run CleanStart for flow "SyncDeletedFiles"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I add a new source file in bitloops
    And I run InitCommit without post-commit refresh for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I delete a source file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows removed greater than 0 for current HEAD in bitloops

  @devql @sync
  Scenario: No-op sync reports zero changes
    Given I run CleanStart for flow "SyncNoop"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I enqueue DevQL sync task with status in bitloops
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
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I create a new branch with additional source files in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows artefacts indexed for current HEAD in bitloops

  @devql @sync
  Scenario: Sync catches up after daemon downtime with accumulated changes
    Given I run CleanStart for flow "SyncDaemonDowntime"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I stop the daemon in bitloops
    And I modify an existing source file in bitloops
    And I commit changes without hooks in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I start the daemon in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows added greater than 0 for current HEAD in bitloops
    And DevQL sync history shows changed greater than 0 for current HEAD in bitloops

  @devql @sync
  Scenario: Sync indexes changes introduced by git pull
    Given I run CleanStart for flow "SyncGitPull"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I simulate a git pull with new changes in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync history shows artefacts indexed for current HEAD in bitloops

  @devql @sync
  Scenario: Sync validate reports clean after a full sync
    Given I run CleanStart for flow "SyncValidateClean"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I enqueue DevQL sync validate task with status in bitloops
    Then DevQL sync validation reports clean in bitloops

  @devql @sync
  Scenario: Sync repair restores clean state after drift
    Given I run CleanStart for flow "SyncRepair"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync repair task with status in bitloops
    And I enqueue DevQL sync validate task with status in bitloops
    Then DevQL sync validation reports clean in bitloops

  @devql @sync
  Scenario: Sync validate reports drift when workspace changes are not reconciled
    Given I run CleanStart for flow "SyncValidateDrift"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync validate task with status in bitloops
    Then DevQL sync validation reports drift in bitloops
    And DevQL sync validation shows expected greater than 0 in bitloops

  @devql @sync
  Scenario: Sync validate shows non-zero expected counts after multiple unsynced changes
    Given I run CleanStart for flow "SyncValidateAccumulatedDrift"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I modify an existing source file in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync validate task with status in bitloops
    Then DevQL sync validation reports drift in bitloops
    And DevQL sync validation shows expected greater than 1 in bitloops

  @devql @sync
  Scenario: Path-scoped sync only updates the specified paths
    Given I run CleanStart for flow "SyncPathScoped"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I add a source file "src/lib.rs" in bitloops
    And I run InitCommit without post-commit refresh for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I snapshot current-state content ids for "src/main.rs,src/lib.rs" in bitloops
    And I modify a source file "src/main.rs" in bitloops
    And I add a source file "src/ignored.rs" in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with paths "src/main.rs" and status in bitloops
    Then DevQL sync summary shows changed greater than 0 in bitloops
    And current-state content id for "src/main.rs" changed since snapshot in bitloops
    And current-state content id for "src/lib.rs" is unchanged since snapshot in bitloops
    And artefacts_current does not contain path "src/ignored.rs" in bitloops

  @devql @sync
  Scenario: Explicit full sync still materializes current-state artefacts
    Given I run CleanStart for flow "SyncExplicitFull"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL full sync task with status in bitloops
    Then DevQL sync history shows artefacts indexed for current HEAD in bitloops
    And DevQL sync summary shows 0 parse errors in bitloops
    And DevQL artefacts query returns results in bitloops

  @devql @sync
  Scenario: Sync enqueue with require-daemon fails when the daemon is unavailable
    Given I run CleanStart for flow "SyncRequireDaemonFailure"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I stop the daemon in bitloops
    And I attempt to enqueue DevQL sync task with require-daemon in bitloops
    Then the command fails with exit code non-zero in bitloops
    And the command output contains "daemon" in bitloops

  @devql @sync @task_queue_observability
  Scenario: Task queue observability surfaces queued task lifecycle
    Given I run CleanStart for flow "SyncTaskQueueObservability"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I wait for the DevQL task queue to become idle in bitloops
    And I pause the DevQL task queue with reason "qat-observe" in bitloops
    And I enqueue DevQL ingest task without status in bitloops
    Then DevQL task id is captured in bitloops
    And the last DevQL task kind is "ingest" in bitloops
    Given I run DevQL tasks status in bitloops
    Then DevQL task queue state is "paused" in bitloops
    And DevQL task queue pause reason is "qat-observe" in bitloops
    Given I run DevQL tasks list in bitloops
    Then DevQL tasks list includes the last task in bitloops
    Given I resume the DevQL task queue in bitloops
    Given I watch the last DevQL task in bitloops
    Given I run DevQL tasks list for status "completed" in bitloops
    Then DevQL tasks list includes the last task in bitloops
    And the last DevQL task has status "completed" in bitloops

  @devql @sync
  Scenario: Task queue pause and resume update repository queue state
    Given I run CleanStart for flow "SyncTaskQueuePauseResume"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I pause the DevQL task queue with reason "qat-maintenance" in bitloops
    Given I run DevQL tasks status in bitloops
    Then DevQL task queue state is "paused" in bitloops
    And DevQL task queue pause reason is "qat-maintenance" in bitloops
    Given I resume the DevQL task queue in bitloops
    Given I run DevQL tasks status in bitloops
    Then DevQL task queue state is "running" in bitloops

  @devql @sync
  Scenario: Queued task can be cancelled while the repository queue is paused
    Given I run CleanStart for flow "SyncTaskQueueCancel"
    And I start the daemon in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run DevQL init in bitloops
    And I add a new source file in bitloops
    And I commit changes without hooks in bitloops
    And I pause the DevQL task queue with reason "qat-cancel" in bitloops
    And I enqueue DevQL sync task without status in bitloops
    Then DevQL task id is captured in bitloops
    Given I cancel the last DevQL task in bitloops
    Then the last DevQL task has status "cancelled" in bitloops
    Given I run DevQL tasks list for status "cancelled" in bitloops
    Then DevQL tasks list includes the last task in bitloops
    Given I resume the DevQL task queue in bitloops

  @devql @sync
  Scenario: Init with sync=true makes immediate follow-up sync report no changes
    Given I run CleanStart for flow "SyncInitSyncTrueNoop"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=true in bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then DevQL sync summary shows 0 added in bitloops
    And DevQL sync summary shows 0 changed in bitloops
    And DevQL sync summary shows 0 removed in bitloops
    And DevQL sync summary shows unchanged greater than 0 in bitloops

  @devql @sync @sync_init_sync_true_incremental @develop_gate
  Scenario: Watcher-driven materialization after init --sync=true
    Given I run CleanStart for flow "SyncInitSyncTrueIncremental"
    And I enable watcher autostart in bitloops
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=true in bitloops
    And artefacts_current does not contain path "src/math.rs" in bitloops
    And I add a source file "src/math.rs" in bitloops
    Then artefacts_current eventually contains path "src/math.rs" in bitloops

  @devql @sync
  Scenario: Init with sync=true keeps sync validation clean without workspace changes
    Given I run CleanStart for flow "SyncInitSyncTrueValidateClean"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=true in bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync validate task with status in bitloops
    Then DevQL sync validation reports clean in bitloops
