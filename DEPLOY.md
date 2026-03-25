# Deploy (Phase A, Non-Brew)

This is the minimal release flow for developers.

## 1. Ship code to `main`

1. Open a PR with your changes.
2. Wait for CI to pass (`.github/workflows/ci.yml`).
3. Merge to `main`.

Do **not** bump the CLI version in random feature PRs unless that PR is intended to be the release cut.

## 2. Decide to cut a release

Cut a release only when:

- Desired PRs are already merged to `main`
- `main` is green
- You want a new public binary version

## 3. Bump version and create tag

Create a release PR that bumps `bitloops/Cargo.toml` to `X.Y.Z`.

1. Open PR (example title: `chore: release vX.Y.Z`).
2. Wait for CI.
3. Merge PR to `main`.

## 4. Create and push release tag

From a clean, up-to-date local `main`:

```bash
git checkout main
git pull --ff-only origin main
./scripts/release.sh
```

What the script does:

- Reads `bitloops/Cargo.toml` version
- Creates tag `vX.Y.Z`
- Pushes the tag only (never pushes `main`)

## 5. Observe release pipeline

Watch `.github/workflows/release.yml` for the tag run.

Success criteria:

- GitHub Release is created
- Artifacts are attached:
  - `bitloops-aarch64-apple-darwin.tar.gz`
  - `bitloops-aarch64-unknown-linux-musl.tar.gz`
  - `bitloops-x86_64-apple-darwin.tar.gz`
  - `bitloops-x86_64-unknown-linux-musl.tar.gz`
  - `bitloops-aarch64-pc-windows-msvc.zip`
  - `bitloops-x86_64-pc-windows-msvc.zip`
  - `checksums-sha256.txt`
- Verify job passes (downloads assets from release, checksums, Linux smoke run)

## 6. Quick install checks

macOS/Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/bitloops/bitloops/main/scripts/install.sh | bash
bitloops --version
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/bitloops/bitloops/main/scripts/install.ps1 | iex
bitloops --version
```

Windows (CMD):

```cmd
curl -fsSL https://raw.githubusercontent.com/bitloops/bitloops/main/scripts/install.cmd -o install.cmd && install.cmd && del install.cmd
bitloops --version
```

## 7. Rollback rule

If release is bad:

1. Delete GitHub Release + tag
2. Fix forward
3. Publish a new patch tag (for example `vX.Y.(Z+1)`)
