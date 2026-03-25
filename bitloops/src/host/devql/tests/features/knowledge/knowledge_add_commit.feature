Feature: Knowledge add command with commit attachment

  Scenario: KS-ADDCOMMIT-01 Add and attach to existing commit
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42" and attaches it to "HEAD"
    Then the last operation succeeds
    And exactly 1 knowledge relation assertions exist
    And the relation target type is "commit"
    And the relation target id equals "HEAD"
    And the relation type is "associated_with"
    And the association method is "manual_attachment"

  Scenario: KS-ADDCOMMIT-02 Add+commit relation binds exact source version from add
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42" and attaches it to "HEAD"
    Then the last operation succeeds
    And the relation source version equals "<version_id>"

  Scenario: KS-ADDCOMMIT-03 Add+commit with reused version binds reused version
    Given a Knowledge test workspace with configured providers
    And the current repository has a valid HEAD commit
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Stable body |
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Stable body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    And the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42" and attaches it to "HEAD"
    Then the last operation succeeds
    And the last two ingests reuse the same knowledge item version id
    And the relation source version equals "<version_id>"
    And exactly 1 knowledge relation assertions exist

  Scenario: KS-ADDCOMMIT-04 Invalid commit fails and creates no relation
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns:
      | title | Issue 42 title |
      | body  | Body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42" and attaches it to "not-a-commit"
    Then the operation fails with message containing "validating commit"
    And exactly 0 knowledge relation assertions exist
