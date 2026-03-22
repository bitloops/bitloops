Feature: Knowledge add command

  Scenario: KS-ADD-01 Add GitHub issue by URL
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | First issue body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the last operation succeeds
    And the last ingest provider is "github"
    And the last ingest source kind is "github_issue"
    And exactly 1 knowledge items exist
    And exactly 1 knowledge document versions exist
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ADD-02 Add GitHub pull request by URL
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/pull/1370" returns:
      | title | PR 1370 title |
      | body  | PR body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/pull/1370"
    Then the last operation succeeds
    And the last ingest provider is "github"
    And the last ingest source kind is "github_pull_request"
    And exactly 1 knowledge items exist
    And exactly 1 knowledge document versions exist

  Scenario: KS-ADD-03 Add Jira issue by URL
    Given a Knowledge test workspace with configured providers
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Jira body |
    When the developer adds knowledge from "https://bitloops.atlassian.net/browse/CLI-1370"
    Then the last operation succeeds
    And the last ingest provider is "jira"
    And the last ingest source kind is "jira_issue"
    And exactly 1 knowledge items exist
    And exactly 1 knowledge document versions exist

  Scenario: KS-ADD-04 Add Confluence page by URL
    Given a Knowledge test workspace with configured providers
    And Confluence knowledge for "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge" returns:
      | title | Knowledge page |
      | body  | Confluence body |
    When the developer adds knowledge from "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge"
    Then the last operation succeeds
    And the last ingest provider is "confluence"
    And the last ingest source kind is "confluence_page"
    And exactly 1 knowledge items exist
    And exactly 1 knowledge document versions exist

  Scenario: KS-ADD-05 Re-add unchanged source reuses item and version
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Stable body |
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Stable body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    And the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the last operation succeeds
    And the last two ingests reuse the same knowledge item id
    And the last two ingests reuse the same knowledge item version id
    And exactly 1 knowledge items exist
    And exactly 1 knowledge document versions exist

  Scenario: KS-ADD-06 Re-add changed source reuses item and creates new version
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    And the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the last operation succeeds
    And the last two ingests reuse the same knowledge item id
    And the last two ingests have different knowledge item version ids
    And exactly 1 knowledge items exist
    And exactly 2 knowledge document versions exist

  Scenario: KS-ADD-07 Unsupported URL fails early
    Given a Knowledge test workspace with configured providers
    When the developer adds knowledge from "https://example.com/docs/unsupported"
    Then the operation fails with message containing "unsupported knowledge URL"
    And exactly 0 knowledge items exist
    And exactly 0 knowledge document versions exist
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ADD-08 Provider fetch failure leaves no partial state
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" fails with "simulated provider failure"
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the operation fails with message containing "simulated provider failure"
    And exactly 0 knowledge items exist
    And exactly 0 knowledge document versions exist
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ADD-09 Add without commit creates no relation assertion
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the last operation succeeds
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ADD-10 Ingestion rows are provenance stamped
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the last operation succeeds
    And all knowledge source rows are stamped for "knowledge.add"
    And all knowledge item rows are stamped for "knowledge.add"
