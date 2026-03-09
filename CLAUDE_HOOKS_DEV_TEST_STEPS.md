# Claude Hooks Dev Test Steps (Rust Bitloops)

## 1) Build and install local CLI

```bash
cd bitloops_cli
cargo install --path . --force
bitloops --version
```

## 2) Create a fresh repo outside this workspace

```bash
mkdir -p /tmp/bitloops-hooks-smoke
cd /tmp/bitloops-hooks-smoke
rm -rf test-repo
mkdir test-repo
cd test-repo
git init
```

## 3) Initialize agent hooks, then enable Bitloops

```bash
bitloops init
bitloops enable
cat .claude/settings.json
ls -la .git/hooks
```

Expected:

- Claude hooks installed in `.claude/settings.json`
- Git hooks installed (`prepare-commit-msg`, `commit-msg`, `post-commit`, `pre-push`)

## 4) Validate empty-repo behavior (no HEAD yet)

Start a Claude chat and make a file change (for example create `index.html`).

Then run:

```bash
git rev-parse --verify HEAD
ls -la .git/bitloops-sessions
cat .git/bitloops-sessions/*.json
ls -la .bitloops
ls -la .bitloops/tmp
```

Expected:

- `git rev-parse --verify HEAD` fails (no commit yet)
- session state exists in `.git/bitloops-sessions`
- no checkpoint crash
- metadata may not exist yet (checkpointing is no-op until first commit)

## 5) Create first commit

```bash
git add .
git commit -m "initial"
```

## 6) Run second Claude turn (with real edits)

Make a new change via Claude (edit existing file or create a new one), then end the turn.

Now verify:

```bash
ls -la .bitloops/metadata
find .bitloops/metadata -maxdepth 3 -type f | sort
git branch --list 'bitloops/*'
```

Expected:

- `.bitloops/metadata/<session-id>/` contains:
  - `full.jsonl`
  - `prompt.txt`
  - `summary.txt`
  - `context.md`
- shadow branch exists: `bitloops/<head7...>` (or worktree-suffixed variant)

## 7) Commit again and verify checkpoints branch

```bash
git add .
git commit -m "second commit"
git branch --list 'bitloops/checkpoints/v1'
git log --oneline --decorate --graph --all | head -n 80
```

Expected:

- `bitloops/checkpoints/v1` exists
- checkpoint commits appear in graph/log

## 8) Push flow

```bash
git remote add origin <your-remote-url>
git branch -M main
git push -u origin main
```

Expected:

- main branch push succeeds
- pre-push hook may also push `bitloops/checkpoints/v1` when it exists

## 9) If push appears stuck

Check these first:

```bash
ssh -T git@github.com
GIT_TRACE=1 GIT_SSH_COMMAND='ssh -v' git push -u origin main
```

Common causes:

- SSH auth/host-key prompt waiting for input
- Network/auth issue to remote

Current Rust hook behavior:

- pre-push push for checkpoints uses `--no-verify` to avoid recursive hook loops
- hook failures are non-blocking warnings
