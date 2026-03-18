# Agent Adapter Family/Profile Playbook

## Purpose

This guide explains when to add a **protocol family**, when to add a **target profile**, and when to add both.
It is grounded in the Phase 2 implementation in `bitloops_cli/src/engine/agent/adapters.rs` and lifecycle routing in `bitloops_cli/src/engine/lifecycle/adapters.rs`.

## Core Terms

- **Protocol family**: shared transport and event-shape behaviour reused across multiple targets.
- **Target profile**: target-specific behaviour layered on top of a family.
- **Adapter registration**: the concrete host-facing binding for one supported target, with compatibility, config, and readiness metadata.

## Current Built-in Model

- `jsonl-cli` family:
  - Profiles: `claude-code`, `codex`, `cursor`, `opencode`
- `json-event` family:
  - Profiles: `copilot`, `gemini`

This is the reference shape for future extensions.

## Decision Rules

Add a **new protocol family** when all of the following are true:

- The hook/event format is materially different from existing families.
- Existing family-level parsing or lifecycle semantics would become conditional and hard to reason about.
- At least one second target is likely to reuse the same transport/event model.

Add a **new target profile** when all of the following are true:

- The target can reuse an existing family transport/event model.
- Differences are target-level (CLI flags, hook aliases, transcript locations, protected directories, resume command format).
- Family-level host semantics do not need to change.

Add **both** when:

- You are introducing a fundamentally new protocol model and a first target that uses it.

## Ownership Boundaries

Family-level behaviour belongs to:

- Hook/event protocol shape
- Shared lifecycle translation approach
- Shared compatibility envelope for that protocol model

Profile-level behaviour belongs to:

- Target identity and aliases
- Target-specific config schema
- Resume command string format
- Target quirks that do not change protocol semantics

Do not push target quirks into family logic unless the behaviour is genuinely protocol-level.

## Composition Model

Resolution is host-owned and composed:

1. Resolve input to adapter/profile metadata.
2. Validate runtime compatibility.
3. Validate family/profile association.
4. Resolve the concrete adapter registration.
5. Emit correlation metadata for diagnostics.

Lifecycle routing composes using `(protocol_family, target_profile)` and then dispatches to the concrete lifecycle adapter.

## Implementation Checklist

When adding a new family:

1. Add a new `AgentProtocolFamilyDescriptor`.
2. Define compatibility and runtime metadata.
3. Define family config schema and validation rules.
4. Add at least one profile using that family.
5. Add lifecycle composition mapping for the family/profile pairs.

When adding a new profile:

1. Add a `AgentTargetProfileDescriptor` mapped to an existing family.
2. Add profile aliases (if needed).
3. Add profile config schema.
4. Register the adapter with profile + family metadata.
5. Add lifecycle composition mapping for that profile.

## Testing Checklist

For every new family/profile change:

1. Registration validation:
   - Duplicate ids, alias collisions, family/profile mismatch
2. Composed resolution:
   - Valid family/profile composition
   - Invalid composition failure path
3. Compatibility:
   - Supported and unsupported runtime metadata paths
4. Config and readiness:
   - Missing required config
   - Valid config path
5. Observability:
   - Correlation metadata populated
   - Resolution diagnostics include family/profile

## Thought Experiment: Adding "Acme CLI"

If `Acme CLI` emits JSONL hooks compatible with `jsonl-cli`:

- Add profile `acme-cli` under `jsonl-cli`.
- Do not add a new family.
- Add target-specific aliases and resume command behaviour at profile/adapter level.

If `Acme CLI` introduces a novel streaming event protocol:

- Add a new family (for example `stream-event-cli`).
- Add profile `acme-cli` under that family.
- Add lifecycle composition mapping for `(stream-event-cli, acme-cli)`.
