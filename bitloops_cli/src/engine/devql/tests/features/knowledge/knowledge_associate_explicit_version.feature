Feature: Knowledge associate explicit source version references

  Scenario: KS-EXPLVER-01 Explicit source version to commit
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
    And the relation target type is "commit"
    And the relation source version equals "<source_first_version_id>"

  Scenario: KS-EXPLVER-02 Explicit source version to knowledge target
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
    And the relation target type is "knowledge_item"
    And the relation source version equals "<source_first_version_id>"

  Scenario: KS-EXPLVER-03 Explicit source version to checkpoint target
    Given a Knowledge test workspace with configured providers
    And a checkpoint "a1b2c3d4e5f6" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    And the developer has already added two versions from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>:<source_first_version_id>" to "checkpoint:a1b2c3d4e5f6"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "checkpoint"
    And the relation source version equals "<source_first_version_id>"

  Scenario: KS-EXPLVER-04 Explicit source version to artefact target
    Given a Knowledge test workspace with configured providers
    And an artefact "bbbbbbbb-1111-2222-3333-444444444444" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    And the developer has already added two versions from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>:<source_first_version_id>" to "artefact:bbbbbbbb-1111-2222-3333-444444444444"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "artefact"
    And the relation source version equals "<source_first_version_id>"

  Scenario: KS-EXPLVER-05 Reject version that does not belong to source item
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Other body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "first_item"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "second_item"
    When the developer associates "knowledge:<first_item_item_id>:<second_item_item_version_id>" to "commit:HEAD"
    Then the operation fails with message containing "does not belong"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-EXPLVER-06 Reject non-existent source version
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>:missing-version-id" to "commit:HEAD"
    Then the operation fails with message containing "not found"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-EXPLVER-07 Support versioned knowledge target ref
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And Jira knowledge for "https://bitloops.atlassian.net/browse/CLI-1370" returns:
      | title | CLI-1370 |
      | body  | Target body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "target"
    When the developer associates "knowledge:<source_item_id>:<source_item_version_id>" to "knowledge:<target_item_id>:<target_item_version_id>"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "knowledge_item"
    And the relation target id equals "<target_item_id>"
    And the relation source version equals "<source_item_version_id>"
    And the latest relation provenance has fields:
      | key                                | value                    |
      | target_knowledge_item_version_id   | <target_item_version_id> |

  Scenario: KS-EXPLVER-08 Deprecated knowledge_version source compatibility
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title | body |
      | Issue 42 title | First body |
      | Issue 42 title | Updated body |
    And the developer has already added two versions from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge_version:<source_first_version_id>" to "commit:HEAD"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation source version equals "<source_first_version_id>"
