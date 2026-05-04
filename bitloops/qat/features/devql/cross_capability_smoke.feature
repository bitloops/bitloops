Feature: Cross-capability deterministic smoke

  This is the integrated offline smoke for the hardened DevQL surfaces:
  agent queryability, blast radius, TestHarness, and semantic clones
  should all remain queryable in the same repository after a real
  agent-driven change.

  Background:
    Given I run CleanStart for flow "CrossCapabilitySmoke"
    And I start the daemon in bitloops
    And I create a TypeScript project with tests and coverage in bitloops
    And I add semantic clone fixtures in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I configure semantic clones with fake embeddings runtime in bitloops
    And I configure context guidance with fake text-generation runtime in bitloops
    And I run DevQL init in bitloops
    And DevQL pack health for semantic clones is ready in bitloops
    And I make a first change using Claude Code to bitloops
    And I committed today in bitloops
    And I run DevQL semantic clones rebuild in bitloops
    And I run TestHarness ingest-tests for latest commit in bitloops
    And I run TestHarness ingest-coverage for latest commit in bitloops

  @devql @integration @develop_gate
  Scenario: Hardened DevQL capability surfaces compose in one offline workflow
    Then checkpoint mapping exists in bitloops
    And claude-code session exists in bitloops
    And DevQL checkpoints query returns results for "claude-code" in bitloops
    And DevQL artefacts query returns results in bitloops
    And DevQL deps query for "UserService.createUser" with direction "in" and asOf latest commit returns at least 1 result in bitloops
    And TestHarness query for "UserService.createUser" at current workspace state with view "summary" returns results in bitloops
    And DevQL clones query for "renderInvoice" returns at least 1 result in bitloops
    And DevQL context guidance query for "src/services/user-service.ts" returns at least 1 item in bitloops
    And DevQL context guidance query for "src/services/user-service.ts" includes kind "qat_mocked_guidance_generation" in bitloops

  @devql @integration @develop_gate @devql_search
  Scenario: Unified selectArtefacts search remains queryable across fuzzy and semantic lanes
    Then DevQL selectArtefacts search for "rendrInvoice()" returns symbol "src/render/render-invoice.ts::renderInvoice" in bitloops
    And DevQL selectArtefacts search for "invoice document renderer" returns at least 1 result in bitloops
