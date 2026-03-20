# Knowledge Capability Manual Testing Guide

This guide explains how to manually test the DevQL knowledge capability with practical, copy-paste examples.

## 1. What This Capability Does

The knowledge capability lets you:

- ingest external knowledge (GitHub/Jira/Confluence URLs),
- version each knowledge item immutably,
- associate knowledge to targets (commit, knowledge item, checkpoint, artefact),
- refresh a source to create a new version when content changed,
- inspect versions and list repository knowledge through DevQL queries.

## 2. Command Reference

```bash
bitloops devql knowledge add <url> [--commit <sha_or_ref>]
bitloops devql knowledge associate <source_ref> --to <target_ref>
bitloops devql knowledge refresh <knowledge_ref>
bitloops devql knowledge versions <knowledge_ref>
bitloops devql query '<pipeline>'
```

## 3. Ref Grammar (Important)

### Source refs (`knowledge associate <source_ref>`)

- `knowledge:<item_id>`
  - resolves to latest source version
- `knowledge:<item_id>:<version_id>`
  - uses explicit source version
- `knowledge_version:<version_id>` (deprecated)
  - still supported as source compatibility syntax

### Target refs (`--to <target_ref>`)

- `commit:<sha_or_ref>` (for example `commit:HEAD`)
- `knowledge:<item_id>`
  - resolves to latest target version
- `knowledge:<item_id>:<version_id>`
  - uses explicit target version
- `checkpoint:<checkpoint_id>`
- `artefact:<artefact_uuid>`

Current behavior:

- If target version is omitted (`knowledge:<item_id>`), it is resolved to that target item's latest version.

## 4. Prerequisites

From your repository root:

```bash
bitloops init --agent codex
bitloops devql init
```

### 4.1 Required repository config

Create or update `<repo>/.bitloops/config.json` with:

- store backends (`stores.*`),
- knowledge provider credentials (`knowledge.providers.*`).

Minimal working example:

```json
{
  "stores": {
    "relational": {
      "provider": "sqlite",
      "sqlite_path": ".bitloops/stores/relational/relational.db"
    },
    "event": {
      "provider": "duckdb",
      "duckdb_path": ".bitloops/stores/event/events.duckdb"
    },
    "blob": {
      "provider": "local",
      "local_path": ".bitloops/stores/blob"
    }
  },
  "knowledge": {
    "providers": {
      "github": {
        "token": "${GITHUB_TOKEN}"
      },
      "atlassian": {
        "site_url": "https://bitloops.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      }
    }
  }
}
```

### 4.2 Provider configuration rules

- GitHub URLs (`github.com/.../issues/...`, `.../pull/...`) require:
  - `knowledge.providers.github.token`
- Jira and Confluence URLs can use:
  - shared Atlassian config under `knowledge.providers.atlassian`, or
  - product-specific overrides under `knowledge.providers.jira` and `knowledge.providers.confluence`

Example with Jira/Confluence overrides:

```json
{
  "knowledge": {
    "providers": {
      "atlassian": {
        "site_url": "https://bitloops.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      },
      "jira": {
        "site_url": "https://bitloops.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_JIRA_TOKEN}"
      },
      "confluence": {
        "site_url": "https://bitloops.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_CONFLUENCE_TOKEN}"
      }
    }
  }
}
```

### 4.3 Environment variables used by config interpolation

Set the env vars referenced above before running commands:

```bash
export GITHUB_TOKEN="..."
export ATLASSIAN_EMAIL="name@example.com"
export ATLASSIAN_TOKEN="..."
export ATLASSIAN_JIRA_TOKEN="..."          # optional if jira override is used
export ATLASSIAN_CONFLUENCE_TOKEN="..."    # optional if confluence override is used
```

### 4.4 Bootstrap sequence

After config is in place:

```bash
bitloops init --agent codex
bitloops devql connection-status
bitloops devql init
```

Then proceed with the manual knowledge commands in this guide.

## 5. Step-by-Step Manual Testing

### Step A: Add knowledge from URL

```bash
bitloops devql knowledge add "https://github.com/bitloops/bitloops/issues/42"
```

Expected output includes:

- `knowledge item: <id>`
- `knowledge item version: <version_id>`
- `status: ...`

Save the returned `knowledge item` and `knowledge item version` IDs for next steps.

### Step B: Add another knowledge item (target candidate)

```bash
bitloops devql knowledge add "https://bitloops.atlassian.net/browse/CLI-1370"
```

Save this second `knowledge item` and `knowledge item version` too.

### Step C: List repository knowledge with DevQL query

```bash
bitloops devql query 'repo("<repo-name>")->knowledge()->select(id,knowledge_item_version_id,title,source_kind,provider,updated_at)->limit(50)'
```

This is the easiest way to retrieve IDs again if you lost terminal output.

### Step D: Associate source to commit

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "commit:HEAD"
```

Explicit source version:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>:<source_version_id>" --to "commit:HEAD"
```

### Step E: Associate source to knowledge target (latest target version default)

Unversioned target (uses latest target version):

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "knowledge:<target_item_id>"
```

Explicit target version:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "knowledge:<target_item_id>:<target_version_id>"
```

Explicit source + explicit target:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>:<source_version_id>" --to "knowledge:<target_item_id>:<target_version_id>"
```

### Step F: Associate to checkpoint and artefact

Checkpoint:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "checkpoint:<checkpoint_id>"
```

Artefact:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "artefact:<artefact_uuid>"
```

### Step G: Inspect versions for a knowledge item

```bash
bitloops devql knowledge versions "knowledge:<item_id>"
```

or explicit item+version ref syntax:

```bash
bitloops devql knowledge versions "knowledge:<item_id>:<version_id>"
```

### Step H: Refresh a source and verify version changes

```bash
bitloops devql knowledge refresh "knowledge:<item_id>"
```

Expected output includes:

- latest knowledge item version id,
- `content changed: true/false`,
- `new version created: true/false`.

After refresh, verify version list:

```bash
bitloops devql knowledge versions "knowledge:<item_id>"
```

## 6. End-to-End Example Flow

```bash
# 1) Add source
bitloops devql knowledge add "https://github.com/bitloops/bitloops/issues/42"

# 2) Add target
bitloops devql knowledge add "https://bitloops.atlassian.net/browse/CLI-1370"

# 3) Inspect IDs (replace repo name)
bitloops devql query 'repo("<repo-name>")->knowledge()->select(id,knowledge_item_version_id,title,source_kind,provider)->limit(50)'

# 4) Associate source -> target (latest target version)
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "knowledge:<target_item_id>"

# 5) Associate source explicit version -> target explicit version
bitloops devql knowledge associate "knowledge:<source_item_id>:<source_version_id>" --to "knowledge:<target_item_id>:<target_version_id>"

# 6) Associate source -> commit
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "commit:HEAD"

# 7) Refresh and inspect versions
bitloops devql knowledge refresh "knowledge:<source_item_id>"
bitloops devql knowledge versions "knowledge:<source_item_id>"
```

## 7. How To Verify Target Version Resolution

When associating to an unversioned target ref:

```bash
bitloops devql knowledge associate "knowledge:<source_item_id>" --to "knowledge:<target_item_id>"
```

the target version is resolved to latest target version at association time.

To inspect persisted relation rows directly (SQLite default path), run:

```bash
sqlite3 .bitloops/stores/relational/relational.db \
'SELECT source_knowledge_item_version_id, target_type, target_id, target_knowledge_item_version_id, relation_type, association_method, created_at
 FROM knowledge_relation_assertions
 ORDER BY created_at DESC
 LIMIT 20;'
```

If your config uses a custom relational SQLite path, replace the DB path accordingly.

## 8. Troubleshooting

Unsupported URL:

- You will get an error similar to `unsupported knowledge URL`.

Missing or invalid target:

- Missing knowledge target: contains `target knowledge item ... not found`
- Invalid commit ref: contains `validating commit`
- Invalid checkpoint: checkpoint identifier validation error
- Invalid artefact id: UUID format validation error

Version mismatch errors:

- Source mismatch: `does not belong to knowledge item`
- Target mismatch: `target knowledge version ... does not belong to knowledge item ...`

## 9. Practical Notes

- Repeating the exact same resolved association is idempotent.
- Associations are version-aware. If latest target version changes later, running the same unversioned target association again can create a different relation assertion.
- Historical rows may still contain `NULL target_knowledge_item_version_id` from older behavior, but new unversioned target associations should store a concrete target version id.
