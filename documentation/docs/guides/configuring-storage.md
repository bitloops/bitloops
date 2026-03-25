---
sidebar_position: 8
title: Configuring Storage
---

# Configuring Storage

Bitloops works out of the box with bundled SQLite and DuckDB. This guide covers how to configure alternative backends for teams or production environments.

## PostgreSQL (Relational Store)

Replace SQLite with PostgreSQL for shared access or larger datasets.

### Configuration

In `.bitloops/config.json`:

```json
{
  "stores": {
    "relational": {
      "provider": "postgres",
      "postgres_dsn": "postgres://user:password@localhost:5432/bitloops"
    }
  }
}
```

### Setup

1. Create a PostgreSQL database
2. Set the DSN in your config (or use environment variables: `"postgres_dsn": "${BITLOOPS_PG_DSN}"`)
3. Run `bitloops devql init` to create the schema
4. Verify with `bitloops --connection-status`

## ClickHouse (Event Store)

Replace DuckDB with ClickHouse for high-volume analytics.

### Configuration

```json
{
  "stores": {
    "event": {
      "provider": "clickhouse",
      "clickhouse_url": "http://localhost:8123"
    }
  }
}
```

### Setup

1. Install and start ClickHouse
2. Set the URL in your config
3. Run `bitloops devql init` to create the schema
4. Verify with `bitloops --connection-status`

## AWS S3 (Blob Store)

Replace local filesystem with S3 for centralized storage.

### Configuration

```json
{
  "stores": {
    "blob": {
      "provider": "s3",
      "s3_bucket": "your-bitloops-bucket",
      "s3_region": "us-east-1"
    }
  }
}
```

AWS credentials are resolved from your environment (AWS CLI profile, environment variables, or IAM role).

## Google Cloud Storage (Blob Store)

### Configuration

```json
{
  "stores": {
    "blob": {
      "provider": "gcs",
      "gcs_bucket": "your-bitloops-bucket"
    }
  }
}
```

GCS credentials are resolved from your environment (application default credentials or service account).

## Mixed Configurations

You can mix and match backends. For example, keep SQLite for local development but use PostgreSQL in CI:

```json
{
  "stores": {
    "relational": { "provider": "postgres", "postgres_dsn": "${BITLOOPS_PG_DSN}" },
    "event": { "provider": "duckdb" },
    "blob": { "provider": "s3", "s3_bucket": "my-team-bitloops" }
  }
}
```

## Verifying Configuration

After changing storage configuration:

```bash
# Re-initialize schema for new backends
bitloops devql init

# Check connectivity
bitloops --connection-status
```
