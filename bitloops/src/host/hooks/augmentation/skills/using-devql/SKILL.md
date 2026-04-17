---
name: using-devql
description: >
  Use when understanding code structure, resolving artefacts by path or line
  range, finding callers/usages/imports/tests/checkpoints/clones/dependencies,
  or answering architecture questions in a repo with DevQL enabled.
---

# Using DevQL

## Prime Directive

This repo has DevQL, a semantic code index. For code understanding and repo
exploration, you MUST use DevQL first before falling back to broad repo search,
file reads, or directory crawling.

Use `summary` only for first-pass orientation when you do not yet know whether
the selector matched anything or which stage to expand. If the artefact or
question is already known, query concrete rows with `artefacts(first: ...)` or
stage `items(first: ...)`.

If DevQL returns no useful artefacts or stage rows, fall back to targeted repo
search or file reads.

## Use DevQL When

- understanding what a file, function, module, class, or symbol does
- resolving the concrete artefacts matched by a path or line range
- finding callers, usages, imports, tests, checkpoints, clones, or dependencies
- getting a structured overview of a file or area
- answering architecture questions

## Agent Flow

1. Select the target with `symbolFqn`, `path`, or `path + lines`.
2. Ask for `summary` only if you need orientation or to discover which stage to expand.
3. Rerun with `artefacts(first: ...)` or the relevant stage `items(first: ...)`.
4. Return the concrete rows. Summaries are optional follow-up, not substitutes.
5. If DevQL returns nothing useful, fall back to targeted repo search or file reads.

## Sandbox Execution

- In sandboxed agent environments, run `bitloops devql ...` outside the sandbox by default.
- These commands rely on Bitloops-managed daemon and runtime state under platform app directories, so they can fail inside a workspace-only sandbox even when DevQL is healthy.
- If your platform requires approval or escalation for out-of-sandbox commands, request it immediately before running `bitloops devql ...`.

## Core Commands

```bash
# Orientation only
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>" }) { summary } }'

# Concrete artefacts for a known file or line range
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>", lines: { start: <start>, end: <end> } }) { artefacts(first: 20) { path symbolFqn canonicalKind startLine endLine } } }'

# Concrete callers/usages/imports once the symbol is known
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { deps(kind: CALLS, direction: IN, includeUnresolved: true) { items(first: 50) { edgeKind startLine endLine fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'

# Discover the exact row fields for the chosen stage
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { deps(kind: CALLS, direction: IN, includeUnresolved: true) { schema } } }'

bitloops devql schema
```

## Do Not Use DevQL For

- editing files
- running tests, builds, or git commands
- literal string search when you already know the exact text
