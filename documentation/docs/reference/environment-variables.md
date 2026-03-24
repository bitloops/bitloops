---
sidebar_position: 3
title: Environment Variables
---

# Environment Variables

Bitloops uses environment variables for secrets, overrides, and build-time configuration.

## Runtime Variables

These are resolved at runtime when Bitloops reads your configuration.

### Knowledge Provider Credentials

| Variable | Used By | Description |
|----------|---------|-------------|
| `GITHUB_TOKEN` | GitHub knowledge provider | Personal access token for GitHub API |
| `ATLASSIAN_EMAIL` | Jira / Confluence providers | Email for Atlassian API authentication |
| `ATLASSIAN_TOKEN` | Jira / Confluence providers | API token for Atlassian services |

### Semantic Provider

| Variable | Used By | Description |
|----------|---------|-------------|
| `OPENAI_API_KEY` | Semantic embeddings | API key for embedding generation |

### Storage Credentials

AWS and GCS credentials are resolved from your standard cloud provider environment:

- **AWS S3**: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`, or AWS CLI profiles
- **Google Cloud Storage**: Application default credentials or `GOOGLE_APPLICATION_CREDENTIALS`

## Bitloops Override Variables

| Variable | Description |
|----------|-------------|
| `BITLOOPS_DASHBOARD_MANIFEST_URL` | Override the dashboard bundle manifest URL |
| `BITLOOPS_DASHBOARD_CDN_BASE_URL` | Override the dashboard CDN base URL |

These are advanced overrides typically used for development or custom deployments.

## Using Variables in Configuration

Reference environment variables in `.bitloops/config.json` using the `${VAR_NAME}` syntax:

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

This lets you commit your configuration to git without exposing secrets.
