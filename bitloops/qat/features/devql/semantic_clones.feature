Feature: Semantic Clones pattern-reuse discovery

  Semantic Clones should surface the best local implementation patterns
  to follow. Results must be explainable, rankable, and filterable by
  score and relation kind. The rebuild step is strict: it must produce
  clone edges, otherwise the scenario should fail.

  Background:
    Given I run CleanStart for flow "SemanticClones"
    And I start the daemon in bitloops
    And I create a TypeScript project with similar implementations in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I configure semantic clones with fake embeddings runtime in bitloops
    And I run DevQL init in bitloops
    And DevQL pack health for semantic clones is ready in bitloops
    And I run DevQL semantic clones rebuild in bitloops

  @devql @semantic-clones
  Scenario: Clones query returns similar implementations for a known artefact
    Then DevQL clones query for "renderInvoice" returns at least 1 result in bitloops
    And DevQL clones results include score and relation_kind fields in bitloops

  @devql @semantic-clones
  Scenario: Score filtering reduces result set
    Then DevQL clones query for "renderInvoice" with min_score 0.3 returns results in bitloops
    And DevQL clones query for "renderInvoice" with min_score 0.95 returns fewer or equal results in bitloops

  @devql @semantic-clones
  Scenario: Strong local patterns rank ahead of weak matches
    Then DevQL clones query for "renderInvoice" has highest-scored result with score above 0.5 in bitloops

  @devql @semantic-clones
  Scenario: Clone results include explanation payload
    Then DevQL clones query for "renderInvoice" returns results with explanation data in bitloops
