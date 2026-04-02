@semantic-clones
Feature: Semantic Clones BDD scenarios

  @SemanticClones-S1
  Scenario: S1 Similar implementations satisfy clone thresholds
    Given the semantic clone fixture "similar implementations" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/pdf.ts::createInvoicePdf")->clones(min_score:0.55)->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                    | relation_kind           |
      | src/render.ts::renderInvoiceDocument | similar_implementation |
    And the clone row for "src/render.ts::renderInvoiceDocument" has metric "score" at least 0.55
    And the clone row for "src/render.ts::renderInvoiceDocument" has metric "semantic_score" at least 0.40
    And the clone row for "src/render.ts::renderInvoiceDocument" has shared signal "call_targets"
    And every clone row includes explainable scores

  # Current scorer behavior still requires matching signature-shape hashes for exact_duplicate.
  @SemanticClones-S2
  Scenario: S2 Exact duplicates expose deterministic duplicate signals
    Given the semantic clone fixture "exact duplicates" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/services/fetch-order.ts::fetch_order")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                         | relation_kind   |
      | src/services/order_copies.ts::fetch_order | exact_duplicate |
    And the clone row for "src/services/order_copies.ts::fetch_order" has metric "score" at least 0.99
    And the clone row for "src/services/order_copies.ts::fetch_order" has duplicate signal "body_hash_match" set to true
    And the clone row for "src/services/order_copies.ts::fetch_order" has duplicate signal "signature_shape_match" set to true
    And every clone row includes explainable scores

  # Confluence still uses extraction_candidate wording; current implementation emits shared_logic_candidate.
  @SemanticClones-S3
  Scenario: S3 Shared logic candidates satisfy threshold evidence
    Given the semantic clone fixture "shared logic candidates" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/billing/invoice.ts::create_invoice_pdf")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                                         | relation_kind           |
      | src/billing/invoice_helpers.ts::build_invoice_pdf_bundle  | shared_logic_candidate |
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has metric "lexical_score" at least 0.68
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has metric "semantic_score" at least 0.42
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has metric "structural_score" at least 0.58
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has metric "body_overlap" at least 0.50
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has shared signal "body_tokens"
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has shared signal "call_targets"
    And the clone row for "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle" has shared signal "dependency_targets"
    And every clone row includes explainable scores

  @SemanticClones-S4
  Scenario: S4 Diverged implementations expose drift-style evidence
    Given the semantic clone fixture "diverged implementations" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/validation/checkout.ts::validate_order_checkout")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                              | relation_kind            |
      | src/validation/draft.ts::validate_order_draft | diverged_implementation |
    And the clone row for "src/validation/draft.ts::validate_order_draft" has metric "semantic_score" at least 0.55
    And the clone row for "src/validation/draft.ts::validate_order_draft" has metric "body_overlap" at least 0.08
    And the clone row for "src/validation/draft.ts::validate_order_draft" has metric "body_overlap" at most 0.45
    And the clone row for "src/validation/draft.ts::validate_order_draft" has metric "call_overlap" at most 0.25
    And the clone row for "src/validation/draft.ts::validate_order_draft" has shared signal "dependency_targets"
    And the clone row for "src/validation/draft.ts::validate_order_draft" has limiting signal "no_shared_calls"
    And every clone row includes explainable scores

  # Current implementation prefers local patterns via churn/path heuristics rather than explicit default-branch metadata.
  @SemanticClones-S5
  Scenario: S5 Preferred local patterns rank above weaker neighbours
    Given the semantic clone fixture "preferred local patterns" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/rendering/invoices.ts::render_invoice_document")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                           | relation_kind           |
      | src/billing/invoice.ts::create_invoice_pdf | similar_implementation |
    And the first clone row targets "src/billing/invoice.ts::create_invoice_pdf"
    And the first clone row has label "preferred_local_pattern"
    And the clone row for "src/rendering/preview.ts::render_invoice_preview" ranks below "src/billing/invoice.ts::create_invoice_pdf"
    And the clone row for "src/rendering/preview.ts::render_invoice_preview" does not have label "preferred_local_pattern"
    And every clone row includes explainable scores

  @SemanticClones-S6
  Scenario: S6 Weak clone candidates keep locality bias explicit
    Given the semantic clone fixture "weak clone candidates" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"method",symbol_fqn:"src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::execute")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                                                        | relation_kind         |
      | src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command | weak_clone_candidate |
    And the clone row for "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command" has bias warning "same_file_bias"
    And the clone row for "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command" has explanation fact "locality_dominates" set to true
    And the clone row for "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command" does not have label "preferred_local_pattern"
    And every clone row includes explainable scores

  @SemanticClones-E1
  Scenario: E1 Stage 1 persisted summary falls back to template
    Given a Stage 1 semantic input for a "method" named "getById"
    And the Stage 1 docstring is empty
    And the Stage 1 summary provider returns invalid candidate:
      """
      short
      """
    When Stage 1 persists semantic feature rows through the original pipeline
    Then the persisted Stage 1 final summary is "Method get by id."
    And the persisted Stage 1 template summary is "Method get by id."

  @SemanticClones-E1-detail
  Scenario: E1 Stage 1 persisted summary appends a valid detail clause
    Given a Stage 1 semantic input for a "method" named "getById"
    And the Stage 1 docstring is empty
    And the Stage 1 summary provider returns:
      """
      Loads a user entity by id from storage.
      """
    When Stage 1 persists semantic feature rows through the original pipeline
    Then the persisted Stage 1 final summary is "Method get by id. Loads a user entity by id from storage."
    And the persisted Stage 1 template summary is "Method get by id."

  @SemanticClones-E2
  Scenario: E2 Comments do not override implementation truth
    Given a Stage 1 semantic input for a "function" named "normalizeEmail"
    And the Stage 1 docstring is:
      """
      Delete a user account.
      """
    And the Stage 1 summary provider returns no candidate
    When Stage 1 persists semantic feature rows through the original pipeline
    Then the persisted Stage 1 final summary starts with "Function normalize email."
    And the persisted Stage 1 final summary is not "Delete a user account."

  # Current implementation still rebuilds clone edges repo-wide; this scenario asserts observable reuse/stability, not neighbourhood-only Stage 3 recompute.
  @SemanticClones-E3
  Scenario: E3 Incremental indexing reuses unchanged Stage 1 and Stage 2 work
    Given the semantic clone incremental fixture "single changed artefact" is prepared
    When semantic clone incremental indexing runs across two snapshots
    Then Stage 1 incremental stats are 1 upserted and 2 skipped
    And Stage 2 incremental stats are 1 upserted and 2 skipped
    And the semantic features hash for "src/billing/common.ts::formatInvoiceTotal" is unchanged across snapshots
    And the embedding hash for "src/billing/common.ts::formatInvoiceTotal" is unchanged across snapshots
    And the clone edge hash from "src/billing/common.ts::formatInvoiceTotal" to "src/billing/render.ts::renderInvoiceLine" is unchanged across snapshots
    And the clone edge hash from "src/billing/create.ts::createInvoice" to "src/billing/render.ts::renderInvoiceLine" changes across snapshots

  @SemanticClones-ERR1
  Scenario: ERR1 Missing embedding configuration fails clearly
    Given a Stage 1 semantic input for a "function" named "normalizeEmail"
    And the Stage 1 docstring is empty
    And the Stage 1 summary provider returns no candidate
    When Stage 2 starts with invalid embedding provider configuration
    Then Stage 2 fails with message containing "spawning embeddings runtime"
    And Stage 2 writes 0 embedding rows

  @SemanticClones-ERR1-profile
  Scenario: ERR1 Missing embedding profile fails clearly
    Given a Stage 1 semantic input for a "function" named "normalizeEmail"
    And the Stage 1 docstring is empty
    And the Stage 1 summary provider returns no candidate
    When Stage 2 starts with embedding provider configuration "missing embedding profile"
    Then Stage 2 fails with message containing "embedding profile `missing-profile` is not defined"
    And Stage 2 writes 0 embedding rows

  # S6 already covers the pure same-file weak-neighbour case. The scenarios below add cross-file handler calibration.
  @semantic-clones-quality
  @SemanticClones-Q1
  Scenario: Q1 Generic execute prefers a cross-file handler over same-file helper noise
    Given the semantic clone fixture "generic execute handlers" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"method",symbol_fqn:"src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                                                                                                                                     | relation_kind           |
      | src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute                                                       | similar_implementation |
      | src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship | weak_clone_candidate |
    And the first clone row targets "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute"
    And the clone row for "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute" has metric "score" at least 0.60
    And the clone row for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship" ranks below "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute"
    And the clone row for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship" has bias warning "same_file_bias"
    And the clone row for "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship" has explanation fact "locality_dominates" set to true
    And every clone row includes explainable scores

  @semantic-clones-quality
  @SemanticClones-Q2
  Scenario: Q2 min_score removes the weak same-file helper but keeps the stronger cross-file handler
    Given the semantic clone fixture "generic execute handlers" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"method",symbol_fqn:"src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute")->clones(min_score:0.60)->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                                                                                   | relation_kind           |
      | src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute    | similar_implementation |
    And no clone row targets "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship"

  @semantic-clones-quality
  @SemanticClones-Q3
  Scenario: Q3 relation_kind filtering excludes the weak same-file helper
    Given the semantic clone fixture "generic execute handlers" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"method",symbol_fqn:"src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute")->clones(relation_kind:"similar_implementation",min_score:0.60)->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                                                                                   | relation_kind           |
      | src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute    | similar_implementation |
    And no clone row targets "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship"

  @semantic-clones-quality
  @SemanticClones-Q4
  Scenario: Q4 A second generic execute source is not dominated by same-file locality
    Given the semantic clone fixture "generic execute handlers" is indexed
    When clones() query executes:
      """
      repo("temp2")->artefacts(kind:"method",symbol_fqn:"src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute")->clones()->limit(10)
      """
    Then clone rows include:
      | target_symbol_fqn                                                                                                                                    | relation_kind           |
      | src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute                                                  | similar_implementation |
      | src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForInstanceInSnapshotRelationship | weak_clone_candidate |
    And the first clone row targets "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute"
    And the clone row for "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForInstanceInSnapshotRelationship" ranks below "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute"
    And the clone row for "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForInstanceInSnapshotRelationship" has bias warning "same_file_bias"
    And the clone row for "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForInstanceInSnapshotRelationship" has explanation fact "locality_dominates" set to true
