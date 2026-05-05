# DevQL sync producer ownership

## Status

Accepted for the init-only background sync contract.

## Product contract

After a user runs `bitloops init`, normal repository activity should keep DevQL current-state data converged without requiring the user to run additional Bitloops commands.

The expected steady-state lifecycle is:

1. `bitloops init` binds the repo, installs the default daemon setup, installs managed Git hooks, runs the initial sync work, and records enough repo-local state for the daemon to know the repo exists.
2. Daemon startup rehydrates watchers for initialized repo bindings.
3. The watcher tracks ordinary working-tree file edits while the daemon is alive.
4. Git hooks queue semantic safety work for Git lifecycle transitions.
5. `sync --validate` is the drift oracle after any flow that should have converged.

DevQL query commands are read-only from a producer-lifecycle perspective. Running `bitloops devql ...` must not be required to start or repair sync producers.

## Producer ownership

| Producer | Ownership | Expected behavior |
| --- | --- | --- |
| `init` | Bootstrap producer | Establishes repo binding, daemon installation, managed hooks, initial current-state materialization, and watcher eligibility. |
| Daemon startup | Watcher lifecycle owner | Rehydrates watchers for initialized repo bindings. This is the normal way a watcher should come back after daemon restart. |
| Watcher | Primary worktree edit producer | Queues path sync work for file add, change, delete, rename, and reset effects observed through filesystem events. |
| `post-checkout` hook | Branch transition safety producer | Queues full sync work for branch checkouts. It covers semantic HEAD changes and final branch state even when filesystem events are incomplete or already handled. |
| `post-merge` hook | Merge/pull safety producer | Queues changed-path refresh/backfill work for merge or pull results. It may complete as `unchanged` when checkout or watcher work already materialized the result. |
| `post-commit` hook | Commit-scoped safety producer | Queues committed-path semantic refresh work. It may complete as `changed` or `unchanged` when watcher work already indexed the edited files before commit. |
| Manual `sync --repair` | Explicit recovery producer | Repairs known drift when validation or manual QA proves state is stale. |
| Manual `sync --validate` | Read-only validator | Checks for drift. It should not mutate current-state data. |

## Hook and watcher overlap

Hook and watcher overlap is allowed. The correctness contract is convergence, not strict causality between the first producer that observed an event and the producer task that eventually reports work.

This means these outcomes are valid:

- A file edit is indexed by the watcher, then a later `post-commit` refresh completes with `unchanged` for the committed file.
- A branch checkout is materialized by `post-checkout`, then watcher path work completes with no additional semantic delta.
- A merge or pull is already materialized by checkout or watcher work, then `post-merge` completes with `unchanged`.
- A hard reset is observed by either watcher path work or a checkout/full sync task first, then the other producer becomes a no-op.

These outcomes are not valid:

- Final current-state data remains stale after producers drain.
- `sync --validate` reports drift after an eventually consistent wait window.
- A hook failure is required for normal convergence.
- A DevQL query command is required to start, restart, or repair a watcher.
- Hook overlap produces user-visible Git failures or persistent noisy warnings.

## Why hooks remain installed

The watcher is the primary producer for file edits, but it should not be the only producer for Git semantics.

Git operations can change repository meaning in ways that are not cleanly represented by reliable path events alone:

- branch checkout changes the active HEAD and can replace large portions of the tree
- merge and pull have commit graph semantics in addition to file changes
- commit has a stable commit SHA and committed path set that is useful for semantic refresh and checkpoint-related follow-up

For that reason, managed Git hooks remain installed as safety producers. They are intentionally tolerant of already-converged state and should not block Git. A hook that becomes mostly a no-op after watcher work is acceptable when final state is correct and validation is clean.

## QAT assertion policy

QAT should assert producer causality only where the contract requires that producer to own the observed transition.

Use strict summary-field assertions when a scenario isolates a producer:

- watcher-only add/change/remove flows can assert watcher `added`, `changed`, or `removed`
- checkout scenarios that isolate `post-checkout` can assert a `post_checkout` full task with useful work

Use eventual-state assertions for race-prone overlap flows:

- `post_commit` and `post_merge` should prove their producer task ran and did work or observed already-converged paths
- reset and checkout flows should prove the expected final files are materialized
- every flow should end with `sync --validate` reporting clean state

When a scenario depends on overlap, QAT should capture diagnostics for the completed producer task records so failures show which producer won the race.

## QAT lanes

The high-value DevQL Sync QAT lane is producer-contract coverage:

- `cargo qat-devql-sync-producer` runs the DevQL Sync feature filtered to `@sync_producer`.
- The producer alias excludes `@sync_known_gap` scenarios until the corresponding product fix lands.
- Develop-gate sync coverage is also producer-contract coverage. Any DevQL Sync scenario tagged `@develop_gate` must also be tagged `@sync_producer` and must not be tagged `@sync_known_gap`.
- `@sync_manual_smoke` marks the smaller manual sync smoke subset for explicit enqueue, validate, repair, and path-scoped behavior.
- `@sync_legacy` remains available for historical `init --sync=true` convergence behavior, but it is not the main correctness signal for the product contract.
- `@sync_known_gap` keeps target-contract scenarios visible when they currently reproduce known product gaps. Remove the tag from a scenario when its product fix lands so the producer lane starts enforcing it.

The producer lane supplies its filter in code, so `CUCUMBER_FILTER_TAGS` does not accidentally widen or narrow the passing producer contract run. Run `CUCUMBER_FILTER_TAGS='@sync_known_gap' cargo qat-devql-sync` when intentionally investigating those product gaps.

## Current implementation notes

- Watcher producer jobs are written to the repo-local producer spool and claimed by the daemon task coordinator.
- Git hook producer jobs are also written to the producer spool. Hook handlers warn and continue when DevQL follow-up cannot be queued or executed.
- `post-checkout` queues a full sync task with source `post_checkout`.
- `post-commit` and `post-merge` execute hook-specific refresh paths rather than ordinary watcher path tasks.
- The daemon claims at most one producer spool job per repo at a time, and queued compatible sync tasks can be coalesced before execution.

## Follow-up risks

- Multi-repo daemon watcher rehydration still needs manual and automated coverage beyond single-repo smoke tests.
- Manual QA should inspect normal Git flows for persistent hook warnings, repeated failed producer tasks, and validation drift.
- If future product requirements demand strict causality from a specific hook, the producer spool and QAT contract must change explicitly instead of tightening assertions around today's overlapping model.
