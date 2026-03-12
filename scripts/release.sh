#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh
#
# Reads the version from bitloops_cli/Cargo.toml on main,
# creates tag vX.Y.Z, and pushes the tag only.
# Version bump must already be merged via PR.

CARGO="bitloops_cli/Cargo.toml"

# Read version from Cargo.toml
BARE=$(grep '^version' "$CARGO" | head -1 | sed 's/version = "\(.*\)"/\1/')
VERSION="v$BARE"

if [[ -z "$BARE" ]]; then
  echo "Error: could not read version from $CARGO"
  exit 1
fi

if [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Error: version in $CARGO must be semver (e.g. 1.2.3), got: $BARE"
  exit 1
fi

echo "Version: $VERSION (from $CARGO)"

# Must be on main and exactly synced with origin/main
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" ]]; then
  echo "Error: releases must be cut from main (currently on '$BRANCH')"
  exit 1
fi

git fetch origin main
LOCAL_HEAD=$(git rev-parse HEAD)
REMOTE_MAIN=$(git rev-parse origin/main)
if [[ "$LOCAL_HEAD" != "$REMOTE_MAIN" ]]; then
  echo "Error: local main is not exactly at origin/main — pull/reset to origin/main first"
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Error: working tree is not clean — commit/stash/discard changes first"
  exit 1
fi

# Tag must not already exist (local or remote)
if git rev-parse "$VERSION" &>/dev/null; then
  echo "Error: local tag $VERSION already exists"
  exit 1
fi

if git ls-remote --exit-code --tags origin "refs/tags/$VERSION" >/dev/null 2>&1; then
  echo "Error: remote tag $VERSION already exists"
  exit 1
fi

git tag "$VERSION"

echo ""
echo "Pushing tag $VERSION to origin..."
git push origin "$VERSION" --no-verify

echo ""
echo "Done. Watch the build at:"
echo "  https://github.com/$(git remote get-url origin | sed 's|.*github.com[:/]||;s|\.git$||')/actions"
