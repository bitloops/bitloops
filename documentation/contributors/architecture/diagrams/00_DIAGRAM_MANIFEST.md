# Bitloops architecture diagram manifest

This manifest is the maintenance contract for architecture diagrams in this
directory. It is meant for humans and automation agents that need to create,
update, or validate the diagram pack without collapsing the system into one
oversized drawing.

## Purpose

The diagram pack should explain the Bitloops runtime through a small set of
static and dynamic views:

- Static views answer "what exists" and "where are the boundaries?"
- Dynamic views answer "what triggers what" and "what side effects follow?"
- Narrative architecture docs and ADRs explain "why this is the shape."

Use C4-style diagrams as the static backbone, Mermaid sequence diagrams for
runtime flows, and separate narrative docs or ADRs for tradeoffs and decisions.

## Target diagram pack

Keep the durable diagram pack intentionally small:

| Role | Expected file(s) | Rule |
| --- | --- | --- |
| C4 context | [01-system-context.md](01-system-context.md) | Always keep. This is the highest-level product/system boundary view. |
| C4 container | [02-container-view.md](02-container-view.md) | Always keep. This is the main runtime/storage boundary view. |
| Critical runtime flows | [05-capture-lifecycle.md](05-capture-lifecycle.md), [06-sync-materialization.md](06-sync-materialization.md), planned `07-init-bootstrap-flow.md`, planned `08-watcher-change-flow.md` | Keep only flows that explain important product or correctness behavior. Do not add sequence diagrams for every internal lifecycle. |
| Data ownership | planned `09-data-ownership-and-producers.md` | Add because producer ownership and current-state convergence are central to Bitloops correctness. |
| Deployment/infrastructure | None today | Do not create a cloud-style deployment diagram unless Bitloops gains meaningful cloud/network/region topology. The current local runtime topology belongs in the container view. |

## Maintenance rules

- Keep one abstraction level per diagram.
- Prefer several focused diagrams over one mega diagram.
- Keep diagrams as code in Markdown using Mermaid unless a stronger repo-local
  convention replaces it.
- Every diagram must include:
  - a title
  - a short purpose statement
  - a "Use this when..." sentence
  - the Mermaid diagram
  - notes that call out modeling choices or intentional omissions
  - a glossary with beginner-friendly explanations for specialized terms in
    the diagram
- Do not mix C4 container, component, deployment, and sequence concerns in one
  diagram.
- Do not treat C4 as the whole architecture artifact. Link to narrative docs
  and ADRs for decisions, tradeoffs, and deeper explanation.
- Do not rely on Mermaid hover tooltips as the only explanation. Mermaid
  flowchart tooltips require interactive rendering and may be disabled by strict
  Markdown renderers, so the glossary is the portable source of truth.
- Do not create a diagram just because a subsystem exists. Create or keep it
  only when it answers a recurring architecture question better than prose.
- Do not keep standalone component inventory diagrams by default. Prefer
  sequence diagrams for behavior and narrative docs for internal structure.
- Before updating a diagram, inspect the source files and companion docs listed
  in this manifest.
- When adding, renaming, or removing a diagram, update
  [README.md](README.md) and this manifest in the same change.
- Retire or merge noisy diagrams instead of adding more nodes to make them
  complete.

## Current diagram inventory

| File | Type | Question answered | Status | Primary sources |
| --- | --- | --- | --- | --- |
| [01-system-context.md](01-system-context.md) | C4-style context | Who and what interacts with the Bitloops local system? | Core | `bitloops/src/cli.rs`, `documentation/contributors/architecture/layered-extension-architecture.md`, `documentation/contributors/architecture/host-substrate.md` |
| [02-container-view.md](02-container-view.md) | C4-style container | What runtime and storage boundaries exist around the CLI, daemon, watcher, hooks, dashboard, and stores? | Core; update when runtime boundaries change | `bitloops/src/cli.rs`, `bitloops/src/daemon/server_runtime.rs`, `bitloops/src/host/devql/watch.rs`, `documentation/contributors/architecture/host-substrate.md` |
| [05-capture-lifecycle.md](05-capture-lifecycle.md) | Sequence | What happens when agent and Git hooks capture work, checkpoints, and provenance? | Critical flow | `bitloops/src/host/hooks`, `bitloops/src/host/checkpoints`, `bitloops/src/host/devql/capture.rs` |
| [06-sync-materialization.md](06-sync-materialization.md) | Sequence | How does repo state become current-state DevQL data across watcher, manual, and capture-triggered producers? | Critical flow; refine after producer changes | `bitloops/src/host/devql/commands_sync`, `bitloops/src/host/devql/producer_spool`, `bitloops/src/daemon/tasks`, `documentation/contributors/architecture/devql-sync-producer-ownership.md` |

## Planned diagrams

These diagrams should be created next unless implementation changes make them
obsolete.

| Proposed file | Type | Question answered | Required content | Primary sources |
| --- | --- | --- | --- | --- |
| `07-init-bootstrap-flow.md` | Sequence plus flag matrix | What happens when a user runs `bitloops init`, and how do flags alter the path? | CLI entry, repo discovery, daemon bootstrap, telemetry handling, agent selection, repo policy writes, hook/surface install, embeddings/summaries/context-guidance setup, final sync/ingest choices, watcher reconcile, runtime `startInit`, and init-session task lanes. Put flags in a matrix below the sequence instead of making the sequence diagram a flag encyclopedia. | `bitloops/src/cli/init/args.rs`, `bitloops/src/cli/init/workflow.rs`, `bitloops/src/cli/init/daemon_bootstrap.rs`, `bitloops/src/cli/init/final_setup.rs`, `bitloops/src/cli/init/embeddings_setup.rs`, `bitloops/src/cli/init/summary_setup.rs`, `bitloops/src/cli/init/context_guidance_setup.rs`, `bitloops/src/cli/watcher_bootstrap.rs`, `bitloops/src/api/runtime_schema/start_init.rs`, `bitloops/src/daemon/init_runtime` |
| `08-watcher-change-flow.md` | Sequence | After init, what happens when a repo file is added, changed, deleted, renamed, or reset? | Watcher startup assumptions, notify event, ignored path filtering, gitignored filtering, debounce batching, git index lock deferral, dirty worktree rescan fallback, branch checkout promotion, temporary tree hash, path classification/exclusion, producer spool enqueue, daemon claim, visible sync task, current-state materialization, workspace revision persistence, and retry/no-op behavior. | `bitloops/src/host/devql/watch.rs`, `bitloops/src/host/devql/capture.rs`, `bitloops/src/host/devql/producer_spool`, `bitloops/src/daemon/tasks`, `bitloops/src/host/devql/commands_sync`, `documentation/contributors/architecture/devql-sync-producer-ownership.md` |
| `09-data-ownership-and-producers.md` | Data ownership / producer map | Which producer owns which DevQL data, stores, schemas, and convergence contract? | Repo-local runtime state, producer spool, current-state relational data, historical/event/blob state, workplane/enrichment state, and producer responsibilities for init, watcher, hooks, daemon startup, manual repair, and validation. | `documentation/contributors/architecture/devql-sync-producer-ownership.md`, `documentation/contributors/architecture/devql-core-pack-boundaries.md`, `bitloops/src/host/devql`, `bitloops/src/daemon`, `bitloops/src/capability_packs` |

## Do not create as standalone diagrams

These topics are useful, but too narrow or too operational to deserve separate
diagrams by default:

| Topic | Where it belongs instead |
| --- | --- |
| Standalone capture or daemon component inventories | Capture behavior belongs in [05-capture-lifecycle.md](05-capture-lifecycle.md). Daemon behavior belongs in [06-sync-materialization.md](06-sync-materialization.md), planned `07-init-bootstrap-flow.md`, planned `08-watcher-change-flow.md`, planned `09-data-ownership-and-producers.md`, or companion narrative docs. |
| Daemon startup watcher rehydration | Cover briefly in [02-container-view.md](02-container-view.md), `08-watcher-change-flow.md`, or [09-data-ownership-and-producers.md](09-data-ownership-and-producers.md) if needed. Create a standalone sequence only if this lifecycle becomes a repeated source of bugs or design decisions. |
| Exhaustive `bitloops init` flag permutations | Put the flag effects in the `07-init-bootstrap-flow.md` matrix. Keep the sequence diagram focused on the main branch points. |
| Internal task-queue lane mechanics | Keep in narrative daemon docs unless the lane behavior is the architecture question. If it becomes architecture-relevant, fold it into a critical sequence or data ownership diagram. |
| Generated module dependency graphs | Generate on demand outside this curated diagram pack. |

## Retirement criteria

Remove or merge a diagram when any of these are true:

- It repeats another diagram at the same abstraction level.
- Its main value is a complete inventory of internals rather than a clear
  architecture question.
- It needs frequent churn for implementation details that do not change
  architecture.
- It is better answered by an ADR, narrative doc, CLI help, or source comments.
- Its Mermaid block becomes hard to scan even after splitting notes and tables
  out of the diagram.

Before deleting a diagram, check for links to it with `rg` and update the README,
this manifest, and any companion narrative docs.

## Automation instructions

When an automation agent is asked to update the architecture diagrams:

1. Read this manifest and [README.md](README.md).
2. Read the target diagram file if it exists.
3. Read the listed primary sources for the diagram being created or changed.
4. Check companion architecture docs and ADRs that are directly linked from the
   target diagram or this manifest.
5. Update only diagrams whose answered question changed.
6. Keep existing file naming and numbering unless the user explicitly asks for a
   reorganization.
7. If a new diagram is added, add it to the README reading order and to this
   manifest.
8. If a source file reveals that the diagram is stale, update the notes as well
   as the Mermaid block.
9. Add or update the diagram glossary whenever terms are introduced, renamed, or
   removed.
10. For sequence diagrams with many branches, keep the sequence readable and move
   detailed flag or policy combinations into tables below the diagram.
11. Run a Markdown/Mermaid sanity check if the repo provides one. If there is no
    diagram check, at least inspect the edited Markdown for unmatched code
    fences and Mermaid syntax mistakes.

## Update triggers

Update the diagram pack when any of these change:

- `bitloops init` flags, prompts, defaults, setup lanes, or runtime
  orchestration.
- Daemon lifecycle, supervisor behavior, watcher rehydration, or runtime state
  paths.
- DevQL producer ownership across init, watcher, hooks, manual sync, validation,
  or repair.
- Producer spool payloads, admission rules, coalescing behavior, or daemon task
  lane ownership.
- Storage boundaries for repo-local state, relational current state, event
  history, blob storage, or capability workplane state.
- Agent hook surfaces or Git hook responsibilities.
- Capability-pack or language-pack boundaries that change what owns parsing,
  extraction, enrichment, or query behavior.

## Out of scope

The diagram pack is not a replacement for:

- detailed implementation docs
- ADRs
- API reference docs
- exhaustive CLI help
- generated Rust module dependency graphs

If a diagram starts needing those details, split the view or link to the
appropriate narrative document instead.
