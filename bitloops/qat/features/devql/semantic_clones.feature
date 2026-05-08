Feature: Semantic Clones enrichment and query coverage

  The default semantic-clones QAT lane follows the enrichment QA guide.
  It validates the offline deterministic fake-runtime path, not real local
  model warm-cache behavior.

  Background:
    Given I run CleanStart for flow "SemanticClones"
    And I start the daemon in bitloops
    And I create a TypeScript project with semantic clone quality fixtures in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I configure guide-aligned semantic clones with fake embeddings runtime in bitloops
    And I run DevQL init in bitloops
    And DevQL pack health for semantic clones is ready in bitloops

  @devql @semantic-clones
  Scenario: Historical ingest preserves core history without backfilling semantic-clone history
    When I enqueue DevQL ingest task with status in bitloops
    Then semantic clone ingest does not populate historical semantic tables in bitloops

  @devql @semantic-clones
  Scenario: Current projection populates semantic-clone current tables
    When I enqueue DevQL sync task with status in bitloops
    Then semantic clone current projection tables are populated in bitloops

  @devql @semantic-clones
  Scenario: Current embeddings expose separate code and summary channels
    When I enqueue DevQL sync task with status in bitloops
    Then semantic clone current embeddings expose code and summary channels in bitloops

  @devql @semantic-clones
  Scenario: Sync drives embeddings before clone-edge rebuild fully drains
    When I enqueue DevQL sync task without status in bitloops
    Then semantic clone enrichments show embeddings before clone-edge rebuild work fully drains in bitloops

  @devql @semantic-clones
  Scenario: Commit snapshots current semantic-clone data into historical tables
    Given I set DevQL producer policy --sync=true --ingest=false in bitloops
    And I committed today in bitloops
    And I enqueue DevQL sync task with status in bitloops
    And I modify a semantic clone fixture source file in bitloops
    And I committed today in bitloops
    Then semantic clone historical tables are populated in bitloops
    And semantic clone historical and current embeddings expose code and summary channels in bitloops

  @devql @semantic-clones
  Scenario: Handler clones rank the cross-file execute peer above the weaker same-file helper
    When I enqueue DevQL sync task with status in bitloops
    Then DevQL clones query for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" returns at least 2 results in bitloops
    And DevQL clones results include score and relation_kind fields in bitloops
    And DevQL clones query for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" has highest-scored result with score above 0.60 in bitloops
    And DevQL clones query for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" ranks "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute" above "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship" in bitloops
    And DevQL clones query for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" returns results with explanation data in bitloops
    And DevQL clones query for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" with min_score 0.90 excludes "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship" in bitloops

  @devql @semantic-clones
  Scenario: DevQL clone summary returns grouped counts for the handler fixture
    When I enqueue DevQL sync task with status in bitloops
    Then DevQL clone summary for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" with min_score 0.60 returns grouped counts in bitloops

  @devql @semantic-clones
  Scenario: GraphQL clone summary returns grouped counts for the handler fixture
    When I enqueue DevQL sync task with status in bitloops
    Then GraphQL clone summary for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute" with min_score 0.60 returns grouped counts in bitloops
