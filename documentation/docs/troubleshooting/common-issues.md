---
sidebar_position: 1
title: Common Issues
---

# Common Issues

Solutions for frequently encountered problems with Bitloops.

## Session Appears Stuck

**Symptom**: `bitloops checkpoints status` shows an active session that should have ended.

**Solution**: Run the doctor command to diagnose:

```bash
bitloops doctor
```

If the session is genuinely stuck, reset the state:

```bash
bitloops reset
```

This clears the shadow state without deleting checkpoint data.

## Hooks Not Firing

**Symptom**: You're using an AI agent but `bitloops checkpoints status` shows no session activity.

**Possible causes**:

1. **Bitloops not enabled** — run `bitloops enable`
2. **Hooks not installed** — reinitialize: `bitloops init --agent <name> --force`
3. **Agent not detected** — specify the agent explicitly: `bitloops init --agent claude-code`

## DevQL Ingestion Fails

**Symptom**: `bitloops devql ingest` errors out.

**Steps**:

1. Check store connectivity:
   ```bash
   bitloops --connection-status
   ```

2. Re-initialize the schema:
   ```bash
   bitloops devql init
   ```

3. If using PostgreSQL or ClickHouse, verify the connection string in `.bitloops/config.json`

## Dashboard Won't Start

**Symptom**: `bitloops dashboard` fails or the page doesn't load.

**Possible causes**:

1. **Port already in use** — start the daemon on a different port:
   ```bash
   bitloops daemon start --port 8080
   ```

2. **Stores not initialized** — run `bitloops devql init` first

3. **Local HTTPS/hostname not configured** — follow [Dashboard Local HTTPS Setup](/guides/dashboard-local-https-setup)

4. **Check daemon status**:
   ```bash
   bitloops status
   ```

## No Checkpoints After Committing

**Symptom**: You committed code from an AI session but no checkpoint was created.

**Check**:

1. Is capture enabled? `bitloops checkpoints status`
2. Are git hooks installed? Check `.git/hooks/` for Bitloops hooks
3. Reinitialize if needed: `bitloops init --agent <name> --force`

## Orphaned Data

**Symptom**: Storage grows unexpectedly or data seems inconsistent.

**Solution**: Clean up orphaned data:

```bash
bitloops clean
```

This removes data that's no longer linked to valid sessions or checkpoints.

## Getting More Help

If these solutions don't resolve your issue:

1. Run `bitloops doctor` for a comprehensive diagnostic
2. Check the [FAQ](/troubleshooting/faq) for common questions
3. Open an issue on [GitHub](https://github.com/bitloops/bitloops/issues)
4. Join the [Discord community](https://discord.com/invite/vj8EdZx8gK)
