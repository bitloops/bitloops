---
sidebar_position: 7
title: Connecting Knowledge Sources
---

# Connecting Knowledge Sources

Bitloops can ingest context from external sources — GitHub, Jira, and Confluence — and associate it with your codebase. This gives AI agents access to the "why" behind your code: the requirements, discussions, and decisions that shaped it.

## GitHub

### Configuration

Add GitHub as a knowledge provider in `.bitloops/config.json`:

```json
{
  "knowledge": {
    "providers": {
      "github": {
        "token": "${GITHUB_TOKEN}"
      }
    }
  }
}
```

Set your GitHub token as an environment variable:

```bash
export GITHUB_TOKEN=ghp_your_token_here
```

### Ingesting Issues and PRs

```bash
# Ingest a specific issue
bitloops devql ingest --knowledge-url https://github.com/org/repo/issues/123

# Ingest a pull request
bitloops devql ingest --knowledge-url https://github.com/org/repo/pull/456
```

## Jira

### Configuration

```json
{
  "knowledge": {
    "providers": {
      "jira": {
        "site_url": "https://your-org.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      }
    }
  }
}
```

### Ingesting Jira Tickets

```bash
bitloops devql ingest --knowledge-url https://your-org.atlassian.net/browse/PROJ-123
```

## Confluence

### Configuration

```json
{
  "knowledge": {
    "providers": {
      "confluence": {
        "site_url": "https://your-org.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      }
    }
  }
}
```

### Ingesting Confluence Pages

```bash
bitloops devql ingest --knowledge-url https://your-org.atlassian.net/wiki/spaces/SPACE/pages/12345
```

## Environment Variable Interpolation

Notice the `${VAR_NAME}` syntax in the configuration. Bitloops resolves these at runtime from your environment variables. This keeps secrets out of your configuration file while allowing the config to be committed to git.

## Knowledge Versioning

Ingested knowledge is associated with a specific version of your codebase. This means:

- Knowledge stays relevant to the code it was captured with
- As your code evolves, you can re-ingest to keep context current
- AI agents get version-appropriate context, not stale information
