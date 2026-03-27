Feature: Knowledge ingestion, versioning, and association

  The Knowledge capability allows developers to link external context
  (Confluence pages, Jira issues, GitHub issues/PRs) to their repository.
  Knowledge items must maintain stable identity, correct versioning, and
  preserved associations.

  Background:
    Given I run CleanStart for flow "KnowledgeIngestion"
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run EnableCLI for bitloops
    And I run DevQL init in bitloops
    And I run DevQL ingest in bitloops

  @devql @knowledge @requires-network
  Scenario: Add a knowledge item by URL and verify it appears in inventory
    Given I add knowledge URL "https://github.com/bitloops/bitloops/issues/1" in bitloops
    Then DevQL knowledge query returns at least 1 item in bitloops
    And knowledge item has provider "github" and source_kind "issue" in bitloops

  @devql @knowledge @requires-network
  Scenario: Add knowledge with commit association
    Given I add knowledge URL "https://github.com/bitloops/bitloops/issues/1" with commit association in bitloops
    Then DevQL knowledge query returns at least 1 item in bitloops
    And knowledge item is associated to a commit in bitloops

  @devql @knowledge @requires-network
  Scenario: Associate knowledge to another knowledge item
    Given I add knowledge URL "https://github.com/bitloops/bitloops/issues/1" in bitloops
    And I add knowledge URL "https://github.com/bitloops/bitloops/pull/2" in bitloops
    And I associate knowledge "https://github.com/bitloops/bitloops/issues/1" to knowledge "https://github.com/bitloops/bitloops/pull/2" in bitloops
    Then DevQL knowledge query returns at least 2 items in bitloops

  @devql @knowledge @requires-network
  Scenario: Refresh unchanged knowledge does not create a new version
    Given I add knowledge URL "https://github.com/bitloops/bitloops/issues/1" in bitloops
    And I refresh knowledge "https://github.com/bitloops/bitloops/issues/1" in bitloops
    Then knowledge versions for "https://github.com/bitloops/bitloops/issues/1" shows exactly 1 version in bitloops

  @devql @knowledge
  Scenario: Unsupported URL fails cleanly without partial persistence
    Given I attempt to add knowledge URL "https://unsupported-provider.example.com/doc/123" in bitloops
    Then the knowledge add command fails with an error in bitloops
    And DevQL knowledge query returns 0 items in bitloops
