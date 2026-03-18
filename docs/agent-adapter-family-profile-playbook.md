# Agent Adapter Family vs Profile Playbook

This playbook explains when to add a **protocol family**, when to add a **target profile**, and when to add both.

## Core rule

Start by asking: _is this new behaviour protocol-level reuse, target-level variation, or both?_  
Keep reusable protocol mechanics at the family level and keep target-specific behaviour at the profile level.

## Add a protocol family when

- You are introducing a new hook/event protocol shape that can support multiple targets.
- Parsing, event translation, and lifecycle semantics are mostly shared.
- At least two current or expected targets can reuse the same protocol contract.
- The host would otherwise duplicate protocol orchestration per target.

## Add a target profile when

- The protocol family already exists, but one target needs specific identifiers, aliases, or policy switches.
- The target has unique UX behaviour (for example resume command wording) that should not affect sibling targets.
- You are adding support for another environment that fits an existing protocol family.

## Add both when

- The target introduces a new protocol shape and also requires target-specific overlay behaviour.
- There is no safe existing family to reuse without forcing unrelated targets into special cases.

## Composition model

Bitloops resolves an adapter in two steps:

1. Resolve target profile (including aliases).
2. Validate that profile belongs to the requested protocol family.
3. Resolve the final adapter registration from `family + profile`.
4. Apply runtime compatibility and configuration validation before use.

Legacy agent-name resolution remains available as a compatibility path, but the canonical model is family/profile composition.

## Ownership boundaries

Family-level ownership:

- Protocol event contract and normalisation shape.
- Shared capability surface.
- Family-level runtime/config constraints.

Profile-level ownership:

- Target identity, aliases, and profile-specific behaviour.
- Target quirks that do not change the protocol contract.
- Profile-level runtime/config constraints.

Host ownership:

- Registration and composition resolution.
- Compatibility, readiness, and configuration validation.
- Structured observability and correlation metadata.

## Testing checklist for new family/profile work

- Registration tests for duplicate IDs, alias collisions, and invalid family/profile relationships.
- Composition tests for valid `family + profile` resolution and deterministic failures for mismatches.
- Configuration validation tests for missing required values, malformed values, and conflicting values.
- Readiness tests for ready and not-ready states.
- Correlation/observability tests for resolution traces and metadata propagation.

## Worked examples from built-ins

- `jsonl-cli` family with profiles: `claude-code`, `codex`, `cursor`, `opencode`.
- `json-event` family with profiles: `copilot`, `gemini`.

In both examples, protocol-level concerns stay with the family while profile IDs and aliases stay target-specific.

## Thought experiment (new target)

If a new target uses the same JSONL hook contract as `codex` but only changes naming and setup paths:

- Add a new **target profile** under `jsonl-cli`.
- Reuse existing family behaviour.
- Add profile-specific tests and config/readiness checks.

If a new target introduces a fundamentally different event protocol:

- Add a new **protocol family**.
- Add one initial target profile for that family.
- Add composition and compatibility tests before onboarding additional profiles.
