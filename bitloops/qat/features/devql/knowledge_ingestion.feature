Feature: Knowledge ingestion rejection handling

  The default knowledge lane keeps only the deterministic unsupported-URL
  rejection path. Knowledge items must not be synthesized when the add
  command fails, and the repository should remain empty afterward.

  Background:
    Given I run CleanStart for flow "KnowledgeIngestion"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL ingest task with status in bitloops

  @devql @knowledge
  Scenario: Unsupported URL fails cleanly without partial persistence
    Given I attempt to add knowledge URL "https://unsupported-provider.example.com/doc/123" in bitloops
    Then the knowledge add command fails with an error in bitloops
    And DevQL knowledge query returns 0 items in bitloops
