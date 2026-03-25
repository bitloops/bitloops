Feature: Test-harness coverage mapping

  # --- Happy paths ---

  Scenario: S1 – Ingest valid LCOV report for committed snapshot
    Given an initialized Rust coverage repository with production artefacts for commit "C1"
    When I ingest a valid LCOV report for commit "C1"
    Then coverage captures are stored for commit "C1"
    And coverage hits include line and branch data for commit "C1"

  Scenario: S2 – Map file coverage to artefact spans
    Given an initialized Rust coverage repository with production artefacts for commit "C1"
    When I ingest a valid LCOV report for commit "C1"
    Then coverage hits are attributed only to lines within artefact spans for commit "C1"

  Scenario: S3 – Return artefact-level coverage via query
    Given an initialized Rust coverage repository with production artefacts and tests for commit "C1"
    When I ingest a valid LCOV report for commit "C1"
    Then querying artefact "find_by_id" returns coverage with line and branch percentages for commit "C1"

  Scenario: S4 – Surface uncovered branch data
    Given an initialized Rust coverage repository with production artefacts and tests for commit "C1"
    When I ingest a valid LCOV report for commit "C1"
    Then querying artefact "find_by_id" returns uncovered branches for commit "C1"

  # --- Alternative paths ---

  Scenario: S5 – Line-only coverage fallback
    Given an initialized Rust coverage repository with production artefacts for commit "C1"
    When I ingest an LCOV report with line coverage but no branch data for commit "C1"
    Then coverage captures are stored for commit "C1"
    And coverage hits include line data but no branch data for commit "C1"

  # --- Edge cases ---

  Scenario: E1 – Partial artefact overlap in a covered file
    Given an initialized Rust coverage repository with multiple artefacts for commit "C1"
    When I ingest a valid LCOV report for commit "C1"
    Then each artefact receives only the coverage hits within its own span for commit "C1"

  Scenario: E2 – Unmappable report paths emit diagnostics
    Given an initialized Rust coverage repository with production artefacts for commit "C1"
    When I ingest an LCOV report with unmappable file paths for commit "C1"
    Then coverage diagnostics include "unmapped_file" entries for commit "C1"
    And the mapped files still produce coverage hits for commit "C1"

  # --- Error cases ---

  Scenario: ERR1 – Malformed coverage report lines emit diagnostics
    Given an initialized Rust coverage repository with production artefacts for commit "C1"
    When I ingest an LCOV report with malformed DA lines for commit "C1"
    Then coverage diagnostics include "malformed_line" entries for commit "C1"
    And valid lines from the same report still produce coverage hits for commit "C1"

  Scenario: ERR3 – Missing source file in snapshot continues processing
    Given an initialized Rust coverage repository with production artefacts for commit "C1"
    When I ingest an LCOV report referencing a missing source file for commit "C1"
    Then coverage diagnostics include "unmapped_file" entries for commit "C1"
    And coverage hits from other files in the report are still persisted for commit "C1"
