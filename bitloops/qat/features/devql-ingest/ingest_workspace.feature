Feature: DevQL ingest workspace history coverage
  The ingest command replays commit history into relational artefacts tables.
  These scenarios validate ingest behavior using DB-first checks on
  commit_ingest_ledger, artefacts_current, and file_state, including rewritten
  history SHA guarantees and bounded backfill behavior.

  @devql @ingest
  Scenario: Initial backlog ingest completes all reachable history
    Given I run CleanStart for flow "IngestInitialBacklog"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I create 2 ingest commits in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest
  Scenario: Re-ingest at same HEAD is idempotent
    Given I run CleanStart for flow "IngestIdempotentReplay"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I create 2 ingest commits in bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I snapshot ingest DB state in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then DevQL ingest summary shows 0 commits_processed in bitloops
    And completed ledger count is unchanged since snapshot in bitloops
    And artefacts_current count is unchanged since snapshot in bitloops
    And no new SHAs were completed since snapshot in bitloops

  @devql @ingest
  Scenario: Two commits are ingested together in one replay
    Given I run CleanStart for flow "IngestTwoCommitBatch"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I snapshot ingest DB state in bitloops
    And I create 2 ingest commits in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs are completed in commit_ingest_ledger in bitloops
    And expected SHAs have file_state rows in bitloops

  @devql @ingest
  Scenario: Commits made while daemon is down are batched on next ingest
    Given I run CleanStart for flow "IngestDaemonDownBatch"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I snapshot ingest DB state in bitloops
    And I stop the daemon in bitloops
    And I create 2 ingest commits in bitloops
    And I start the daemon in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs are completed in commit_ingest_ledger in bitloops
    And expected SHAs have file_state rows in bitloops

  @devql @ingest
  Scenario: Non-FF merge ingests feature commits and merge commit
    Given I run CleanStart for flow "IngestNonFFMerge"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I snapshot ingest DB state in bitloops
    And I create a non-FF merge with 2 feature commits in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs are completed in commit_ingest_ledger in bitloops
    And expected SHAs have file_state rows in bitloops
    And artefacts_current contains path "src/non_ff_feature_one.rs" in bitloops
    And artefacts_current contains path "src/non_ff_feature_two.rs" in bitloops
    And all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest
  Scenario: FF merge ingests feature commits without creating a merge SHA
    Given I run CleanStart for flow "IngestFFMerge"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I snapshot ingest DB state in bitloops
    And I create an FF merge with 2 feature commits in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs are completed in commit_ingest_ledger in bitloops
    And expected SHAs have file_state rows in bitloops
    And artefacts_current contains path "src/ff_feature_one.rs" in bitloops
    And artefacts_current contains path "src/ff_feature_two.rs" in bitloops
    And all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest
  Scenario: Cherry-pick ingests the cherry-picked SHAs
    Given I run CleanStart for flow "IngestCherryPick"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I snapshot ingest DB state in bitloops
    And I cherry-pick 2 commits in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs are completed in commit_ingest_ledger in bitloops
    And expected SHAs have file_state rows in bitloops
    And artefacts_current contains path "src/cherry_source_one.rs" in bitloops
    And artefacts_current contains path "src/cherry_source_two.rs" in bitloops
    And all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest
  Scenario: Rebase with edit rewrites SHAs and ingests rewritten history
    Given I run CleanStart for flow "IngestRebaseRewrite"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I create 2 ingest commits in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I capture top 2 reachable SHAs before rewrite in bitloops
    And I rewrite last 2 commits with rebase edit in bitloops
    And I snapshot ingest DB state in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then rewrite introduces exactly 2 new reachable SHAs in bitloops
    And old rewritten SHAs are absent from post-rewrite reachable segment in bitloops
    And rewritten new SHAs are completed in commit_ingest_ledger in bitloops
    And exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs have file_state rows in bitloops
    And all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest
  Scenario: Reset rewrite introduces replacement SHAs and ingests them
    Given I run CleanStart for flow "IngestResetRewrite"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude --sync=false in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I create 2 ingest commits in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I capture top 2 reachable SHAs before rewrite in bitloops
    And I reset last 2 commits and create replacement commits in bitloops
    And I snapshot ingest DB state in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then rewrite introduces exactly 2 new reachable SHAs in bitloops
    And old rewritten SHAs are absent from post-rewrite reachable segment in bitloops
    And rewritten new SHAs are completed in commit_ingest_ledger in bitloops
    And exact expected SHAs were newly completed since snapshot in bitloops
    And expected SHAs have file_state rows in bitloops
    And all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest @backfill
  Scenario: Init backfill 1 ingests only latest commit
    Given I run CleanStart for flow "IngestBackfillOneInit"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I create 2 ingest commits in bitloops
    And I run bitloops init --agent claude --sync=false --ingest=true --backfill=1 in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    Then only latest 1 reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest @backfill
  Scenario: Full ingest catches up after init backfill 1
    Given I run CleanStart for flow "IngestBackfillOneCatchup"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I create 2 ingest commits in bitloops
    And I run bitloops init --agent claude --sync=false --ingest=true --backfill=1 in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    Then only latest 1 reachable SHAs are completed in commit_ingest_ledger in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then all reachable SHAs are completed in commit_ingest_ledger in bitloops

  @devql @ingest @backfill
  Scenario: Init backfill 2 is bounded and full ingest catches up
    Given I run CleanStart for flow "IngestBackfillTwoCatchup"
    And I start the daemon in bitloops
    And I create a simple Rust project in bitloops
    And I run InitCommit for bitloops
    And I create 3 ingest commits in bitloops
    And I run bitloops init --agent claude --sync=false --ingest=true --backfill=2 in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    Then only latest 2 reachable SHAs are completed in commit_ingest_ledger in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    Then all reachable SHAs are completed in commit_ingest_ledger in bitloops
