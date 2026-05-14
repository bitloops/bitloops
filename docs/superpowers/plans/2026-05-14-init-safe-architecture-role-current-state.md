# Init-Safe Architecture Role Current-State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep `bitloops init --install-default-daemon` completion scoped to semantic init work while still running architecture role classification from sync/current-state updates.

**Architecture:** Add an explicit init-completion policy to capability cursor mailboxes. Only semantic clones current-state work opts into the init barrier; architecture graph and architecture role current-state work remain background current-state consumers. Split role classification out of the architecture graph snapshot consumer into its own background cursor consumer so sync still drives role facts/rules/assignments/adjudication without blocking init.

**Tech Stack:** Rust, Tokio, rusqlite-backed runtime store, capability host/workplane APIs, cargo-nextest via repo Cargo aliases.

---

## File Structure

- Modify `bitloops/src/host/capability_host/registrar.rs`
  - Add `CapabilityMailboxInitPolicy`.
  - Store the policy on `CapabilityMailboxRegistration`.
  - Add a builder method for blocking init completion.
  - Add tests for default/background and blocking behavior.

- Modify `bitloops/src/capability_packs/semantic_clones/register.rs`
  - Mark the semantic clones current-state cursor as blocking init completion.
  - Update registration tests to assert the cursor policy.

- Modify `bitloops/src/capability_packs/architecture_graph/register.rs`
  - Keep `architecture_graph.snapshot` as a background cursor.
  - Register a new background cursor for architecture role classification.
  - Update registration tests to assert the snapshot and role cursors are background.

- Modify `bitloops/src/capability_packs/architecture_graph/types.rs`
  - Add a new role current-state consumer/mailbox constant.

- Add `bitloops/src/capability_packs/architecture_graph/roles/current_state_consumer.rs`
  - Implement `ArchitectureGraphRoleCurrentStateConsumer`.
  - Load canonical files.
  - Run `classify_architecture_roles_for_current_state`.
  - Enqueue adjudication requests.
  - Return metrics and warnings.

- Modify `bitloops/src/capability_packs/architecture_graph/roles.rs`
  - Expose the new role current-state consumer module.

- Modify `bitloops/src/capability_packs/architecture_graph/current_state/consumer.rs`
  - Remove role classification and adjudication enqueueing from the graph snapshot consumer.
  - Keep graph snapshot generation and graph replacement only.

- Modify `bitloops/src/capability_packs/architecture_graph/current_state/tests.rs`
  - Move role-classification expectations to the new role consumer.
  - Add a graph snapshot test proving it no longer enqueues role adjudication jobs.

- Modify `bitloops/src/daemon/capability_events/coordinator/ingestion.rs`
  - Pass `init_session_id` only to cursor mailboxes whose init policy blocks init completion.

- Modify `bitloops/src/daemon/capability_events/coordinator/tests.rs`
  - Assert sync/current-state generation attaches `init_session_id` only to blocking cursors.

---

### Task 1: Add Cursor Init Policy to Mailbox Registration

**Files:**
- Modify: `bitloops/src/host/capability_host/registrar.rs`

- [x] **Step 1: Write the failing registration-policy test**

Add this test near the existing registration tests in `bitloops/src/host/capability_host/registrar.rs`:

```rust
#[test]
fn mailbox_registration_defaults_to_background_and_can_block_init() {
    let background = CapabilityMailboxRegistration::new(
        "capability",
        "capability.background",
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer("capability.background"),
    );

    assert_eq!(
        background.init_policy,
        CapabilityMailboxInitPolicy::Background
    );

    let blocking = CapabilityMailboxRegistration::new(
        "capability",
        "capability.blocking",
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer("capability.blocking"),
    )
    .blocks_init_completion();

    assert_eq!(
        blocking.init_policy,
        CapabilityMailboxInitPolicy::BlocksInitCompletion
    );
}
```

- [x] **Step 2: Run the focused failing test**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib mailbox_registration_defaults_to_background_and_can_block_init
```

Expected: FAIL because `CapabilityMailboxInitPolicy` and `init_policy` do not exist yet.

- [x] **Step 3: Implement the policy model**

In `bitloops/src/host/capability_host/registrar.rs`, add the policy enum next to the other mailbox policy enums:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMailboxInitPolicy {
    Background,
    BlocksInitCompletion,
}
```

Update `CapabilityMailboxRegistration`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityMailboxRegistration {
    pub capability_id: &'static str,
    pub mailbox_name: &'static str,
    pub policy: CapabilityMailboxPolicy,
    pub handler: CapabilityMailboxHandler,
    pub readiness_policy: CapabilityMailboxReadinessPolicy,
    pub backlog_policy: CapabilityMailboxBacklogPolicy,
    pub init_policy: CapabilityMailboxInitPolicy,
}
```

Update `CapabilityMailboxRegistration::new` so the default is background:

```rust
pub const fn new(
    capability_id: &'static str,
    mailbox_name: &'static str,
    policy: CapabilityMailboxPolicy,
    handler: CapabilityMailboxHandler,
) -> Self {
    Self {
        capability_id,
        mailbox_name,
        policy,
        handler,
        readiness_policy: CapabilityMailboxReadinessPolicy::None,
        backlog_policy: CapabilityMailboxBacklogPolicy::None,
        init_policy: CapabilityMailboxInitPolicy::Background,
    }
}
```

Add builder methods:

```rust
pub const fn init_policy(mut self, policy: CapabilityMailboxInitPolicy) -> Self {
    self.init_policy = policy;
    self
}

pub const fn blocks_init_completion(self) -> Self {
    self.init_policy(CapabilityMailboxInitPolicy::BlocksInitCompletion)
}
```

- [x] **Step 4: Run the focused test**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib mailbox_registration_defaults_to_background_and_can_block_init
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add bitloops/src/host/capability_host/registrar.rs
git commit -m "feat: add init policy to capability cursor mailboxes"
```

---

### Task 2: Mark Only Semantic Current-State as Init-Blocking

**Files:**
- Modify: `bitloops/src/capability_packs/semantic_clones/register.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/register.rs`

- [x] **Step 1: Write failing semantic registration assertions**

In `bitloops/src/capability_packs/semantic_clones/register.rs`, change the test `CollectingRegistrar` mailbox storage from tuples to full registrations:

```rust
mailboxes: Vec<CapabilityMailboxRegistration>,
```

Update its `register_mailbox` implementation:

```rust
fn register_mailbox(&mut self, registration: CapabilityMailboxRegistration) -> Result<()> {
    self.mailboxes.push(registration);
    Ok(())
}
```

Update the existing mailbox assertion to map registrations to tuples:

```rust
assert_eq!(
    registrar
        .mailboxes
        .iter()
        .map(|mailbox| (mailbox.capability_id, mailbox.mailbox_name))
        .collect::<Vec<_>>(),
    vec![
        (
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX
        ),
        (
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
        ),
        (
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
        ),
        (
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
        ),
        (
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        ),
        (
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
        )
    ]
);
```

Add this assertion after the mailbox list assertion:

```rust
let current_state_mailbox = registrar
    .mailboxes
    .iter()
    .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX)
    .expect("semantic clones current-state mailbox to be registered");
assert_eq!(
    current_state_mailbox.init_policy,
    CapabilityMailboxInitPolicy::BlocksInitCompletion
);
```

Import `CapabilityMailboxInitPolicy` in the test module.

- [x] **Step 2: Write failing architecture registration assertions**

In `bitloops/src/capability_packs/architecture_graph/register.rs`, add assertions to the existing registration test:

```rust
let snapshot_mailbox = registrar
    .mailboxes
    .iter()
    .find(|mailbox| mailbox.mailbox_name == ARCHITECTURE_GRAPH_CONSUMER_ID)
    .expect("architecture graph snapshot mailbox to be registered");
assert_eq!(
    snapshot_mailbox.init_policy,
    CapabilityMailboxInitPolicy::Background
);
```

Also import `CapabilityMailboxInitPolicy`.

- [x] **Step 3: Run the focused failing tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib register_semantic_clones_pack_registers_expected_contributions register_architecture_graph_pack_registers_expected_contributions
```

Expected: FAIL because the semantic cursor has not been marked blocking yet.

- [x] **Step 4: Mark semantic current-state as blocking**

In `bitloops/src/capability_packs/semantic_clones/register.rs`, update only the semantic current-state cursor registration:

```rust
registrar.register_mailbox(
    CapabilityMailboxRegistration::new(
        super::types::SEMANTIC_CLONES_CAPABILITY_ID,
        super::types::SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX,
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer(
            super::types::SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
        ),
    )
    .blocks_init_completion(),
)?;
```

Do not mark `architecture_graph.snapshot` as blocking. It should keep the default `Background` policy.

- [x] **Step 5: Run focused registration tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib register_semantic_clones_pack_registers_expected_contributions register_architecture_graph_pack_registers_expected_contributions
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add bitloops/src/capability_packs/semantic_clones/register.rs bitloops/src/capability_packs/architecture_graph/register.rs
git commit -m "fix: mark semantic current-state as init blocking"
```

---

### Task 3: Attach Init Sessions Only to Blocking Cursor Runs

**Files:**
- Modify: `bitloops/src/daemon/capability_events/coordinator/ingestion.rs`
- Modify: `bitloops/src/daemon/capability_events/coordinator/tests.rs`

- [x] **Step 1: Write the failing scheduling test**

In `bitloops/src/daemon/capability_events/coordinator/tests.rs`, import:

```rust
use crate::capability_packs::architecture_graph::types::ARCHITECTURE_GRAPH_CONSUMER_ID;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX,
};
```

Add this test after `record_sync_generation_schedules_consumers_for_successful_empty_sync`:

```rust
#[test]
fn record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes() {
    let temp = TempDir::new().expect("tempdir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    write_test_daemon_config(&repo_root);

    let cfg = test_cfg(&repo_root);
    let host = DevqlCapabilityHost::builtin(repo_root.clone(), cfg.repo.clone())
        .expect("build capability host");
    let store = test_runtime_store(&temp);
    let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());

    coordinator
        .record_sync_generation(
            &host,
            &cfg,
            &SyncSummary {
                success: true,
                mode: "auto".to_string(),
                active_branch: Some("main".to_string()),
                head_commit_sha: Some("abc123".to_string()),
                ..SyncSummary::default()
            },
            SyncGenerationInput {
                file_diff: SyncFileDiff::default(),
                artefact_diff: SyncArtefactDiff::default(),
                source_task_id: None,
                init_session_id: Some("init-session-1"),
            },
        )
        .expect("record sync generation");

    let rows = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT capability_id, mailbox_name, init_session_id
                 FROM capability_workplane_cursor_runs
                 WHERE repo_id = ?1
                 ORDER BY capability_id ASC, mailbox_name ASC",
            )?;
            let rows = stmt
                .query_map(params![cfg.repo.repo_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .expect("load cursor runs");

    let semantic = rows
        .iter()
        .find(|(capability_id, mailbox_name, _)| {
            capability_id == SEMANTIC_CLONES_CAPABILITY_ID
                && mailbox_name == SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX
        })
        .expect("semantic current-state run");
    assert_eq!(semantic.2.as_deref(), Some("init-session-1"));

    let architecture = rows
        .iter()
        .find(|(_, mailbox_name, _)| mailbox_name == ARCHITECTURE_GRAPH_CONSUMER_ID)
        .expect("architecture graph snapshot run");
    assert_eq!(architecture.2, None);
}
```

- [x] **Step 2: Run the focused failing test**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes
```

Expected: FAIL because all cursor runs currently receive the init session.

- [x] **Step 3: Apply the policy during scheduling**

In `bitloops/src/daemon/capability_events/coordinator/ingestion.rs`, import:

```rust
use crate::host::capability_host::CapabilityMailboxInitPolicy;
```

Before `ensure_consumer_run(...)`, compute the init session passed to the run:

```rust
let run_init_session_id =
    if registration.init_policy == CapabilityMailboxInitPolicy::BlocksInitCompletion {
        input.init_session_id
    } else {
        None
    };
```

Then pass `run_init_session_id` into `ConsumerRunRequest`:

```rust
init_session_id: run_init_session_id,
```

Do not filter out background cursors. They still need to run after sync; they just must not be associated with the init session.

- [x] **Step 4: Run scheduling tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib record_sync_generation_schedules_consumers_for_successful_empty_sync record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add bitloops/src/daemon/capability_events/coordinator/ingestion.rs bitloops/src/daemon/capability_events/coordinator/tests.rs
git commit -m "fix: scope init sessions to blocking cursor runs"
```

---

### Task 4: Split Architecture Role Classification into a Background Cursor Consumer

**Files:**
- Create: `bitloops/src/capability_packs/architecture_graph/roles/current_state_consumer.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/roles.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/types.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/register.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/current_state/consumer.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/current_state/tests.rs`

- [x] **Step 1: Write failing registration expectations for the new role cursor**

In `bitloops/src/capability_packs/architecture_graph/types.rs`, the test will need this constant:

```rust
pub const ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID: &str =
    "architecture_graph.roles.current_state";
```

In `bitloops/src/capability_packs/architecture_graph/register.rs`, extend the registration test imports with:

```rust
ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
```

Update the expected `current_state_consumers` assertion:

```rust
assert_eq!(
    registrar.current_state_consumers,
    vec![
        (
            ARCHITECTURE_GRAPH_CAPABILITY_ID,
            ARCHITECTURE_GRAPH_CONSUMER_ID
        ),
        (
            ARCHITECTURE_GRAPH_CAPABILITY_ID,
            ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
        ),
    ]
);
```

Update the expected mailbox list to include the role current-state cursor before the adjudication job mailbox:

```rust
(
    ARCHITECTURE_GRAPH_CAPABILITY_ID,
    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
),
```

Add an assertion that the role cursor is background:

```rust
let role_current_state_mailbox = registrar
    .mailboxes
    .iter()
    .find(|mailbox| mailbox.mailbox_name == ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID)
    .expect("architecture role current-state mailbox to be registered");
assert_eq!(
    role_current_state_mailbox.init_policy,
    CapabilityMailboxInitPolicy::Background
);
```

- [x] **Step 2: Run the focused failing registration test**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib register_architecture_graph_pack_registers_expected_contributions
```

Expected: FAIL because the new constant/consumer/mailbox do not exist yet.

- [x] **Step 3: Register the new background role current-state cursor**

In `bitloops/src/capability_packs/architecture_graph/types.rs`, add:

```rust
pub const ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID: &str =
    "architecture_graph.roles.current_state";
```

In `bitloops/src/capability_packs/architecture_graph/register.rs`, import the new consumer:

```rust
use super::roles::ArchitectureGraphRoleCurrentStateConsumer;
```

Import the new constant:

```rust
ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
```

Register the new cursor after the snapshot cursor:

```rust
registrar.register_mailbox(CapabilityMailboxRegistration::new(
    ARCHITECTURE_GRAPH_CAPABILITY_ID,
    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
    CapabilityMailboxPolicy::Cursor,
    CapabilityMailboxHandler::CurrentStateConsumer(
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
    ),
))?;
```

Register the new consumer:

```rust
registrar.register_current_state_consumer(CurrentStateConsumerRegistration::new(
    ARCHITECTURE_GRAPH_CAPABILITY_ID,
    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
    Arc::new(ArchitectureGraphRoleCurrentStateConsumer),
))?;
```

Do not call `.blocks_init_completion()` for this cursor.

- [x] **Step 4: Add the role current-state consumer implementation**

Create `bitloops/src/capability_packs/architecture_graph/roles/current_state_consumer.rs`:

```rust
use anyhow::Context;
use serde_json::json;

use crate::capability_packs::architecture_graph::roles::{
    RoleAdjudicationEnqueueMetrics, default_queue_store, enqueue_adjudication_requests,
};
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};
use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};

use super::classifier::{
    ArchitectureRoleClassificationInput, classify_architecture_roles_for_current_state,
    role_classification_scope_from_request,
};
use super::fact_extraction::RelationalArchitectureRoleCurrentStateSource;

pub struct ArchitectureGraphRoleCurrentStateConsumer;

impl CurrentStateConsumer for ArchitectureGraphRoleCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let files = context
                .relational
                .load_current_canonical_files(&request.repo_id)
                .context("loading current files for architecture role classification")?;
            let role_current_state = RelationalArchitectureRoleCurrentStateSource::new(
                &request.repo_id,
                context.relational.as_ref(),
            );
            let outcome = classify_architecture_roles_for_current_state(
                context.storage.as_ref(),
                &role_current_state,
                ArchitectureRoleClassificationInput {
                    repo_id: &request.repo_id,
                    generation_seq: request.to_generation_seq_inclusive,
                    scope: role_classification_scope_from_request(request),
                    files: &files,
                },
            )
            .await
            .context("classifying architecture roles for current state")?;

            let role_metrics = serde_json::to_value(&outcome.metrics)
                .unwrap_or_else(|_| json!({ "serialization_error": true }));
            let adjudication_metrics = enqueue_adjudication_requests(
                &outcome.adjudication_requests,
                context.workplane.as_ref(),
                default_queue_store().as_ref(),
            )
            .context("enqueueing architecture role adjudication requests")?;

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings: outcome.warnings,
                metrics: Some(role_current_state_metrics(
                    role_metrics,
                    &adjudication_metrics,
                )),
            })
        })
    }
}

fn role_current_state_metrics(
    role_metrics: serde_json::Value,
    adjudication_metrics: &RoleAdjudicationEnqueueMetrics,
) -> serde_json::Value {
    json!({
        "roles": role_metrics,
        "role_adjudication_selected": adjudication_metrics.selected,
        "role_adjudication_enqueued": adjudication_metrics.enqueued,
        "role_adjudication_deduped": adjudication_metrics.deduped,
    })
}
```

- [x] **Step 5: Expose the new module**

In `bitloops/src/capability_packs/architecture_graph/roles.rs`, add:

```rust
pub mod current_state_consumer;
```

Export the consumer:

```rust
pub use current_state_consumer::ArchitectureGraphRoleCurrentStateConsumer;
```

Update the facade test module list to include `"current_state_consumer"` and update the expected length from `14` to `15`.

- [x] **Step 6: Remove role classification from the graph snapshot consumer**

In `bitloops/src/capability_packs/architecture_graph/current_state/consumer.rs`, remove these imports:

```rust
use crate::capability_packs::architecture_graph::roles::{
    RoleAdjudicationEnqueueMetrics, default_queue_store, enqueue_adjudication_requests,
};
```

Remove the block that:

```rust
let mut role_metrics = serde_json::Value::Null;
let mut adjudication_requests = Vec::new();
...
classify_architecture_roles_for_current_state(...)
```

Remove `"roles": role_metrics` from graph snapshot metrics.

Remove the block that calls:

```rust
enqueue_adjudication_requests(...)
```

Return graph snapshot metrics directly:

```rust
Ok(CurrentStateConsumerResult {
    applied_to_generation_seq: request.to_generation_seq_inclusive,
    warnings,
    metrics: Some(metrics),
})
```

- [x] **Step 7: Add/update current-state consumer tests**

In `bitloops/src/capability_packs/architecture_graph/current_state/tests.rs`, update role-related tests so they call `ArchitectureGraphRoleCurrentStateConsumer` instead of `ArchitectureGraphCurrentStateConsumer`.

For tests that currently assert role metrics from graph reconcile, use:

```rust
let result = ArchitectureGraphRoleCurrentStateConsumer
    .reconcile(&request, &test.context)
    .await?;
```

For graph snapshot behavior, add this test:

```rust
#[tokio::test]
async fn graph_snapshot_reconcile_does_not_enqueue_role_adjudication_jobs() -> anyhow::Result<()> {
    let repo_id = "repo-graph-snapshot-no-role-jobs";
    let test = architecture_consumer_test_context(repo_id).await?;
    insert_current_file(&test.sqlite_path, repo_id, "src/api.rs", "rust")?;
    upsert_test_role(test.storage.as_ref(), repo_id, "role-api-low-review", "api").await?;
    upsert_path_suffix_rule(
        test.storage.as_ref(),
        repo_id,
        "role-api-low-review",
        "rule-api-low-review",
        "api.rs",
        0.6,
    )
    .await?;

    let request = CurrentStateConsumerRequest {
        run_id: Some("run".to_string()),
        repo_id: repo_id.to_string(),
        repo_root: test._temp.path().to_path_buf(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 29,
        reconcile_mode: crate::host::capability_host::ReconcileMode::MergedDelta,
        file_upserts: Vec::new(),
        file_removals: Vec::new(),
        affected_paths: vec!["src/api.rs".to_string()],
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };

    let result = ArchitectureGraphCurrentStateConsumer
        .reconcile(&request, &test.context)
        .await?;

    assert!(test.workplane.jobs().is_empty());
    assert!(
        result
            .metrics
            .as_ref()
            .and_then(|metrics| metrics.get("roles"))
            .is_none(),
        "graph snapshot metrics should not include role classification metrics"
    );
    Ok(())
}
```

Add imports:

```rust
use crate::capability_packs::architecture_graph::roles::ArchitectureGraphRoleCurrentStateConsumer;
```

- [x] **Step 8: Run focused architecture tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib register_architecture_graph_pack_registers_expected_contributions graph_snapshot_reconcile_does_not_enqueue_role_adjudication_jobs current_state_reconcile_enqueues_low_confidence_role_adjudication_job
```

Expected: PASS.

- [x] **Step 9: Commit**

```bash
git add \
  bitloops/src/capability_packs/architecture_graph/types.rs \
  bitloops/src/capability_packs/architecture_graph/register.rs \
  bitloops/src/capability_packs/architecture_graph/roles.rs \
  bitloops/src/capability_packs/architecture_graph/roles/current_state_consumer.rs \
  bitloops/src/capability_packs/architecture_graph/current_state/consumer.rs \
  bitloops/src/capability_packs/architecture_graph/current_state/tests.rs
git commit -m "fix: run architecture role classification as background current-state work"
```

---

### Task 5: Prove Init Runtime Is Unblocked by Background Architecture Cursors

**Files:**
- Modify: `bitloops/src/daemon/capability_events/coordinator/tests.rs`
- Modify: `bitloops/src/daemon/init_runtime/tests.rs`

- [x] **Step 1: Add a current-state scheduling regression assertion**

Extend `record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes` with this assertion after loading rows:

```rust
assert!(
    rows.iter()
        .any(|(_, mailbox_name, init_session_id)| {
            mailbox_name == ARCHITECTURE_GRAPH_CONSUMER_ID && init_session_id.is_none()
        }),
    "architecture graph snapshot must be scheduled as background current-state work"
);
```

Add a similar assertion for the new role current-state cursor:

```rust
assert!(
    rows.iter()
        .any(|(_, mailbox_name, init_session_id)| {
            mailbox_name == ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
                && init_session_id.is_none()
        }),
    "architecture role current-state work must be scheduled as background work"
);
```

Import:

```rust
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CONSUMER_ID, ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};
```

- [x] **Step 2: Add an init-lane regression test**

In `bitloops/src/daemon/init_runtime/tests.rs`, add a focused lane-level test near `code_embeddings_lane_waits_for_codebase_updates_after_sync_task_completion`:

```rust
#[test]
fn code_embeddings_lane_completes_when_no_init_blocking_current_state_remains() {
    let session = embeddings_only_session();
    let initial_sync = completed_sync_task("sync-task-1", 10);
    let stats = SessionWorkplaneStats::default();

    let lane = derive_code_embeddings_lane(
        &session,
        Some(&initial_sync),
        None,
        None,
        StatusCounts::default(),
        &stats,
        Some(InitRuntimeLaneProgressView {
            completed: 2243,
            in_memory_completed: 0,
            total: 2243,
            remaining: 0,
        }),
    );

    assert_eq!(lane.status, "completed");
    assert_eq!(lane.waiting_reason, None);
}
```

This test does not simulate background cursor rows directly. The scheduling test proves background cursors do not get the init session, and this lane test proves semantic completion depends only on the `current_state` aggregate passed to init.

- [x] **Step 3: Run focused init and scheduling tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes code_embeddings_lane_completes_when_no_init_blocking_current_state_remains
```

Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add bitloops/src/daemon/capability_events/coordinator/tests.rs bitloops/src/daemon/init_runtime/tests.rs
git commit -m "test: cover init-safe background current-state cursors"
```

---

### Task 6: Verify Role Current-State Flow Matches the Expected Sync Pipeline

**Files:**
- Modify: `bitloops/src/capability_packs/architecture_graph/current_state/tests.rs`
- Modify: `bitloops/src/capability_packs/architecture_graph/roles/classifier/tests.rs` only if existing coverage is missing.

- [x] **Step 1: Add or verify test coverage for affected-path classification**

Confirm these existing tests still pass:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib artefact_removals_affect_but_do_not_remove_role_paths full_reconcile_uses_all_live_role_paths
```

Expected: PASS.

- [x] **Step 2: Add a role current-state delta test**

Add this test in `bitloops/src/capability_packs/architecture_graph/current_state/tests.rs`:

```rust
#[tokio::test]
async fn role_current_state_reconcile_uses_affected_paths_from_sync_delta() -> anyhow::Result<()> {
    let repo_id = "repo-role-current-state-delta";
    let test = architecture_consumer_test_context(repo_id).await?;
    insert_current_file(&test.sqlite_path, repo_id, "src/api.rs", "rust")?;
    insert_current_file(&test.sqlite_path, repo_id, "src/worker.rs", "rust")?;
    upsert_test_role(test.storage.as_ref(), repo_id, "role-api", "api").await?;
    upsert_path_suffix_rule(
        test.storage.as_ref(),
        repo_id,
        "role-api",
        "rule-api",
        "api.rs",
        0.95,
    )
    .await?;

    let request = CurrentStateConsumerRequest {
        run_id: Some("run".to_string()),
        repo_id: repo_id.to_string(),
        repo_root: test._temp.path().to_path_buf(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 10,
        to_generation_seq_inclusive: 11,
        reconcile_mode: crate::host::capability_host::ReconcileMode::MergedDelta,
        file_upserts: vec![crate::host::capability_host::ChangedFile {
            path: "src/api.rs".to_string(),
            language: "rust".to_string(),
            content_id: "content-api".to_string(),
        }],
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };

    let result = ArchitectureGraphRoleCurrentStateConsumer
        .reconcile(&request, &test.context)
        .await?;

    assert_eq!(
        result
            .metrics
            .as_ref()
            .and_then(|metrics| metrics.pointer("/roles/affected_paths"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        result
            .metrics
            .as_ref()
            .and_then(|metrics| metrics.pointer("/roles/full_reconcile"))
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    Ok(())
}
```

- [x] **Step 3: Run role current-state tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib role_current_state_reconcile_uses_affected_paths_from_sync_delta current_state_reconcile_enqueues_low_confidence_role_adjudication_job
```

Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add bitloops/src/capability_packs/architecture_graph/current_state/tests.rs bitloops/src/capability_packs/architecture_graph/roles/classifier/tests.rs
git commit -m "test: verify architecture role current-state delta flow"
```

---

### Task 7: Targeted Verification

**Files:**
- No source changes.

- [x] **Step 1: Run capability-host registration and scheduler tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib \
  mailbox_registration_defaults_to_background_and_can_block_init \
  register_semantic_clones_pack_registers_expected_contributions \
  register_architecture_graph_pack_registers_expected_contributions \
  record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes
```

Expected: PASS.

- [x] **Step 2: Run architecture graph and role focused tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib \
  graph_snapshot_reconcile_does_not_enqueue_role_adjudication_jobs \
  role_current_state_reconcile_uses_affected_paths_from_sync_delta \
  current_state_reconcile_enqueues_low_confidence_role_adjudication_job \
  artefact_removals_affect_but_do_not_remove_role_paths \
  full_reconcile_uses_all_live_role_paths
```

Expected: PASS.

- [x] **Step 3: Run init runtime focused tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features -p bitloops --lib \
  code_embeddings_lane_waits_for_codebase_updates_after_sync_task_completion \
  code_embeddings_lane_completes_when_no_init_blocking_current_state_remains \
  selected_session_workplane_stats_ignore_unselected_semantic_lanes \
  selected_session_workplane_stats_only_include_requested_embedding_lanes
```

Expected: PASS.

- [x] **Step 4: Run compile check**

Run:

```bash
cargo dev-check
```

Expected: PASS.

- [x] **Step 5: Record verification status**

No source changes are expected in this step. If any command fails, return to the task that owns the failed behavior and fix it there before rerunning the focused command.

Record the exact commands and pass/fail results in the final handoff message.

---

## Expected Final Behavior

After implementation:

```text
devql sync / watcher detects changes
  -> canonical files, artefacts, and edges update
  -> current-state generation advances
  -> semantic current-state cursor runs with init_session_id when sync belongs to init
  -> architecture graph snapshot cursor runs without init_session_id
  -> architecture role current-state cursor runs without init_session_id
  -> role facts are re-extracted for affected artefacts/paths
  -> deterministic role rules are re-run for refreshed paths
  -> role assignments are updated or marked stale/needs_review
  -> assignment history records meaningful changes
  -> ambiguous/high-impact cases enqueue LLM adjudication jobs without init_session_id
  -> init completion waits only for semantic current-state plus selected semantic jobs
```

## Self-Review

- Spec coverage: The plan handles init not being blocked by architecture work, preserves sync/current-state driven role classification, keeps affected-path incremental behavior, updates assignment history through existing classifier storage, and leaves LLM adjudication as queued background work.
- Placeholder scan: No placeholders are intentionally left. Each code-changing task has specific files, test names, code snippets, commands, and expected outcomes.
- Type consistency: The plan uses `CapabilityMailboxInitPolicy`, `CapabilityMailboxRegistration::blocks_init_completion`, `ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID`, and `ArchitectureGraphRoleCurrentStateConsumer` consistently across registration, scheduling, and tests.
