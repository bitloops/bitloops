---
sidebar_position: 2
title: Dashboard Local HTTPS Setup
---

# Dashboard Local HTTPS Setup

This guide configures local HTTPS for the Bitloops daemon and dashboard.

## 1. Prepare Local Certificates

Create or install the local certificate material required by your operating system and browser. Bitloops does not generate certificate authorities for you.

## 2. Enable The TLS Hint

Persist the local HTTPS hint in the daemon config:

```toml title="config.toml"
[dashboard.local_dashboard]
tls = true
```

This tells Bitloops that HTTPS should be preferred for loopback dashboard traffic when the local setup is already in place.

## 3. Recheck The Local Network Setup

```bash
bitloops daemon start --recheck-local-dashboard-net
```

Or open the dashboard directly:

```bash
bitloops dashboard
```

## 4. Force HTTP If Needed

If you want a local HTTP run instead:

```bash
bitloops daemon start --http --host 127.0.0.1
```

## Notes

- Dashboard bundle assets belong in the cache directory by default
- HTTPS hints belong in the global daemon config, not in repo policy
- `bitloops dashboard` is a launcher, not the long-lived server command
