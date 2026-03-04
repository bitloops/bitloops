# Releasing

1. Merge a PR that bumps `version` in `bitloops_cli/Cargo.toml`
2. From clean local `main`, run:

```bash
./scripts/release.sh
```

That's it — the script tags current `main` and pushes the tag. GitHub Actions builds all platform binaries and attaches them to the release automatically.

Release assets include a `checksums-sha256.txt` file used by install scripts to verify integrity.

Installers:
- macOS/Linux: `install.sh`
- Windows: `install.ps1`

See `DEPLOY.md` for release steps and `bitloops_cli/README.md` for distribution details.
