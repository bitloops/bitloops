# Knowledge Capability BDD Plan (CLI-1432)

## 1. Goal

Add BDD coverage for the currently implemented Knowledge capability flows, aligned to:

- Confluence spec: `Knowledge - Feature Specification` (page `438337548`)
- Jira task: `CLI-1432`

Primary BDD scope (first pass):

- `knowledge add <url>`
- `knowledge add <url> --commit <sha>`
- `knowledge associate knowledge:<id> --to commit:<sha>`
- `knowledge associate knowledge:<id> --to knowledge:<id>`
- `knowledge associate knowledge:<id> --to checkpoint:<id>`
- `knowledge associate knowledge:<id> --to artefact:<id>`
- explicit source version refs:
  - `knowledge:<knowledge_item_id>:<knowledge_item_version_id>`
  - deprecated `knowledge_version:<knowledge_item_version_id>` (optional compatibility scenario)

## 2. Current Code Deep Dive (What Exists Today)

### 2.1 Active runtime path

The active Knowledge capability path is:

1. CLI command parsing in `bitloops/src/commands/devql.rs`
2. `run_knowledge_*_via_host` in `bitloops/src/engine/devql/capabilities/knowledge/cli.rs`
3. `DevqlCapabilityHost::invoke_ingester(...)`
4. ingesters registered in `knowledge/register.rs`:
   - `knowledge.add`
   - `knowledge.associate`
   - `knowledge.refresh`
   - `knowledge.versions`
5. service layer in `knowledge/services.rs`
6. storage gateways:
   - SQLite relational store (`sqlite_relational.rs`)
   - DuckDB document versions (`duckdb_documents.rs`)
   - blob payloads (`blob_payloads.rs`)

### 2.2 Important capability behavior already implemented

- URL parsing and provider detection (`knowledge/url.rs`)
  - GitHub issue + PR
  - Jira issue
  - Confluence page
  - unsupported URL fails early
- canonical identity and deterministic IDs (`storage/models.rs`)
- content-hash dedupe for immutable versions (`content_hash` + `has_knowledge_item_version`)
- repository-scoped item identity (`knowledge_item_id(repo_id, source_id)`)
- association resolution (`knowledge/refs.rs`)
  - source supports latest + explicit version
  - target supports commit/knowledge/checkpoint/artefact
  - versioned target refs rejected
- relation idempotency (`relation_assertion_id(...)` + SQLite `INSERT OR IGNORE`)
- provenance stamping for add/associate (`knowledge/provenance.rs`)
- checkpoint and artefact target validation in relational gateway

### 2.3 Storage contract actually enforced by code

- SQLite:
  - `knowledge_sources`
  - `knowledge_items`
  - `knowledge_relation_assertions`
- DuckDB:
  - `knowledge_document_versions`
- Blob:
  - payload key: `knowledge/<repo>/<item>/<version>/payload.json`

### 2.4 Existing tests we can leverage

Strong helper/test logic already exists in Knowledge test code (scenario matrix for most CLI-1432 cases), especially in:

- `bitloops/src/engine/devql/capabilities/knowledge/tests.rs` (flow matrix + helpers)
- `bitloops/src/engine/devql/capabilities/knowledge/services.rs` test module (stub connector + runtime context)

Reusable assets already present:

- temp repo bootstrap
- git commit fixtures
- checkpoint seeding helper
- artefact seeding helper
- sqlite + duckdb row assertions
- deterministic stub knowledge documents

### 2.5 Important nuance (must account for in plan)

`plugin.rs` and `knowledge/tests.rs` currently exist as reference, but `knowledge/mod.rs` does **not** include `pub mod plugin;` or `#[cfg(test)] mod tests;`. So those are not part of the active compiled module graph right now.

Implication for BDD implementation:

- do not couple BDD to dormant module wiring
- implement the BDD seam around active services/ingesters
- still reuse fixture ideas from existing tests

## 3. BDD Design Strategy

### 3.1 Test boundary

Keep BDD at the capability-flow boundary (as requested in CLI-1432), not parser-only and not low-level storage unit tests.

Implementation seam for BDD support module:

- `run_add_flow(...)`: wraps `ingestion.ingest_source(...)` and optional commit association
- `run_associate_flow(...)`: wraps `relations.associate_by_refs(...)`

These wrappers should sit in a test support module and use active `KnowledgeServices` + a stubbed runtime context.

### 3.2 Why this seam

It exercises exactly what we need:

- provider URL parsing and routing
- ingestion materialization behavior
- version reuse/new version behavior
- typed ref resolution
- relation creation and idempotency
- source-version binding
- failure rollback/no-partial-state behavior

without network calls and without brittle CLI stdout parsing.

## 4. File-by-File Implementation Plan

## Phase 0: Shared foundation (serial, mandatory)

### 4.1 Add dedicated Knowledge BDD support module

Create:

- `bitloops/src/engine/devql/tests/knowledge_support.rs`

Move/extract from existing Knowledge tests into this module:

- stub connector adapter and record queues
- test runtime context implementing `CapabilityIngestContext`
- repo/bootstrap helpers
- provider-config helpers
- checkpoint/artefact seed helpers
- sqlite/duckdb/blob assertion helpers
- placeholder substitution helper for `<item_id>`, `<version_id>`, etc.
- BDD flow wrappers:
  - `run_add_flow(...)`
  - `run_associate_flow(...)`

Code skeleton:

```rust
// bitloops/src/engine/devql/tests/knowledge_support.rs
pub(super) async fn run_add_flow(
    services: &KnowledgeServices,
    ctx: &mut TestRuntimeContext,
    url: &str,
    commit: Option<&str>,
) -> anyhow::Result<(IngestKnowledgeResult, Option<AssociateKnowledgeResult>)> {
    let ingest = services
        .ingestion
        .ingest_source(IngestKnowledgeRequest { url: url.to_string() }, ctx)
        .await?;

    let association = if let Some(commit) = commit {
        Some(services.relations.associate_to_commit(ctx, &ingest, commit).await?)
    } else {
        None
    };

    Ok((ingest, association))
}

pub(super) async fn run_associate_flow(
    services: &KnowledgeServices,
    ctx: &mut TestRuntimeContext,
    source_ref: &str,
    target_ref: &str,
) -> anyhow::Result<AssociateKnowledgeResult> {
    services
        .relations
        .associate_by_refs(ctx, source_ref, target_ref)
        .await
}
```

### 4.2 Restructure cucumber steps into modules

Current file is monolithic: `tests/cucumber_steps.rs`.

Refactor to:

- `bitloops/src/engine/devql/tests/cucumber_steps/mod.rs`
- `bitloops/src/engine/devql/tests/cucumber_steps/core.rs` (existing non-knowledge steps)
- `bitloops/src/engine/devql/tests/cucumber_steps/knowledge.rs` (new)

Update `devql/mod.rs` path include:

```rust
#[cfg(test)]
#[path = "tests/cucumber_steps/mod.rs"]
mod cucumber_steps;
```

### 4.3 Extend Cucumber world with Knowledge state

Update `bitloops/src/engine/devql/tests/cucumber_world.rs`.

Add fields:

```rust
pub(super) struct DevqlBddWorld {
    // existing fields...
    pub(super) knowledge: Option<KnowledgeBddHarness>,
    pub(super) last_ingest: Option<IngestKnowledgeResult>,
    pub(super) last_association: Option<AssociateKnowledgeResult>,
    pub(super) last_error: Option<anyhow::Error>,
    pub(super) ids: std::collections::HashMap<String, String>,
}
```

Add reset/init methods:

- `init_knowledge_harness()`
- `remember_id(alias, value)`
- `resolve_placeholders(input)`

## Phase 1: Step vocabulary and registration

### 4.4 Add dedicated knowledge step module

Create:

- `bitloops/src/engine/devql/tests/cucumber_steps/knowledge.rs`

Step groups:

- Given workspace/setup and stub provider responses
- Given seeded commit/checkpoint/artefact state
- When add/associate operations
- Then relational/document/blob assertions
- Then failure assertions (and no partial state)

Registration pattern:

```rust
pub(super) fn register(collection: Collection<DevqlBddWorld>) -> Collection<DevqlBddWorld> {
    collection
        .given(None, regex(r"^a Knowledge test workspace with configured providers$"), step_fn(given_workspace))
        .when(None, regex(r#"^the developer adds knowledge from "([^"]+)"$"#), step_fn(when_add))
        .when(None, regex(r#"^the developer associates "([^"]+)" to "([^"]+)"$"#), step_fn(when_associate))
        .then(None, regex(r"^exactly one knowledge relation assertion exists$"), step_fn(then_one_relation))
}
```

## Phase 2: Feature files (BDD scenarios)

Place under `bitloops/src/engine/devql/tests/features/`:

1. `knowledge_add.feature`
2. `knowledge_add_commit.feature`
3. `knowledge_associate_commit.feature`
4. `knowledge_associate_knowledge.feature`
5. `knowledge_associate_checkpoint_artefact.feature`
6. `knowledge_associate_explicit_version.feature`

Use Jira scenario mapping from CLI-1432.

## 5. Scenario Inventory (Exact Backlog)

## 5.1 `knowledge_add.feature` (CLI-1370)

- add GitHub issue
- add GitHub PR
- add Jira issue
- add Confluence page
- re-add unchanged -> same item + same version
- re-add changed -> same item + new version
- unsupported URL -> fail + zero persisted rows
- provider fetch failure -> fail + zero persisted rows
- add without `--commit` -> no relation assertion
- provenance stamped on knowledge source/item rows

## 5.2 `knowledge_add_commit.feature` (CLI-1371)

- add + attach to valid commit
- relation source version bound to ingested version
- add+commit with reused version binds to reused version
- invalid commit -> fail + no relation assertion

## 5.3 `knowledge_associate_commit.feature` (CLI-1372)

- `knowledge:<id>` to commit uses latest version
- explicit source version to commit
- same knowledge item -> multiple commits
- multiple knowledge items -> same commit
- missing source -> fail
- invalid commit target -> fail

## 5.4 `knowledge_associate_knowledge.feature` (CLI-1373)

- knowledge -> knowledge
- explicit source version -> knowledge
- one source -> multiple knowledge targets
- repeated same relation is idempotent
- missing target knowledge item -> fail
- versioned knowledge target rejected

## 5.5 `knowledge_associate_checkpoint_artefact.feature` (CLI-1374, CLI-1375)

Checkpoint:

- knowledge -> checkpoint
- explicit source version -> checkpoint
- missing checkpoint -> fail
- invalid checkpoint id format -> fail
- provenance target type/id correct

Artefact:

- knowledge -> artefact
- explicit source version -> artefact
- missing artefact -> fail
- invalid artefact id format -> fail
- same source can have both artefact and commit relations
- provenance target type/id correct

## 5.6 `knowledge_associate_explicit_version.feature` (cross-cut)

- `knowledge:<item_id>:<version_id>` -> commit
- `knowledge:<item_id>:<version_id>` -> knowledge
- `knowledge:<item_id>:<version_id>` -> checkpoint
- `knowledge:<item_id>:<version_id>` -> artefact
- version does not belong to item -> fail
- version does not exist -> fail
- versioned target rejected
- optional backward-compat: `knowledge_version:<version_id>` source

## 6. Example Gherkin (Representative)

```gherkin
Feature: Knowledge add command

  @KADD1
  Scenario: Re-adding unchanged source reuses item and version
    Given a Knowledge test workspace with configured providers
    And GitHub knowledge for "https://github.com/bitloops/bitloops/issues/42" returns in sequence:
      | title    | body       |
      | Issue 42 | First body |
      | Issue 42 | First body |
    When the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    And the developer adds knowledge from "https://github.com/bitloops/bitloops/issues/42"
    Then the last two ingests reuse the same knowledge item id
    And the last two ingests reuse the same knowledge item version id
    And exactly one knowledge document version exists
```

```gherkin
Feature: Knowledge associate explicit version

  @KVER5
  Scenario: Reject mismatched knowledge item and version pair
    Given a Knowledge test workspace with configured providers
    And the developer has already added knowledge from "https://github.com/bitloops/bitloops/issues/42" as "first_item"
    And the developer has already added knowledge from "https://bitloops.atlassian.net/browse/CLI-1370" as "second_item"
    When the developer associates "knowledge:<first_item_id>:<second_item_version_id>" to "commit:HEAD"
    Then the operation fails with message containing "does not belong"
    And zero knowledge relation assertions exist
```

## 7. Minimal Step Vocabulary

Use this exact vocabulary set to avoid drift and duplicate semantics:

- `Given a Knowledge test workspace with configured providers`
- `And the current repository has a valid HEAD commit`
- `And a checkpoint "<id>" exists`
- `And an artefact "<id>" exists`
- `And GitHub/Jira/Confluence knowledge for "<url>" returns ...`
- `And ... returns in sequence ...`
- `And ... fails with "<message>"`
- `When the developer adds knowledge from "<url>"`
- `When the developer adds knowledge from "<url>" and attaches it to "<rev>"`
- `When the developer associates "<source_ref>" to "<target_ref>"`
- `Then exactly N knowledge items/document versions/relation assertions exist`
- `Then the relation target type is "..."`
- `Then the relation source version equals "..."`
- `Then the operation fails with message containing "..."`

## 8. Assertions to Keep in BDD (Business-Level, Not Low-Level Unit)

Keep these BDD assertions explicit:

- item/version reuse semantics
- append-only relation semantics + idempotency
- source-version binding correctness
- target typing correctness (`commit`, `knowledge_item`, `checkpoint`, `artefact`)
- no partial state on failure
- provenance field stamping for knowledge operations

Avoid re-testing internal serialization minutiae already covered by unit tests.

## 9. Validation Commands

Local validation loop:

```bash
cargo test -p bitloops devql_bdd_features_pass
```

Then focused full capability tests (if needed):

```bash
cargo test -p bitloops knowledge
```

Expected guardrails:

- zero skipped cucumber steps (`fail_on_skipped` already enforced)
- no parse errors in feature files
- all existing non-knowledge BDD scenarios still pass

## 10. Delivery Sequence

1. Extract shared Knowledge fixtures into `knowledge_support.rs`
2. Split cucumber step file into modular structure (`core` + `knowledge`)
3. Extend BDD world with Knowledge runtime/result state
4. Add `knowledge_add.feature`
5. Add `knowledge_add_commit.feature`
6. Add association feature files (commit, knowledge, checkpoint/artefact)
7. Add explicit version feature file
8. Run BDD suite and de-duplicate step regexes

## 11. Parallelization Plan

Serial prerequisites:

- support module extraction
- world extension
- step module registration wiring

After prerequisites, parallel lanes:

- Lane A: `knowledge_add.feature`
- Lane B: `knowledge_add_commit.feature`
- Lane C: `knowledge_associate_commit.feature`
- Lane D: `knowledge_associate_knowledge.feature`
- Lane E: `knowledge_associate_checkpoint_artefact.feature`
- Lane F: explicit-version feature file

## 12. Risks and Mitigations

Risk: dormant `plugin.rs` path diverges from active services path.
Mitigation: implement BDD on active service/ingester flow wrappers; do not reactivate dormant module just for tests.

Risk: step explosion and regex collisions.
Mitigation: keep one clear vocabulary, isolate knowledge steps in dedicated module.

Risk: flaky git/cwd behavior in multi-scenario tests.
Mitigation: reuse existing process/git helpers from `test_support` and isolate temp repos per scenario.

Risk: over-coupling BDD to storage internals.
Mitigation: keep BDD assertions at flow-level invariants; leave serialization details to unit tests.

## 13. Done Criteria

Done for CLI-1432 when:

- all 6 Knowledge feature files exist with implemented scenarios from scope
- BDD steps are in dedicated `knowledge.rs` module
- shared test fixtures extracted into dedicated support module
- `devql_bdd_features_pass` passes with zero skipped steps
- Knowledge BDD covers add/add+commit/associate + explicit version source grammar and key failure paths
