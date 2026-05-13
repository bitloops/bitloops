# Bitloops architecture diagrams

No single diagram captures the current runtime shape. This directory splits the system into a small set of static and dynamic views that match the code as it exists today.

## Maintenance manifest

[00_DIAGRAM_MANIFEST.md](00_DIAGRAM_MANIFEST.md) defines the diagram inventory, planned diagrams, source-of-truth files, and automation rules for keeping this directory current.

## Recommended reading order

1. [01-system-context.md](01-system-context.md)
2. [02-container-view.md](02-container-view.md)
3. [05-capture-lifecycle.md](05-capture-lifecycle.md)
4. [06-sync-materialization.md](06-sync-materialization.md)

## What each diagram is for

- `01-system-context.md`: C4-style system context. Shows Bitloops as one system and its external actors and dependencies.
- `02-container-view.md`: C4-style container view. Shows the main local process and storage boundaries.
- `05-capture-lifecycle.md`: dynamic flow for agent hooks, Git hooks, checkpoints, and provenance.
- `06-sync-materialization.md`: dynamic flow for watcher/manual/capture-triggered sync and post-sync consumers.

## Reading notes

- Treat `capture` and `sync` as separate flows.
- `post_commit` is an intersection point, not proof that both concerns belong in one diagram.
- The static views answer "what exists".
- The dynamic views answer "what triggers what" and "what side effects follow".
- Component-level details belong in focused sequence diagrams or companion narrative docs unless a static component view proves clearer than both.
- Each diagram includes a glossary for terms that may be unfamiliar to new contributors.

## Companion narrative docs

- [../layered-extension-architecture.md](../layered-extension-architecture.md)
- [../host-substrate.md](../host-substrate.md)
- [../capability-packs.md](../capability-packs.md)
- [../language-adapters.md](../language-adapters.md)
- [../agent-adapters.md](../agent-adapters.md)
- [../devql-sync-producer-ownership.md](../devql-sync-producer-ownership.md)
