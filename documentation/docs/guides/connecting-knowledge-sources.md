---
sidebar_position: 5
title: Connecting Knowledge Sources
---

# Connecting Knowledge Sources

Knowledge source setup is now split in two:

- Global daemon config stores provider credentials and endpoints
- Repo policy imports point the thin CLI at repo-specific knowledge definitions

## 1. Configure Provider Credentials

Add provider credentials to the global daemon config:

```toml title="config.toml"
[knowledge.providers.github]
token = "${GITHUB_TOKEN}"

[knowledge.providers.atlassian]
site_url = "https://example.atlassian.net"
email = "${ATLASSIAN_EMAIL}"
token = "${ATLASSIAN_TOKEN}"
```

## 2. Create A Repo Knowledge File

```toml title="bitloops/knowledge.toml"
[sources.github]
repositories = ["bitloops/bitloops"]
labels = ["devql", "documentation"]

[sources.atlassian]
spaces = ["ENG", "DOCS"]
projects = ["BIT"]
```

## 3. Import It From Repo Policy

```toml title=".bitloops.toml"
[imports]
knowledge = ["bitloops/knowledge.toml"]
```

## 4. Ingest Knowledge

```bash
bitloops devql knowledge ingest github
bitloops devql knowledge ingest atlassian
```

## Notes

- Imported knowledge files resolve relative to the policy file that declares them
- Repo policy affects the config fingerprint sent by the CLI
- Provider authentication remains global and should not be committed to repo policy
