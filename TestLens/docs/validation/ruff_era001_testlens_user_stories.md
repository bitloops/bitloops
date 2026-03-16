# TestLens User Stories Useful For Ruff ERA001 Task

Last updated: 2026-03-16

Task brief:
- Confluence page `440500241`
- Title: `Rust Task Brief 2: astral-sh__ruff-15330 (ERA001 script metadata false positive)`
- Base commit: `b2a0d68d70ee690ea871fe9b3317be43075ddb33`

Workspace used for this note:
- Repo path: `/Users/markos/code/bitloops/bitloops/TestLens/b2a0d68d70ee690ea871fe9b3317be43075ddb33`
- DB path: `./target/ruff-era001-task2.db`

Fresh local TestLens ingest on March 16, 2026:
- production ingest: `files: 1472, artefacts: 15913`
- test ingest: `files: 930, suites: 874, scenarios: 4861, links: 65204`
- enumeration mode: `source-only`

This note answers one narrow question:
- which TestLens user stories are genuinely useful for solving the ERA001 script-metadata false-positive task right now

## Most Useful Right Now

### 1. Rule-level safety and harness discovery

User story:
- As an agent preparing the ERA001 fix, I need to know whether the main rule entry point already has linked Ruff tests and which fixture drives them.

Commands:

```bash
testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact commented_out_code \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view summary

testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact commented_out_code \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view tests \
  --min-strength 0.0
```

Current result:
- `commented_out_code` is `partially_tested`
- linked test: `rules[CommentedOutCode, ERA001.py]`

Why this helps on this task:
- It points directly at the Ruff rule harness in `crates/ruff_linter/src/rules/eradicate/mod.rs`.
- It confirms that `ERA001.py` is the core fixture/snapshot path you should extend.
- It matches the task brief’s recommendation to add regression fixture coverage rather than editing in the dark.

Important nuance:
- the default `tests` view currently hides this harness because its computed strength is below the default threshold
- for this task, `--min-strength 0.0` is the right query shape for narrow intra-rule helpers and wrappers

### 2. Script-block boundary helper discovery

User story:
- As an agent fixing the false positive, I need to know whether Ruff already has direct helper tests around script-block parsing and closing-marker precedence.

Command:

```bash
testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact skip_script_comments \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view tests \
  --min-strength 0.0
```

Current result:
- `script_comment`
- `script_comment_end_precedence`

Why this helps on this task:
- This is the highest-value TestLens story for ERA001.
- The task brief explicitly calls out deterministic end-of-block handling when an ordinary comment follows the closing marker.
- TestLens surfaces a direct unit test named `script_comment_end_precedence`, which is almost exactly the edge case the brief wants protected.
- It shows that the relevant helper is not just indirectly exercised; it already has focused unit coverage.

### 3. General commented-out-code regression surface

User story:
- As an agent tightening script-metadata exemptions, I need to see the broad detection tests that protect ERA001 from becoming too permissive outside valid metadata blocks.

Commands:

```bash
testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact comment_contains_code \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view summary

testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact comment_contains_code \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view tests
```

Current result:
- `comment_contains_code` is `partially_tested`
- `10` linked unit tests, including:
  - `comment_contains_code_basic`
  - `comment_contains_code_single_line`
  - `comment_contains_code_with_multiline`
  - `comment_contains_code_with_default_allowlist`
  - `comment_contains_todo`

Why this helps on this task:
- The task brief says to keep existing behavior for true commented-out code outside script metadata.
- This query gives you the existing regression surface for the core detection heuristic.
- It is the best TestLens-supported way to see what general ERA001 sensitivity you must preserve while changing metadata-block handling.

## Useful, But With Limits

### 4. Gap detection for tiny parsing helpers

User story:
- As an agent changing the boundary parser, I need to know whether TestLens can trace tests down to every tiny helper involved in script-tag handling.

Commands:

```bash
testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact script_line_content \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view summary

testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact is_own_line_comment \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view summary

testlens query \
  --db ./target/ruff-era001-task2.db \
  --artefact is_script_tag_start \
  --commit b2a0d68d70ee690ea871fe9b3317be43075ddb33 \
  --view summary
```

Current result:
- these helpers currently show `untested`

Why this is still useful:
- It tells you where current static linkage stops being reliable.
- For this task, TestLens is strong at the rule-function, boundary-helper, and detection-helper level.
- It is weaker for tiny local parsing helpers, even though they may be indirectly exercised by broader tests.

Practical implication:
- use TestLens confidently for `commented_out_code`, `skip_script_comments`, and `comment_contains_code`
- do not treat `untested` on `script_line_content` or `is_script_tag_start` as proof that no tests cover that behavior indirectly

## Less Useful For This Task Right Now

### 5. Coverage-driven branch hunting

Why not:
- this Ruff workspace does not have a validated TestLens coverage flow yet
- the task brief is primarily fixture/snapshot and unit-test driven

Meaning:
- TestLens `coverage()` is not the main story for ERA001 today

### 6. Pre-existing failure detection

Why not:
- TestLens run-outcome ingestion remains deferred outside the current Jest-oriented path

Meaning:
- TestLens is not yet the right tool for answering whether Ruff already has failing ERA001-related tests before your patch

## Recommended TestLens Workflow For ERA001

The highest-value TestLens sequence for this task is:

1. Query `commented_out_code` with `--view summary`
2. Query `commented_out_code` with `--view tests --min-strength 0.0`
3. Query `skip_script_comments` with `--view tests --min-strength 0.0`
4. Query `comment_contains_code` with `--view tests`
5. Treat tiny helper `untested` results as linkage limits, not as definitive absence of coverage

## Bottom Line

The TestLens stories that are genuinely useful for solving this ERA001 task right now are:

- finding the exact Ruff harness fixture for the rule
- discovering the direct helper tests for script-block parsing
- surfacing the exact closing-marker precedence test that matches the task brief
- inspecting the broad general-detection test surface that must not regress
- identifying where current static linkage becomes too shallow to trust on tiny helpers

Compared with the earlier F523 task, this ERA001 task is a better fit for current TestLens linkage because the key boundary helper `skip_script_comments` and the general detection helper `comment_contains_code` already resolve to direct unit tests.
