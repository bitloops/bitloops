---
name: using-devql
description: >
  Use when answering repo-understanding questions in a Bitloops repo with DevQL
  guidance enabled, especially when you need to locate code by path, line
  range, exact symbol, or approximate/conceptual search before reading files.
---

# Using DevQL

## Overview

DevQL is Bitloops's typed repo-intelligence surface. In repos where this
guidance is enabled, use DevQL first for repo-understanding questions when it
is available in the current session. If DevQL returns nothing useful or is not
available, fall back to targeted repo search or file reads.

## When to Use

- understanding what a file, function, module, class, or symbol does
- resolving the concrete artefacts matched by a path or line range
- looking up an exact symbol with `symbolFqn`
- looking up identifiers, literals, paths, or snippets with `search` plus `searchMode: LEXICAL`
- looking up an approximate name or conceptual behavior with `search` in the default `AUTO` mode
- answering architecture questions after selecting a concrete area

## Choosing The `by` Selector

- `path`: use this when the starting point is one file; Bitloops selects
  artefacts from that file.
- `path + lines`: use this when the starting point is a specific region inside
  a file; Bitloops limits the seed artefacts to that line range.
- `symbolFqn`: use this when the starting point is one exact artefact or symbol.
- `search`: use this when you do not yet have an exact seed.
  Use `searchMode: LEXICAL` for identifiers, literals, file-ish strings, and
  snippets. Keep the default `AUTO` mode for approximate or conceptual search.
  Use `IDENTITY`, `CODE`, or `SUMMARY` only when you need advanced narrowing.

## Process

1. Choose the most specific selector available: `path`, `path + lines`, or
   `symbolFqn` when the request is exact; `search` when you need lookup rather
   than direct addressing.
2. If the request is an identifier, literal, path-like string, or snippet,
   distill it and run `search` with `searchMode: LEXICAL`.
3. If the request is approximate or conceptual, distill it to a short phrase
   and run `search` in the default `AUTO` mode first. Inspect
   `artefacts(first: 10)` and, when useful, `searchBreakdown(first: 3)` to see
   which retrieval mode is carrying the result.
4. Reserve `searchMode: IDENTITY`, `CODE`, or `SUMMARY` for advanced narrowing
   when `AUTO` is broad but you already know which representation is likely to
   matter.
5. Once you have a concrete file or artefact, run `overview`.
6. If the response includes `expandHint`, `schema`, or another typed follow-up
   hint, read that hint before composing the next query.
7. Expand only one relevant stage or `artefacts(first: ...)` if the overview
   shows that more detail is needed.
8. If DevQL returns nothing useful, fall back to targeted repo search or file
   reads.

## Query Templates

```bash
# Use AUTO search first when the request is approximate or conceptual
bitloops devql query '{ selectArtefacts(by: { search: "<distilled conceptual phrase>" }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine score } searchBreakdown(first: 3) { lexical { path symbolFqn score } identity { path symbolFqn score } code { path symbolFqn score } summary { path symbolFqn score } } } }'

# Use LEXICAL search for identifiers, literals, paths, or snippets
bitloops devql query '{ selectArtefacts(by: { search: "<identifier or snippet>", searchMode: LEXICAL }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine score } } }'

# Ask for overview once the selection is concrete
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { overview } }'

# Same overview shape for file or file+line selectors
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>", lines: { start: <start>, end: <end> } }) { overview } }'
```

## Reading Response Hints

- Treat `overview` as the compact first pass.
- Treat `searchBreakdown` as an `AUTO`-only widening tool when the unified top
  hits are relevant but you want to see which lexical or semantic mode is also
  surfacing candidates.
- When DevQL returns `expandHint`, `schema`, or another typed follow-up hint,
  read it before composing the next query.
- Prefer the hint's suggested parameters and drill-down shape over guessing
  the next stage call.

## Sandbox Execution

- In sandboxed agent environments, run `bitloops devql ...` outside the sandbox by default.
- These commands rely on Bitloops-managed daemon and runtime state under platform app directories, so they can fail inside a workspace-only sandbox even when DevQL is healthy.
- If your platform requires approval or escalation for out-of-sandbox commands, request it immediately before running `bitloops devql ...`.

## Common Mistakes

- Do not reintroduce obsolete selector names from older DevQL guidance.
- Do not dump the full conversational prompt into `search`; distill it.
- Do not use conceptual `AUTO` search for obvious identifiers or snippets when
  `searchMode: LEXICAL` is the right tool.
- Do not force `IDENTITY`, `CODE`, or `SUMMARY` unless you are intentionally
  narrowing an already-understood search problem.
- Do not ignore `expandHint` or other response hints when DevQL already tells
  you how to drill down.
- Do not start with stage-specific detail queries before selecting a concrete
  artefact.
- Do not use DevQL for edits, builds, tests, formatting, or git operations.

## Integration

- If `overview` shows something relevant, expand only the one stage needed for
  the current question.
- If DevQL is unavailable or empty, switch to targeted `rg` or file reads.
