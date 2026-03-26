# Agent Adapter Extension Playbook

## Purpose

This playbook explains how to extend the adapter model with package-shaped adapters, richer runtime semantics, and host-owned policy/provenance controls.

It is based on the current implementation in:

- `bitloops/src/adapters/agents/adapters/types.rs`
- `bitloops/src/adapters/agents/adapters/registry.rs`
- `bitloops/src/adapters/agents/canonical.rs`
- `bitloops/src/adapters/agents/policy.rs`
- `bitloops/src/host/checkpoints/lifecycle.rs`
- `bitloops/src/host/hooks/runtime/agent_runtime.rs`

## model summary

introduces five extension surfaces:

1. Package-ready adapter metadata and lifecycle boundaries.
2. Deterministic package discovery and validation diagnostics.
3. Rich canonical contract support for streaming/progress/partial/resumable flows.
4. Host-owned policy, provenance, and audit semantics.
5. Built-in adapter retrofit rules for rich vs collapsed canonical mappings.

## Package metadata model

Define package metadata through `AgentAdapterPackageDescriptor`.

Required fields:

- `id`, `display_name`, `version`
- `metadata_version`
- `source` (`first-party-linked` or `manifest-package`)
- `trust_model`
- `boundary`
- `lifecycle`
- `compatibility`

Current schema version:

- `HOST_ADAPTER_PACKAGE_METADATA_VERSION = 1`

Validation is deterministic through `validate_package_metadata()` and returns stable issue codes such as:

- `unsupported_metadata_version`
- `invalid_package_version`
- `package_id_mismatch`
- `source_trust_mismatch`
- `missing_compatibility_claims`
- plus boundary/lifecycle/compatibility validation failures

## Discovery and validation flow

Use the registry-owned flow:

1. `discover_packages()`
2. `package_discovery_reports()`
3. `validate_package_metadata()`
4. `package_validation_reports()`

Behavioural rules:

- Discovery and validation must be deterministic for the same registry state.
- Invalid metadata must never silently degrade into a valid adapter registration.
- Diagnostics must include package id/source/version metadata so failures are auditable.

## Family/profile interaction

Package metadata does not replace family/profile composition.

Resolution order remains:

1. target/family resolution (`resolve` or `resolve_composed`)
2. package metadata compatibility and runtime checks
3. readiness/config checks
4. lifecycle routing

Keep protocol mechanics in family/profile descriptors and package concerns in package metadata.

## Rich canonical runtime semantics

Use `CanonicalContractCompatibility` and related types:

- `CanonicalStreamEvent`
- `CanonicalProgressUpdate`
- `CanonicalResultFragment`
- `CanonicalResumableSession`

Rules:

- Rich targets should attach compatibility/progress/resumable context.
- Simpler targets should emit valid collapsed flows (default compatibility, no fabricated progress stream).
- Do not encode target-specific transport details into canonical host types.

## Policy, provenance, and audit ownership

Host policy semantics are centralised in `policy.rs`.

Use:

- `PolicyDecision` for allow/deny/restricted/redacted outcomes
- `ProvenanceMetadata` for source and correlation propagation
- `AuditRecord` for policy/runtime lifecycle audit trails
- `enforce_policy(...)` and `attach_policy_audit(...)` for deterministic enforcement paths

Rules:

- Policy authority remains host-owned.
- Adapters may surface signals, but they do not decide final policy outcomes.
- Provenance and audit metadata should be propagated across request, response, and failure paths.

## Built-in retrofit guidance

Current built-ins should be treated as two buckets:

- Rich-capable mappings: attach richer canonical context where meaningful.
- Collapsed mappings: remain on simple canonical flows without ambiguity.

Do not force richer semantics onto targets that do not provide enough signal.

## Testing checklist

When adding a new packaged adapter or runtime extension:

1. Package metadata validation:
   - metadata version mismatch
   - invalid semantic version
   - source/trust mismatch
   - compatibility claim failures
2. Discovery diagnostics:
   - valid package appears in discovery reports
   - invalid package yields deterministic issue codes
3. Canonical runtime model:
   - rich path includes compatibility/progress/resumable metadata
   - collapsed path remains valid and unambiguous
4. Policy/provenance/audit:
   - allow/deny/restricted/redacted coverage
   - provenance propagation across request/response/failure
   - audit records include decision and correlation context
5. Warning-free quality gate:
   - `cargo clippy --manifest-path bitloops/Cargo.toml --lib --tests -- -D warnings`

## Worked examples from current code

- First-party linked package descriptors are defined in built-in registrations.
- Registry package discovery and validation report APIs are available in `AgentAdapterRegistry`.
- Canonical rich contract types and builders are in `canonical.rs` and `canonical/*`.
- Policy/provenance/audit helpers are in `policy.rs`.
- Lifecycle enrichment for rich/collapsed built-in paths is in `host/checkpoints/lifecycle.rs`, with hook routing in `host/hooks/runtime/agent_runtime.rs`.
