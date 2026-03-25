# Build (from the bitloops_cli directory)

cd /Users/markos/code/bitloops/cli/bitloops_cli

## Local dashboard HTTPS

The dashboard listens with **HTTPS** (rustls). Leaf certificates are generated with **[mkcert](https://github.com/FiloSottile/mkcert)** under `~/.bitloops/certs/<host-id>/`.

1. Install `mkcert` (e.g. macOS: `brew install mkcert nss`).
2. Run **`mkcert -install`** once so your OS trusts the local CA. Bitloops never runs this for you.
3. On each **`bitloops dashboard`** start you should see: `Checking mkcert local CA is trusted…` — Bitloops then runs a **throwaway** `mkcert` leaf in a temp dir (this is **not** `mkcert -install`). That matches what mkcert does when issuing certs: if the local CA is still missing from the trust store, mkcert prints its warning and Bitloops **exits** with instructions to run **`mkcert -install`** yourself (same fix as Chrome’s `ERR_CERT_AUTHORITY_INVALID`).
4. By default the CLI maps **`bitloops.local` → 127.0.0.1** in your hosts file when `--host` is omitted; the printed URL is **`https://bitloops.local:<port>`** (with a **localhost** fallback if the hosts file cannot be updated).

**Testing the trust error path:**

- **`mkcert -uninstall`** may print `The specified item could not be found in the keychain` — that means there was no mkcert root in the system keychain to remove (e.g. you never ran `mkcert -install` successfully on this Mac, or it was removed already). It is harmless.
- **Reliable approach:** point mkcert at a **fresh CA directory** so the OS does not trust it yet, then start the dashboard in the same shell:
  ```bash
  export CAROOT="$(mktemp -d)"
  bitloops dashboard
  ```
  You should see the check line, then Bitloops exits with the trust error. To fix mkcert for normal use, **unset `CAROOT`** (back to the default under `~/Library/.../mkcert` on macOS) and run **`mkcert -install`** again.
- **Alternative:** after a successful **`mkcert -uninstall`** on a machine where install had worked, run **`bitloops dashboard`** — same error until you run **`mkcert -install`** again.

**Advanced:** set **`BITLOOPS_DASHBOARD_SKIP_MKCERT_TRUST_PROBE=1`** to skip only the startup probe (not recommended; you may not see the trust error until mkcert regenerates certs).

The integration test `dashboard_bundle_lifecycle_e2e` is skipped when `mkcert` is missing or when the trust probe fails (e.g. CI without `mkcert -install`).

### Chrome: “Your connection is not private” / `ERR_CERT_AUTHORITY_INVALID`

The leaf cert is signed by mkcert’s **local CA**. Browsers only trust it after the CA is installed:

1. Run **`mkcert -install`** once (on macOS/Linux you may be prompted for your password).
2. **Fully quit Chrome** (not just close the window) and open it again so it reloads the trust store.

If it still fails, confirm `mkcert -install` completed without errors; on macOS you can open **Keychain Access** and check that the **mkcert** root certificate exists and is trusted.

If the terminal shows **`TLS handshake failed: … CertificateUnknown`** while the browser shows “not private”, that is the same problem: the browser is rejecting the server certificate until the mkcert CA is trusted (`mkcert -install`).

**`/etc/hosts` permission denied:** mapping `bitloops.local` needs write access to `/etc/hosts` (e.g. run Bitloops once with `sudo` for that step only, or edit hosts manually with `sudo`). Otherwise Bitloops falls back to **`https://localhost:<port>`**, which is fine for local use.

# Required once per environment: build-time dashboard URL config
cp config/dashboard_urls.template.json config/dashboard_urls.json
# edit config/dashboard_urls.json with real values
# build script validation runs during check/build
cargo check

cargo build

# Then run it from ANY directory

cd /path/to/some-other-repo
/Users/markos/code/bitloops/cli/bitloops_cli/target/debug/bitloops init
/Users/markos/code/bitloops/cli/bitloops_cli/target/debug/bitloops enable

# OR INSTEAD, BETTER

cargo install --path . --force

# Make sure cargo is in your PATH

# this will make the `bitloops` command available globally, so you can just run

bitloops --version

# Follow these steps

1. git init
2. create + commit a tiny initial file (README.md)
3. bitloops init
4. bitloops enable
5. chat with Claude (so hooks run and stop snapshots)
6. git commit → this one should include Bitloops-Checkpoint: ... and git branch -v ,show the checkpoints branch
