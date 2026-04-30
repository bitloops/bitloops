---
name: devql-explore-first
description: >
  Use before codebase exploration, bug fixing, SWE-bench tasks, locating
  symbols, files, or tests, or before using rg, grep, sed, find, or source-file
  reads in a repository.
---

# DevQL Explore First

<VERY_IMPORTANT>
For code understanding and exploration, DevQL is the primary discovery tool, not
a one-time preflight. Use `bitloops devql query ...` whenever locating symbols,
files, tests, implementations, callers/usages, or the next source region to
inspect.
</VERY_IMPORTANT>

Do not use `sed`, `rg`, `grep`, `find`, `wc`, directory crawling, or broad
source-file reads to discover task-relevant code while DevQL can answer the
lookup. Use traditional tools only to:

- read bounded ranges returned by DevQL
- inspect files you are editing after DevQL selected them
- run tests or git/status commands
- fall back when DevQL fails, is empty, or contradicts the task

Do not run `bitloops devql --help` or `bitloops devql query --help`.

Choose the most specific selector:

- known `symbolFqn`: use `symbolFqn`
- known file or file range: use `path`, optionally with `lines`
- single concrete identifier, method name, literal, error code, path-like
  string, or copied snippet: use `searchMode: LEXICAL`
- multiple related terms, behavior, concept, or task keywords without one exact
  anchor: omit `searchMode` and use default `AUTO`

Fuzzy symbol-name lookup is included in the lexical lane; do not use a separate
fuzzy selector.

Use compact exploration queries. If the prompt has no concrete anchor, start
with the default `AUTO` query:

```bash
bitloops devql query '{ selectArtefacts(by: { search: "<short behavior phrase or task keywords>" }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine score } } }'
bitloops devql query '{ selectArtefacts(by: { search: "<single identifier, literal, path fragment, or short snippet>", searchMode: LEXICAL }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine score } } }'
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine score } } }'
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>", lines: { start: <start>, end: <end> } }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine score } } }'
```

If DevQL returns relevant paths and line ranges:

- read only about 50 lines before and after those ranges
- do not duplicate the same search with `rg`, `grep`, `glob`, or `find`
- for each new exploration question, query DevQL again instead of switching to
  grep/glob/find
- fall back to normal search only if DevQL fails, is empty, or contradicts the task
- use at most 3 DevQL calls before each bounded source-read phase
