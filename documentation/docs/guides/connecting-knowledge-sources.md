---
sidebar_position: 5
title: Connecting Knowledge Sources
---

# Connecting Knowledge Sources

Knowledge source setup is split across:

- Global daemon config stores provider credentials and endpoints
- The CLI workflow that adds, associates, refreshes, and inspects repository-scoped knowledge
- Optional repo-policy imports for shared team configuration

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

## 2. Add Knowledge By URL

```bash
bitloops devql knowledge add https://github.com/bitloops/bitloops/issues/42
bitloops devql knowledge add https://bitloops.atlassian.net/browse/CLI-1370 --commit <sha>
```

The current knowledge workflow starts from URLs that Bitloops can resolve through configured providers.

## 3. Associate Knowledge To Code Or Other Knowledge

```bash
bitloops devql knowledge associate <knowledge_ref> --to commit:HEAD
bitloops devql knowledge associate <knowledge_ref> --to artefact:<artefact_id>
bitloops devql knowledge associate <knowledge_ref> --to knowledge:<other_item_id>
```

## 4. Refresh And Inspect Versions

```bash
bitloops devql knowledge refresh <knowledge_ref>
bitloops devql knowledge versions <knowledge_ref>
```

## 5. Optional: Share Imported Knowledge Config In Repo Policy

Use repo-policy imports when you want team-shared knowledge declarations on top of the CLI workflow:

```toml title="bitloops/knowledge.toml"
[sources.github]
repositories = ["bitloops/bitloops"]
labels = ["devql", "documentation"]

[sources.atlassian]
spaces = ["ENG", "DOCS"]
projects = ["BIT"]
```

```toml title=".bitloops.toml"
[imports]
knowledge = ["bitloops/knowledge.toml"]
```

## Notes

- Imported knowledge files resolve relative to the policy file that declares them
- Repo policy affects the config fingerprint sent by the CLI
- Provider authentication remains global and should not be committed to repo policy
