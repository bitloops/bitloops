---
title: Dashboard Local HTTPS Setup
---

# Dashboard Local HTTPS Setup

This guide configures local HTTPS for the Bitloops daemon and `bitloops dashboard` at `https://127.0.0.1:5667`.

It covers:

- Installing `mkcert` on macOS, Linux, and Windows
- Trusting the local development CA (`mkcert -install`)
- Configuring Bitloops dashboard local TLS hints

## 1) Install `mkcert`

### macOS

Homebrew:

```bash
brew install mkcert
brew install nss
```

Alternative options:

- MacPorts: `sudo port install mkcert`
- Manual binary: download `mkcert` for macOS and place it on your `PATH`

Then trust the local CA:

```bash
mkcert -install
```

### Linux

Homebrew:

```bash
brew install mkcert
brew install nss
```

Common distro packages:

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install -y mkcert libnss3-tools

# Fedora / RHEL
sudo dnf install -y mkcert nss-tools

# Arch
sudo pacman -S mkcert nss
```

Alternative option:

- Manual binary: download `mkcert` for Linux and place it on your `PATH`

Then trust the local CA:

```bash
mkcert -install
```

### Windows

If you use Homebrew in your shell environment:

```bash
brew install mkcert
```

Alternative options:

```powershell
# Chocolatey
choco install mkcert

# Scoop
scoop install mkcert
```

Or download `mkcert.exe` manually and add it to `PATH`.

Then trust the local CA (PowerShell or Command Prompt):

```powershell
mkcert -install
```

## 2) Optional dashboard config hints

When these values are set, Bitloops uses the configured HTTPS fast path (unless you pass `--recheck-local-dashboard-net` to `bitloops daemon start`):

```json title=".bitloops/config.json"
{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "dashboard": {
      "local_dashboard": {
        "tls": true
      }
    }
  }
}
```

Meaning:

- `dashboard.local_dashboard.tls: true` assumes local TLS material is already valid

## 3) Start the daemon or dashboard launcher

Open the dashboard via the launcher:

```bash
bitloops dashboard
```

Start the daemon explicitly:

```bash
bitloops daemon start
```

Force HTTP loopback (no TLS, explicit opt-in):

```bash
bitloops daemon start --http --host 127.0.0.1
```

Force a full local TLS recheck and refresh hints:

```bash
bitloops daemon start --recheck-local-dashboard-net
```

## 4) Verify and troubleshoot

Verify the daemon responds over local HTTPS:

```bash
curl -k https://127.0.0.1:5667/devql/sdl
```

If browser trust still fails:

1. Run `mkcert -install` again.
2. Fully quit and reopen your browser.
3. Re-run `bitloops daemon start --recheck-local-dashboard-net`.
