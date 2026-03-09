# Shadow Branches

Shadow branches are private git refs that automatically capture your working-tree state after each AI turn. They run in parallel to your real branch history — invisible to normal git workflows — and are cleaned up once you commit or rewind.

---

## How they fit into the workflow

```
AI turn ends  →  stop hook  →  save_step()  →  shadow branch commit  (temporary checkpoint)
git commit    →  post-commit hook            →  bitloops/checkpoints/v1  (committed checkpoint)
```

A **committed checkpoint** is created when you run `git commit`. The `post-commit` hook fires, extracts the `Bitloops-Checkpoint` trailer the `prepare-commit-msg` hook injected, and condenses the session state onto `bitloops/checkpoints/v1`.

Shadow branch commits are the *intermediate* state captured between your commits. After each AI turn the `stop` hook fires `save_step()`, which snapshots:

1. **Your working-tree state** — every modified and new file at that exact moment
2. **Session metadata** — the transcript up to that turn, your initial prompt, and structural info

These snapshots live on the shadow branch until you commit, at which point the committed checkpoint supersedes them. They are never pushed to your remote branch and do not appear in `git log`.

---

## Naming format

```
refs/heads/bitloops/<base7>-<wt6>
```

| Component | Description |
|-----------|-------------|
| `bitloops/` | Fixed namespace prefix |
| `<base7>` | First 7 hex characters of the **base commit** — the HEAD at the moment the session started |
| `<wt6>` | First 6 hex characters of the SHA-256 hash of the **worktree ID** |

**Examples:**

```
bitloops/abc1234-e3b0c4   ← main worktree, session started at commit abc1234...
bitloops/abc1234-7f8e9d   ← a linked worktree, same base commit
bitloops/f7e3a1b-e3b0c4   ← main worktree, different base commit
```

The worktree ID for the **main worktree** is the empty string `""`, whose SHA-256 is `e3b0c44...`, so `wt6 = e3b0c4`. Every linked worktree has a unique ID (its internal git name), producing a different six-character hash.

This two-part key means two concurrent sessions in different worktrees never share a shadow branch, even when they start from the same commit.

---

## One branch, many commits

A shadow branch grows one commit per checkpoint. Each `SaveStep` call appends a new commit to the tip of the same branch:

```
bitloops/abc1234-e3b0c4
  commit C1  ← checkpoint 1 (first prompt)
     ↓
  commit C2  ← checkpoint 2
     ↓
  commit C3  ← checkpoint 3 (latest)
```

The first commit on the branch has no parent (it is an orphan relative to the main history). Subsequent commits are chained linearly. If the tree hash does not change between two saves (no file was modified and no transcript delta), the commit is skipped and the previous SHA is reused.

---

## Commit tree structure

Each shadow commit contains a full working-tree snapshot **plus** metadata embedded under `.bitloops/`:

```
<shadow commit tree>
├── src/
│   └── main.rs          ← modified source file
├── new_feature.rs        ← newly added file
└── .bitloops/
    └── metadata/
        └── <session-id>/
            ├── full.jsonl       ← complete session transcript (JSONL)
            ├── prompt.txt       ← user's initial prompt
            ├── context.md       ← generated context summary
            └── content_hash.txt ← SHA-256 of the transcript (dedup guard)
```

Deleted files are **absent** from the tree — their removal is encoded by their non-presence, consistent with how git trees work.

All paths are resolved against the repository root, not the shell's current working directory, so subdirectory sessions produce the same tree layout.

---

## Commit message trailers

Every shadow commit message carries three structured trailers:

```
Checkpoint 3

Bitloops-Metadata: .bitloops/metadata/2026-02-15-abc123def456
Bitloops-Session:  2026-02-15-abc123def456
Bitloops-Strategy: manual-commit
```

| Trailer | Value | Purpose |
|---------|-------|---------|
| `Bitloops-Metadata` | Path to the metadata dir inside the tree | Tells the CLI where to find transcript, prompt, metadata.json |
| `Bitloops-Session` | Session ID (`YYYY-MM-DD-<hex12>`) | Groups all checkpoints for a single session |
| `Bitloops-Strategy` | `manual-commit` | Identifies the checkpoint strategy used |

For task/subagent checkpoints a fourth trailer is added:

```
Bitloops-Metadata-Task: .bitloops/metadata/<session-id>/tasks/<tool-use-id>
```

---

## Reachability and worktree filtering

When the branch-list view (`bitloops explain`) collects temporary checkpoints, it applies two filters:

**1. Worktree filter** — only shows shadow branches whose `wt6` matches the current worktree. This prevents the main worktree from showing checkpoints created in a linked worktree and vice versa.

**2. Reachability filter** — only shows shadow branches whose `base7` prefix corresponds to a commit **reachable from the current HEAD**. This prevents feature-branch sessions from appearing when you switch back to `main`.

The `explain --checkpoint <sha>` command intentionally skips both filters — when you have a specific SHA you want to inspect, it searches across all shadow branches with no restrictions.

---

## Migration on rebase / pull

When HEAD advances (a pull or rebase lands), the base commit encoded in the branch name is stale. The CLI detects this on the next hook invocation and migrates automatically:

1. Compute the new shadow branch name from the updated HEAD
2. Move the ref: `git update-ref refs/heads/bitloops/<newbase>-<wt6> <old-tip>`
3. Delete the old ref: `git update-ref -d refs/heads/bitloops/<oldbase>-<wt6>`
4. Update the persisted session state with the new base commit

---

## Lifecycle

```
session start
    └─► shadow branch created (orphan commit)
            │
            ▼  (after each AI turn — stop hook → save_step)
        shadow commits accumulate on shadow branch tip
            │
            ▼  (user runs: git commit → post-commit hook fires)
        committed checkpoint written to bitloops/checkpoints/v1
        shadow branch retained (enables post-commit rewind)
            │
            ▼  (cleanup: next session / doctor / uninstall)
        shadow branch deleted via: git branch -D bitloops/<base7>-<wt6>
```

The metadata branch `bitloops/checkpoints/v1` is separate from shadow branches — it holds the **committed** checkpoint index and is never a shadow branch itself.

---

## Reading metadata from a shadow commit

Because the full session state is stored in the git object database, any shadow commit can be inspected without checking it out:

```bash
# Read the transcript from shadow commit <sha>
git show <sha>:.bitloops/metadata/<session-id>/full.jsonl

# Read the initial prompt
git show <sha>:.bitloops/metadata/<session-id>/prompt.txt

# Read metadata (agent type, token usage, etc.)
git show <sha>:.bitloops/metadata/<session-id>/metadata.json
```

This is exactly how `bitloops explain --checkpoint <sha>` works when the SHA points to a temporary checkpoint — it reads the `Bitloops-Metadata` trailer from the commit message to locate the metadata directory, then issues `git show` calls against the commit's tree.
