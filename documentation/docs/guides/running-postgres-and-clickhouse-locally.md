---
title: Running Postgres And ClickHouse Locally
---

# Running Postgres And ClickHouse Locally

Bitloops is local-first by default, so you do not need external databases for normal development. When you want to exercise the remote relational and event backends on your machine, the repository includes a local Docker Compose stack at `compose.db.yaml`.

This setup is useful when you want to:

- test the Postgres-backed relational store locally
- test the ClickHouse-backed event store locally
- experiment with a remote-store configuration without changing production infrastructure

## What This Compose File Starts

`compose.db.yaml` starts:

- PostgreSQL with `pgvector` on `localhost:5432`
- ClickHouse HTTP on `localhost:8123`
- ClickHouse native TCP on `localhost:9000`

The services use named Docker volumes, so data survives container restarts until you remove the volumes.

## 1. Start The Databases

From the repository root:

```bash
docker compose -f compose.db.yaml up -d
```

Check that both services are running:

```bash
docker compose -f compose.db.yaml ps
```

## 2. Create The Default Daemon Config

From the repository root, bootstrap the default daemon config and start the daemon:

```bash
bitloops start --create-default-config
```

To see which daemon config file Bitloops is using, run:

```bash
bitloops status
```

On macOS, the default path is typically:

```text
~/Library/Application Support/bitloops/config.toml
```

## 3. Update The Daemon Config

Edit the active daemon `config.toml`. Postgres and ClickHouse settings belong in the daemon config, not in `.bitloops.local.toml`.

Use:

```toml
[stores.relational]
postgres_dsn = "postgres://bitloops:bitloops@localhost:5432/bitloops"

[stores.events]
clickhouse_url = "http://localhost:8123"
clickhouse_user = "bitloops"
clickhouse_password = "bitloops"
clickhouse_database = "bitloops"
```

If you use `BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE`, or the repo has its own nearer `config.toml`, `bitloops status` shows the effective config file to edit.

Bitloops' Postgres-backed semantic embedding schema requires the `vector` extension, so this local Compose stack uses a PostgreSQL image that already includes `pgvector`.

If you already started an older version of this local ClickHouse stack without credentials, reset it once so the container reinitialises with the new user and database:

```bash
docker compose -f compose.db.yaml down -v
docker compose -f compose.db.yaml up -d
```

If you also started an older version of the Postgres container from a plain `postgres:16` image, the same reset recreates it from the `pgvector` image Bitloops needs.

## 4. Reload Bitloops And Verify The Connections

Restart the daemon after updating the config:

```bash
bitloops daemon restart
```

Then verify that Bitloops can see the configured backends:

```bash
bitloops --connection-status
bitloops status
```

When the config is active:

- relational store operations use Postgres through `stores.relational.postgres_dsn`
- event and analytics operations use ClickHouse through `stores.events.clickhouse_*`
- runtime SQLite remains local
- blob storage remains whatever you already configured locally

## 5. Initialise The Repository

Once the daemon has restarted with the config above, initialise the repository against that active daemon setup:

```bash
bitloops init --sync=true
```

If you want to skip the first sync for now, use:

```bash
bitloops init --sync=false
```

`bitloops init` creates or updates `.bitloops.local.toml` for repo-local policy, but it does not replace the daemon `config.toml` storage settings from the earlier steps.

## 6. Use The Stores In Bitloops

After `bitloops init`, Bitloops uses the configured services automatically for repo-scoped workflows.

Common follow-up flows include:

- `bitloops devql tasks enqueue --kind ingest` when you also want checkpoint, commit, and event history materialised
- `bitloops devql test-harness ...` commands when you want the test-harness storage path to use Postgres instead of the local SQLite fallback

Schema initialisation for the test-harness pack is handled by Bitloops during normal startup and command flows; you do not need to create those tables manually.

## 7. Stop Or Reset The Stack

Stop the containers but keep the data:

```bash
docker compose -f compose.db.yaml down
```

Stop the containers and delete the local database volumes:

```bash
docker compose -f compose.db.yaml down -v
```

Use the second command only when you want a clean local reset.
