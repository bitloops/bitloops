Feature: Knowledge associate command for commit targets

  Scenario: KS-ASCOMMIT-01 Associate knowledge item to commit using latest version
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    And the developer has already added two versions from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "commit:HEAD"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "commit"
    And the relation source version equals "<source_second_version_id>"

  Scenario: KS-ASCOMMIT-02 Associate explicit source version to commit
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    And the developer has already added two versions from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>:<source_first_version_id>" to "commit:HEAD"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation source version equals "<source_first_version_id>"

  Scenario: KS-ASCOMMIT-03 One knowledge item can be associated to multiple commits
    Given a Knowledge test workspace with configured providers
    And the repository has two valid commits
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "commit:<first_commit_sha>"
    And the developer associates "knowledge:<source_item_id>" to "commit:<second_commit_sha>"
    Then the last operation succeeds
    And exactly 2 knowledge relation assertions exist
    And the relation target id equals "<second_commit_sha>"

  Scenario: KS-ASCOMMIT-04 Multiple knowledge items can be associated to same commit
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Jira body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source_a"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "source_b"
    When the developer associates "knowledge:<source_a_item_id>" to "commit:HEAD"
    And the developer associates "knowledge:<source_b_item_id>" to "commit:HEAD"
    Then the last operation succeeds
    And exactly 2 knowledge relation assertions exist
    And the relation target id equals "HEAD"

  Scenario: KS-ASCOMMIT-05 Missing knowledge source fails cleanly
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    When the developer associates "knowledge:missing-item-id" to "commit:HEAD"
    Then the operation fails with message containing "not found"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ASCOMMIT-06 Invalid commit target fails cleanly
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "commit:not-a-commit"
    Then the operation fails with message containing "validating commit"
    And exactly 0 knowledge relation assertions exist
