Feature: DevQL context guidance smoke

  This is the focused offline smoke for context guidance generation and
  DevQL queryability. It avoids the broader cross-capability background
  so failures point directly at guidance runtime, distillation, storage,
  or query plumbing.

  @devql @integration @develop_gate @context_guidance
  Scenario: Context guidance generated with fake runtime is queryable
    Given I run CleanStart for flow "ContextGuidanceFakeRuntime"
    And I start the daemon in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent claude-code --sync=false --ingest=true in bitloops
    And I configure context guidance with fake text-generation runtime in bitloops
    And I run DevQL init in bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I enqueue DevQL ingest task with status in bitloops
    And I enqueue DevQL sync task with status in bitloops
    Then daemon enrichments eventually drain in bitloops
    And DevQL context guidance query for "src/services/user-service.ts" returns at least 1 item in bitloops
    And DevQL context guidance query for "src/services/user-service.ts" includes kind "qat_mocked_guidance_generation" in bitloops
