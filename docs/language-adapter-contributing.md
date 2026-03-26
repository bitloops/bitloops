# Contributing a New Language Adapter Pack

This guide explains how to add a new language pack to the Bitloops language-adapter runtime without changing DevQL core dispatch code.

## 1) Implement the `LanguageAdapterPack` contract

The runtime contract lives in:

- `bitloops/src/host/language_adapter/pack.rs`

A new pack must implement:

- `descriptor()` -> returns a `LanguagePackDescriptor`
- `canonical_mappings()` -> mapping table from parser kinds to canonical projections
- `supported_language_kinds()` -> complete supported parser-kind set
- `extract_artefacts(content, path)` -> language symbol extraction
- `extract_dependency_edges(content, path, artefacts)` -> dependency graph extraction

Optional lifecycle hooks:

- `extract_file_docstring(content)` (default `None`)
- `migrations()` (default `[]`)
- `health_checks()` (default `[]`)

## 2) Canonical mapping rules and projections

Canonical mapping types live in:

- `bitloops/src/host/language_adapter/canonical.rs`

Rules:

- Canonical mapping is table-driven (`CanonicalMapping`), not match-block code in DevQL.
- Every `CanonicalMapping.language_kind` must appear in `supported_language_kinds()`.
- `MappingCondition::WhenInsideParent` overrides `Always` when `inside_parent == true`.
- Some language kinds may intentionally have no canonical projection (language-specific-only artefacts).

Projections are defined in:

- `bitloops/src/host/devql/core_contracts.rs` (`CanonicalKindProjection`)

## 3) `LanguageArtefact` and `DependencyEdge` type contracts

Shared types are defined in:

- `bitloops/src/host/language_adapter/types.rs`

Key expectations:

- `LanguageArtefact.symbol_fqn` must be stable and unique per symbol within file scope.
- `parent_symbol_fqn` should be set for nested symbols (methods, members, etc).
- `canonical_kind` should be populated by extractor mapping when a canonical projection exists.
- `DependencyEdge` must include accurate `edge_kind`, source symbol, and either target symbol fqn or unresolved reference.
- `metadata` should preserve edge-resolution and form details (`import`, `call`, `export`, `ref` forms).

## 4) Step-by-step: add a hypothetical Python pack

Create modules using file+folder sibling pattern (no `mod.rs` root files):

- `bitloops/src/adapters/languages/python.rs`
- `bitloops/src/adapters/languages/python/pack.rs`
- `bitloops/src/adapters/languages/python/extraction.rs`
- `bitloops/src/adapters/languages/python/edges.rs`
- `bitloops/src/adapters/languages/python/canonical.rs`

Update registry factory:

- `bitloops/src/adapters/languages.rs`

Actions:

1. Define `PYTHON_CANONICAL_MAPPINGS` and `PYTHON_SUPPORTED_LANGUAGE_KINDS` in `python/canonical.rs`.
2. Implement symbol extraction in `python/extraction.rs` returning `Vec<LanguageArtefact>`.
3. Implement dependency extraction in `python/edges.rs` returning `Vec<DependencyEdge>`.
4. Implement `PythonLanguageAdapterPack` in `python/pack.rs`.
5. Expose module root in `python.rs`.
6. Register pack in `builtin_language_adapter_packs()` in `languages.rs`.

No DevQL dispatch changes should be required.

## 5) Testing strategy for new packs

Required checks:

1. Canonical mapping table coverage tests (supported kinds and projected kinds).
2. Artefact extraction tests for representative constructs and edge cases.
3. Dependency edge tests for imports/exports/calls/references and dedup/order behavior.
4. Registry integration test (`LanguageAdapterRegistry`) for pack registration and execution.

Recommended targeted commands:

```bash
cargo check -p bitloops
cargo test -p bitloops --lib host::devql::mapping_tests -- --nocapture
cargo test -p bitloops --lib extraction_<your_language> -- --nocapture
```

## 6) Where registration happens

Built-in language adapter registration is centralized at:

- `bitloops/src/adapters/languages.rs` -> `builtin_language_adapter_packs()`

Runtime registry initialization is in:

- `bitloops/src/host/devql.rs` -> `language_adapter_registry()`

`devql packs` language-adapter lifecycle/reporting is also assembled in `host/devql.rs`.

## 7) Related but separate (current state)

The test-harness capability keeps its own language-specific mapping code under:

- `bitloops/src/capability_packs/test_harness/mapping/languages`

That path is currently separate from the host language-adapter runtime in this guide.

Future improvement direction:

- move test-harness mapping into a capability-side projection layer that consumes
  `LanguageArtefact` / `DependencyEdge` outputs from `LanguageAdapterPack`
- keep `LanguageAdapterPack` generic and closed to capability-specific domain semantics
- reduce duplicate language-logic maintenance across host and capability code
