# Dashboard Bundle URL Configuration

## Summary
Dashboard bundle URLs are now compiled into the binary at build time.

Source of truth for build values:
- Local/CI file: `config/dashboard_urls.json` (gitignored)
- Example template: `config/dashboard_urls.template.json` (tracked)

Generated at build time:
- `$OUT_DIR/dashboard_env.rs`

Consumed at runtime:
- `src/server/dashboard/bundle.rs`

## Motivation
1. Distributed binaries are self-contained and carry environment-correct dashboard URLs.
2. Runtime deployment does not need an external config file for these URLs.
3. CI can stamp different values for dev/stage/prod builds.

## Build Flow
1. Prepare `config/dashboard_urls.json` before `cargo build`.
2. `build.rs` validates URL fields and generates `dashboard_env.rs` in `OUT_DIR`.
3. `bundle.rs` includes generated constants and uses them as defaults.

## Runtime Precedence
1. `BITLOOPS_DASHBOARD_MANIFEST_URL` (if set and non-empty)
2. `BITLOOPS_DASHBOARD_CDN_BASE_URL` + `/bundle_versions.json` (if set and non-empty)
3. Compiled `DASHBOARD_MANIFEST_URL`
4. Compiled `DASHBOARD_CDN_BASE_URL` + `/bundle_versions.json`

For relative URLs in manifest entries:
1. `BITLOOPS_DASHBOARD_CDN_BASE_URL` (if set)
2. Compiled `DASHBOARD_CDN_BASE_URL`

## Validation Rules (`build.rs`)
1. Both fields must be present and non-empty strings.
2. Both fields must be valid `http://` or `https://` URLs.
3. Build fails fast with remediation instructions when invalid/missing.

## Operational Commands
Generate config from CI variables:
```bash
mkdir -p config
DASHBOARD_CDN_BASE_URL="https://cdn.example.com/bitloops-dashboard/" \
DASHBOARD_MANIFEST_URL="https://cdn.example.com/bitloops-dashboard/bundle_versions.json"

jq -n \
  --arg cdn "$DASHBOARD_CDN_BASE_URL" \
  --arg manifest "$DASHBOARD_MANIFEST_URL" \
  '{dashboard_cdn_base_url:$cdn,dashboard_manifest_url:$manifest}' \
  > config/dashboard_urls.json
```

Build:
```bash
cargo build --release
```
