#!/usr/bin/env bash

set -euo pipefail

GITHUB_REPO="${GITHUB_REPO:-your-org/bitloops-cli}"
INSTALL_DIR="${BITLOOPS_INSTALL_DIR:-$HOME/.local/bin}"
BIN_NAME="bitloops"

log() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'Warning: %s\n' "$1"
}

die() {
  printf 'Error: %s\n' "$1" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

detect_os() {
  case "$(uname -s)" in
    Darwin) echo "apple-darwin" ;;
    Linux) echo "unknown-linux-musl" ;;
    *) die "unsupported operating system: $(uname -s)" ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64" ;;
    arm64|aarch64) echo "aarch64" ;;
    *) die "unsupported architecture: $(uname -m)" ;;
  esac
}

latest_tag() {
  local url="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
  local tag
  tag="$(curl -fsSL "$url" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  [[ -n "$tag" ]] || die "failed to detect latest release tag from ${url}"
  echo "$tag"
}

main() {
  require_cmd curl
  require_cmd tar

  local os arch tag target asset url tmp_dir archive install_path
  os="$(detect_os)"
  arch="$(detect_arch)"
  tag="$(latest_tag)"

  target="${arch}-${os}"
  asset="${BIN_NAME}-${target}.tar.gz"
  url="https://github.com/${GITHUB_REPO}/releases/download/${tag}/${asset}"

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT
  archive="${tmp_dir}/${asset}"
  install_path="${INSTALL_DIR}/${BIN_NAME}"

  log "Installing ${BIN_NAME} ${tag} for ${target}"
  log "Downloading ${asset}"
  curl -fsSL "$url" -o "$archive" || die "failed to download ${url}"

  log "Extracting archive"
  tar -xzf "$archive" -C "$tmp_dir" || die "failed to extract archive"
  [[ -f "${tmp_dir}/${BIN_NAME}" ]] || die "archive did not contain ${BIN_NAME} binary"

  mkdir -p "$INSTALL_DIR"
  mv "${tmp_dir}/${BIN_NAME}" "$install_path"
  chmod +x "$install_path"

  "$install_path" version >/dev/null || die "installed binary failed to execute"
  log "Installed to ${install_path}"

  # Installer parity hook: run post-install completion onboarding best-effort.
  if ! "$install_path" curl-bash-post-install; then
    warn "post-install completion setup skipped"
  fi

  if ! command -v "$BIN_NAME" >/dev/null 2>&1; then
    warn "${INSTALL_DIR} is not on PATH. Add it to your shell config."
  fi
}

main "$@"

