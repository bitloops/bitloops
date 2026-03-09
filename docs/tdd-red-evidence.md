# TDD RED Evidence Policy

## Purpose
This policy defines the RED-evidence gate for TDD work in the CLI repository.  
It is currently enforced for the Logging epic (`CLI-324`), and can be reused for future epics.

## RED Evidence Definition
RED evidence means a Jira comment on the relevant test subtask that includes:

1. The exact test command that was run.
2. A failing result (`FAILED` / non-zero).
3. A meaningful failure reason from the test assertion or panic.

For Logging, the expected command form is:

```bash
cargo test <test-path> -- --exact --nocapture
```

## Gate Rules
1. Test harness/scaffolding must exist first (no "module not found" failures).
2. Implementation tasks are blocked until RED evidence exists for all required test subtasks.
3. Reviewers must verify RED evidence exists before approving implementation completion.

## Prohibited Shortcuts
1. Do not use `#[ignore]` to bypass failing tests.
2. Do not claim RED on compile failures caused by missing scaffolding.
3. Do not modify tests to make implementation appear complete unless the test is explicitly approved to change.

## Reviewer Checklist
1. Confirm each required subtask has a RED evidence comment.
2. Confirm implementation changes make the same tests GREEN.
3. Confirm no tests were silenced or bypassed.

## Logging Epic Notes
For `CLI-324`:
1. RED evidence is tracked under `CLI-325` subtasks (`CLI-326` through `CLI-351`).
2. Harness/scaffolding is tracked under `CLI-352`.
3. Implementation is tracked under `CLI-354`.

