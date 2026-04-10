# Protocol Family vs Target Profile Playbook

For package-ready/runtime guidance, see `documentation/contributors/guides/agent-extension-playbook.md`.

This playbook explains when to add a new **protocol family**, when to add a new **target profile**, and when to add both.

## Core model

- `AgentProtocolFamilyDescriptor` holds behaviour that can be reused across several targets.
- `AgentTargetProfileDescriptor` holds target-specific behaviour layered on top of a family.
- `AgentAdapterDescriptor` binds one concrete built-in adapter to one family and one profile.
- `AgentAdapterRegistry::resolve_composed(family, profile)` resolves the runtime target through composition, with deterministic errors for invalid combinations.

## Decision criteria

Add a new protocol family when:

- Hook/event semantics differ in a way that cannot be represented as profile-only variation.
- Transcript/session primitives differ enough that multiple profiles would otherwise duplicate core mechanics.
- Runtime/config/readiness rules are shared by a new cluster of targets but not by existing families.

Add a new target profile when:

- The target uses an existing family’s protocol mechanics.
- Differences are target-specific (aliases, naming, quirks, defaults, small capability deltas).
- Reuse of existing family validation/readiness/compatibility logic remains correct.

Add both when:

- The new environment introduces genuinely new protocol mechanics and also has target-specific overlays.

## What belongs where

Family-level concerns:

- Reusable protocol mechanics.
- Family runtime compatibility and family config schema.
- Shared family-level capabilities.

Profile-level concerns:

- Target identity and aliases.
- Target-specific quirks or policy overlays.
- Profile runtime compatibility and profile config schema.

Adapter-level concerns:

- Concrete host wiring (`create_agent`, hook install/uninstall, project detection, resume command).
- Adapter-specific config schema when needed.

## Current built-in examples

- `jsonl-cli` family:
  - `claude-code`
  - `codex`
  - `cursor`
  - `opencode`

- `json-event` family:
  - `copilot`
  - `gemini`

These examples show one family supporting several profiles while preserving target-specific behaviour in profile/adapter metadata.

## Extension workflow

1. Define or reuse a family descriptor.
2. Define a profile descriptor bound to that family.
3. Register the adapter descriptor with that family/profile pair.
4. Ensure alias maps and composed resolution are deterministic.
5. Add readiness/config/runtime checks for new metadata.

## Test checklist

When adding a family/profile:

- Registration rejects invalid descriptors (collisions, invalid family/profile links).
- `resolve_composed` succeeds for valid family/profile and fails for invalid combinations.
- Runtime compatibility failures are explicit.
- Config schema validation covers required, invalid, and conflicting values.
- Readiness reports `Ready` and `NotReady` states clearly.
- Resolution trace/correlation metadata includes family/profile/runtime path.

## Thought experiment: adding a new target

If a new target “X” reuses `json-event` semantics but has different naming and one extra target quirk:

- Add `profile-x` under `json-event`.
- Keep family unchanged.
- Add profile/adapter tests and readiness/config coverage.

If target “Y” introduces a fundamentally different hook/event contract:

- Add a new family for that contract.
- Add one or more profiles under it.
- Add family-level tests before expanding profile coverage.
