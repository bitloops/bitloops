Feature: Knowledge associate command for checkpoint and artefact targets

  Scenario: KS-ASCHK-01 Associate knowledge item to existing checkpoint
    Given a Knowledge test workspace with configured providers
    And a checkpoint "a1b2c3d4e5f6" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "checkpoint:a1b2c3d4e5f6"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "checkpoint"
    And the relation target id equals "a1b2c3d4e5f6"

  Scenario: KS-ASCHK-02 Associate explicit source version to checkpoint
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
    And the relation source version equals "<source_first_version_id>"
    And the relation target type is "checkpoint"

  Scenario: KS-ASCHK-03 Missing checkpoint target fails
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "checkpoint:a1b2c3d4e5f6"
    Then the operation fails with message containing "not found"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ASCHK-04 Invalid checkpoint identifier format fails
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "checkpoint:not-valid"
    Then the operation fails with message containing "not a valid checkpoint identifier"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ASCHK-05 Checkpoint relation provenance stamping is correct
    Given a Knowledge test workspace with configured providers
    And a checkpoint "a1b2c3d4e5f6" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "checkpoint:a1b2c3d4e5f6"
    Then the last operation succeeds
    And the latest relation provenance has fields:
      | key | value |
      | capability | knowledge |
      | operation | knowledge.associate |
      | target_type | checkpoint |
      | target_id | a1b2c3d4e5f6 |

  Scenario: KS-ASART-01 Associate knowledge item to existing artefact
    Given a Knowledge test workspace with configured providers
    And an artefact "bbbbbbbb-1111-2222-3333-444444444444" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "artefact:bbbbbbbb-1111-2222-3333-444444444444"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "artefact"
    And the relation target id equals "bbbbbbbb-1111-2222-3333-444444444444"

  Scenario: KS-ASART-02 Associate explicit source version to artefact
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
    And the relation source version equals "<source_first_version_id>"
    And the relation target type is "artefact"

  Scenario: KS-ASART-03 Missing artefact target fails
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "artefact:bbbbbbbb-1111-2222-3333-444444444444"
    Then the operation fails with message containing "not found"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ASART-04 Invalid artefact identifier format fails
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "artefact:not-a-valid-uuid"
    Then the operation fails with message containing "not a valid artefact identifier"
    And exactly 0 knowledge relation assertions exist

  Scenario: KS-ASART-05 Same knowledge item can hold both artefact and commit relations
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And an artefact "bbbbbbbb-1111-2222-3333-444444444444" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "artefact:bbbbbbbb-1111-2222-3333-444444444444"
    And the developer associates "knowledge:<source_item_id>" to "commit:HEAD"
    Then the last operation succeeds
    And exactly 2 knowledge relation assertions exist
    And relation target types include:
      | target_type |
      | artefact |
      | commit |

  Scenario: KS-ASART-06 Artefact relation provenance stamping is correct
    Given a Knowledge test workspace with configured providers
    And an artefact "bbbbbbbb-1111-2222-3333-444444444444" exists
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Source body |
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "source"
    When the developer associates "knowledge:<source_item_id>" to "artefact:bbbbbbbb-1111-2222-3333-444444444444"
    Then the last operation succeeds
    And the latest relation provenance has fields:
      | key | value |
      | capability | knowledge |
      | operation | knowledge.associate |
      | target_type | artefact |
      | target_id | bbbbbbbb-1111-2222-3333-444444444444 |
