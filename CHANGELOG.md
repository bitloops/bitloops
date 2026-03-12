# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Dashboard `/api/commits` now returns all checkpoint session agents via `checkpoint.agents` and no longer exposes a singular `checkpoint.agent` value.
- Dashboard `/api/commits` now includes `checkpoint.first_prompt_preview` with the first 160 characters from the first prompt of the first checkpoint session, after stripping leading `<tag>...</tag>` blocks and trimming leading whitespace.
- Dashboard agent filtering/aggregation now evaluates all session agents per checkpoint, so `/api/commits` agent filters, `/api/agents`, and KPI agent counts reflect multi-session checkpoints correctly.
- Dashboard `files_touched` payloads in `/api/commits` (`commit.files_touched` and `checkpoint.files_touched`) and `/api/checkpoints/{checkpoint_id}` now return arrays of objects (`[{ filepath, additionsCount, deletionsCount }]`) instead of path-keyed maps or plain path arrays.
- DevQL database support has been extended with provider-based backends: relational storage now supports `sqlite` (default) or `postgres`, and events storage now supports `duckdb` (default) or `clickhouse`.
- Local DevQL setup now works out of the box with file-based defaults (`~/.bitloops/devql/relational.db` and `~/.bitloops/devql/events.duckdb`), reducing external database dependencies for local development.
- Existing PostgreSQL/ClickHouse configurations remain backward compatible via legacy `postgres_dsn` / `clickhouse_*` config keys and `BITLOOPS_DEVQL_*` environment variables.

## [0.0.9] - 2026-03-09

- Reissued release after rollback of v0.0.8

## [0.0.8] - 2026-03-09

### Added

- Native Windows CMD installer at `scripts/install.cmd` with GitHub Releases download and SHA256 verification.
- Windows ARM64 (`aarch64-pc-windows-msvc`) release artifacts and installer support.
- DevQL query history injest in agent pre-hook.
- Added workflow to protect main branch from merges of other than develop branches.
