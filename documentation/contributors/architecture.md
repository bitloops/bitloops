---
sidebar_position: 2
title: Architecture Overview
---

# Architecture Overview

A quick map of the Bitloops architecture so you know where the main runtime
layers live and where to go deeper next.

## The Big Picture

Bitloops is organized around four cooperating layers:

1. **Presentation** — CLI commands, dashboard routes, and the public DevQL
   entry points
2. **Host runtime** — capability execution, extension registration, hooks, and
   checkpoint lifecycle
3. **Capability packs and adapters** — domain features plus language, agent,
   model-provider, and connector integrations
4. **Infrastructure** — storage, configuration, telemetry, Git, and shared
   domain types

## Start Here

- [Layered extension architecture](./architecture/layered-extension-architecture.md)
- [Host substrate](./architecture/host-substrate.md)
- [Capability packs](./architecture/capability-packs.md)
- [Language adapters](./architecture/language-adapters.md)
- [Agent adapters](./architecture/agent-adapters.md)
- [DevQL core and pack boundaries](./architecture/devql-core-pack-boundaries.md)
- [GraphQL-first DevQL host runtime ADR](./architecture/decisions/graphql-first-devql-host-runtime.md)

## Where to Go by Task

- Working on CLI commands: `bitloops/src/cli/`
- Working on DevQL host/runtime behavior: `bitloops/src/host/`
- Adding or updating a capability pack: `bitloops/src/capability_packs/`
- Adding or updating an adapter: `bitloops/src/adapters/`
- Touching persistence or migrations: `bitloops/src/host/runtime_store.rs`, `bitloops/src/host/relational_store.rs`, and `bitloops/src/storage/`

Use this page as the orientation layer. The deeper architecture pages under this
section are the canonical contributor docs for the runtime design.
