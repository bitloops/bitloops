---
name: using-devql
description: >
  Use when understanding code structure, resolving artefacts by path or line
  range, resolving approximate symbol names with fuzzy lookup, resolving
  conceptual requests with unified search, finding
  callers/usages/imports/tests/checkpoints/clones/dependencies,
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
- resolving a likely symbol name when the human-entered name may be approximate or misspelled
- finding callers, usages, imports, tests, checkpoints, clones, or dependencies
- getting a structured overview of a file or area
- answering architecture questions

## Agent Flow

1. Select the target with `symbolFqn`, `search`, `path`, or `path + lines`.
   Use `search` when the request is conceptual or when the symbol name may be approximate.
2. Ask for `summary` only if you need orientation or to discover which stage to expand.
3. Rerun with `artefacts(first: ...)` or the relevant stage `items(first: ...)`.
4. Return the concrete rows. Summaries are optional follow-up, not substitutes.
5. If DevQL returns nothing useful, fall back to targeted repo search or file reads.

## Selector Routing

- If the prompt contains a path, line range, scoped symbol, backticked identifier, function-like token, or other code-ish artefact clue, prefer a structured selector first.
- Use `path` or `path + lines` for file references, `symbolFqn` for exact symbol references, and `search` when the user likely named a symbol approximately, misspelled it, or asked for behaviour conceptually.
with queries such as `build invoice pdf`, `validate webhook signature`, or `render checkout summary`.
- Do not pass the whole conversational prompt into `search` when it contains extra wrapper text such as `can you help`, `fix this`, or `help me understand the codebase`.
- Distill semantic lookup into a short intent phrase instead of removing stopwords mechanically. Preserve meaningful qualifiers and drop conversational filler.
- For mixed prompts, try structured lookup first and use `search` as a fallback or supplement when the artefact clue is weak.

Examples:

- `renderInvoicePdf is broken` -> prefer `search` or `symbolFqn`
- `src/payments/invoice.ts:42` -> prefer `path + lines`
- `find the code that builds invoice PDFs` -> prefer `search`
- `help me understand the codebase` -> do not use `search` first; start with scoped `summary` or a concrete project/file selector

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

# Unified search for approximate symbols or conceptual requests
bitloops devql query '{ selectArtefacts(by: { search: "<natural-language request or approx symbol>" }) { artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine } } }'

# Concrete callers/usages/imports once the symbol is known
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { deps(kind: CALLS, direction: IN, includeUnresolved: true) { items(first: 50) { edgeKind startLine endLine fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'

# Discover the exact row fields for the chosen stage
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { deps(kind: CALLS, direction: IN, includeUnresolved: true) { schema } } }'

# Concrete tests that directly target the selected artefact
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { tests { summary items(first: 20) { artefact { name filePath startLine endLine } coveringTests { testName suiteName filePath startLine endLine } } } } }'

# use sparingly to see the whole schema
bitloops devql schema
```

## Do Not Use DevQL

- when you don't have a specific artefact or file in mind
- editing files
- running tests, builds, or git commands
- literal string search when you already know the exact text
