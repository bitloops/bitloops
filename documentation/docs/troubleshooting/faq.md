---
sidebar_position: 2
title: FAQ
---

# FAQ

## Setup

### Can I use Bitloops with a monorepo?

Yes. Run `bitloops init` at the root of your git repository. DevQL will index all supported files in the repo.

### Can I initialize multiple agents in the same project?

Yes. Run `bitloops init --agent <name>` for each one. Sessions from each agent are tracked independently and each checkpoint records which agent was used.

### Does Bitloops modify my git history?

No. Bitloops adds data to the `.bitloops/` directory and installs git hooks (post-commit), but it never modifies existing commits, branches, or git objects.

### What happens if I don't enable Bitloops — does init do anything harmful?

No. `bitloops init` creates the `.bitloops/` directory and installs hook scripts, but nothing is captured until you run `bitloops enable`.

## Usage

### How much disk space does Bitloops use?

It depends on your codebase size and session frequency. The SQLite and DuckDB databases are typically small (a few MB). Session transcripts are the largest component — a long Claude Code session can produce several hundred KB of transcript data.

### Does Bitloops slow down git operations?

Negligibly. The post-commit hook runs asynchronously and typically completes in milliseconds. You won't notice a difference.

### What happens to Bitloops data when I rebase or squash commits?

Checkpoints are linked to commit SHAs. If you rewrite history (rebase, squash, amend), the old checkpoints will reference commits that no longer exist. The data isn't lost — it's still in `.bitloops/checkpoints/` — but the link to the current git history is broken. We recommend creating checkpoints on stable commits.

### Can I exclude certain files or directories from DevQL ingestion?

DevQL currently indexes all supported files in the repository. File-level exclusion is on the roadmap.

## Privacy & Security

### Does Bitloops phone home?

Only if you opt in to telemetry during `bitloops init`. When enabled, only anonymous command-level usage events are sent. No code, file names, or content is ever transmitted. Disable anytime in `.bitloops/settings.json`.

### Is it safe to commit `.bitloops/config.json` to a public repo?

Yes, as long as you use environment variable interpolation (`${GITHUB_TOKEN}`) for secrets instead of hardcoding them. The config file is designed to be committed safely.

### Can my CI pipeline use Bitloops?

Yes. Install Bitloops in CI, run `bitloops devql init && bitloops devql ingest`, and your pipeline can query the knowledge graph. For shared results, configure PostgreSQL or ClickHouse as backends.

## Troubleshooting

### `bitloops status` shows "Session: active" but I'm not using an agent

The session may be stuck. Run:

```bash
bitloops doctor
```

If doctor confirms a stuck session:

```bash
bitloops reset
```

### Hooks don't seem to fire for my agent

1. Verify capture is enabled: `bitloops status`
2. Reinstall hooks: `bitloops init --agent <name> --force`
3. Check that the agent is actually using its hook system (some agents require specific configuration)

### DevQL queries return empty results

Make sure you've ingested:

```bash
bitloops devql ingest
```

If ingestion succeeds but queries are empty, check the language — DevQL currently supports Rust, TypeScript, and JavaScript only.
