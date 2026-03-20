Feature: Knowledge associate command for knowledge targets

  Scenario: KS-ASKNOW-01 Associate knowledge item to another knowledge item
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Target body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "target"
    When the developer associates "knowledge:<source_item_id>" to "knowledge:<target_item_id>"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "knowledge_item"
    And the relation target id equals "<target_item_id>"

  Scenario: KS-ASKNOW-02 Associate explicit source version to knowledge item
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Target body |
    And the developer has already added two versions from "https://github.com/bitloops/bitloops/issues/42" as "source"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "target"
    When the developer associates "knowledge:<source_item_id>:<source_first_version_id>" to "knowledge:<target_item_id>"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation source version equals "<source_first_version_id>"
    And the relation target type is "knowledge_item"

  Scenario: KS-ASKNOW-03 One source knowledge item to multiple knowledge targets
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Target A body |
    And Confluence knowledge for "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge" returns:
      | title | Knowledge page |
      | body  | Target B body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "target_a"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge" as "target_b"
    When the developer associates "knowledge:<source_item_id>" to "knowledge:<target_a_item_id>"
    And the developer associates "knowledge:<source_item_id>" to "knowledge:<target_b_item_id>"
    Then the last operation succeeds
    And exactly 2 knowledge relation assertions exist
    And relation target ids include:
      | target_id |
      | <target_a_item_id> |
      | <target_b_item_id> |

  Scenario: KS-ASKNOW-04 Repeating same association is idempotent
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Target body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "target"
    When the developer associates "knowledge:<source_item_id>" to "knowledge:<target_item_id>"
    And the developer associates "knowledge:<source_item_id>" to "knowledge:<target_item_id>"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist

  Scenario: KS-ASKNOW-05 Missing target knowledge item fails
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "knowledge:missing-target-id"
    Then the operation fails with message containing "target knowledge item"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ASKNOW-06 Versioned knowledge target is supported
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Target body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "target"
    When the developer associates "knowledge:<source_item_id>" to "knowledge:<target_item_id>:<target_item_version_id>"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "knowledge_item"
    And the relation target id equals "<target_item_id>"
    And the relation source version equals "<source_item_version_id>"
