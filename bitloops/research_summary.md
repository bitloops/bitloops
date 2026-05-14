# Summary-Embedding Overlap for Semantic Backfills

## Summary
- Make per-chunk summary-to-summary-embedding handoff the canonical backfill behavior: once a summary batch is successfully persisted, enqueue summary-embedding work for exactly that completed chunk immediately.
- Treat this as both a handoff fix and a scheduling fix. Earlier enqueue alone is not enough because code and summary embeddings currently share the same embeddings pool.

## Implementation Changes
- In `bitloops/src/daemon/enrichment/execution/mailbox_summary.rs`, `bitloops/src/daemon/enrichment/execution/workplane_plan.rs`, and `bitloops/src/daemon/enrichment/semantic_writer/commit.rs`, standardize the chunk contract so the processed summary chunk emits summary-embedding follow-ups only for its own artefact ids, any unprocessed remainder is re-enqueued as the next summary-backfill chunk, and summary persistence plus embedding follow-up upserts remain inside the same successful commit boundary.
- In `bitloops/src/daemon/enrichment/workplane/mailbox_claim.rs`, change embedding-batch selection so the embeddings pool first claims ready `summary` representation batches for repos that still have active summary-refresh work pending or running, whether that summary work is in semantic summary mailbox rows or summary-refresh workplane jobs. If no such candidate exists, keep the current FIFO claim behavior.
- In `bitloops/src/daemon/init_runtime/lanes.rs` and `bitloops/src/daemon/init_runtime/workplane.rs`, update reporting so summary embeddings can show `queued` or `running` while summary refresh is still active, instead of always collapsing to “Waiting for summaries to be ready” whenever any summary work remains.

## Public APIs / Types
- No CLI flags, config schema, or external API behavior changes.
- Internal only: add repo-level claim-priority metadata or helper logic for “summary overlap active” so the shared embeddings pool can temporarily bias summary-embedding claims during an active summary backfill.

## Test Plan
- Extend `bitloops/src/daemon/enrichment_tests.rs` to prove that a completed summary backfill chunk immediately produces claimable summary-embedding work for the same artefacts while the remaining summary backfill stays queued.
- Add mailbox-claim tests that place older code-embedding work ahead of newer summary-embedding work and verify the embeddings pool still claims the ready summary-embedding batch first when summary refresh is active, but falls back to current ordering when it is not.
- Add init-runtime lane tests in `bitloops/src/daemon/init_runtime/tests.rs` for mixed states where summary refresh is still pending or running and summary-embedding jobs are already queued or running, and verify the lane/reporting surfaces the overlap correctly.
- Keep existing retry, dedupe, and clone-rebuild regression coverage intact.

## Assumptions
- “Immediate enqueue” means “inside the same successful batch-completion path,” not a later end-of-run sweep.
- The priority change is temporary and repo-scoped: it only applies while that repo still has outstanding summary-refresh work.
- Code and identity embeddings remain in the shared embeddings pool; this plan changes claim order rather than introducing a new worker pool or new configuration.

## TODOs
- [x] Confirm which summary execution path is active for semantic summary backfills in the target scenario: mailbox summary batches, workplane summary-refresh jobs, or both.
- [x] Update summary batch completion logic so each persisted summary chunk enqueues summary-embedding follow-ups for only the artefacts in that completed chunk.
- [x] Keep summary persistence and summary-embedding follow-up enqueue inside the same successful commit boundary.
- [x] Preserve remainder handling so any unprocessed summary-backfill artefacts are re-enqueued as the next summary chunk instead of being dropped or delayed to an end-of-run sweep.
- [x] Add embeddings-pool claim logic that prefers ready summary-embedding batches while the same repo still has active summary-refresh work.
- [x] Ensure the priority rule is temporary and repo-scoped, and falls back to normal FIFO behavior once summary refresh is no longer active.
- [x] Update init-runtime lane/reporting logic so summary embeddings can appear as `queued` or `running` while summary refresh is still in progress.
- [x] Add regression coverage proving a completed summary backfill chunk immediately creates claimable summary-embedding work while remaining summary chunks stay queued.
- [x] Add regression coverage proving summary-embedding batches can run ahead of unrelated older code-embedding backlog during active summary backfill.
- [x] Add regression coverage proving embeddings-pool ordering falls back to current behavior when no summary refresh is active.
- [x] Add init-runtime lane tests covering mixed states where summary work is still pending or running and summary-embedding jobs are already queued or running.
- [x] Re-run targeted enrichment and init-runtime tests after implementation.
