#!/usr/bin/env bash
set -euo pipefail

REPO="bitloops/bitloops"
INSTALL_DIR="/usr/local/bin"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Error: required command not found: $1" >&2
    exit 1
  fi
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}:${arch}" in
    Darwin:x86_64|Darwin:amd64)
      printf "%s" "x86_64-apple-darwin"
      ;;
    Darwin:arm64|Darwin:aarch64)
      printf "%s" "aarch64-apple-darwin"
      ;;
    Linux:x86_64|Linux:amd64)
      printf "%s" "x86_64-unknown-linux-musl"
      ;;
    Linux:arm64|Linux:aarch64)
      echo "Error: Linux arm64 binaries are not published yet." >&2
      echo "Use Linux x86_64 for now." >&2
      exit 1
      ;;
    *)
      echo "Error: unsupported platform: ${os}/${arch}" >&2
      exit 1
      ;;
  esac
}

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    echo "Error: neither sha256sum nor shasum is available" >&2
    exit 1
  fi
}

require_cmd curl
require_cmd tar
require_cmd awk
require_cmd uname

TARGET="$(detect_target)"
ASSET_NAME="bitloops-${TARGET}.tar.gz"
CHECKSUMS_FILE="checksums-sha256.txt"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"

echo "Resolving latest release for ${REPO}..."
RELEASE_JSON="$(curl -fsSL -H "User-Agent: bitloops-installer" "$API_URL")"
TAG="$(printf "%s" "$RELEASE_JSON" | tr -d '\n' | sed -nE 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p')"

if [[ -z "${TAG}" ]]; then
  echo "Error: could not resolve latest release tag from ${API_URL}" >&2
  exit 1
fi

ASSET_URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET_NAME}"
CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${TAG}/${CHECKSUMS_FILE}"

echo "Downloading ${ASSET_NAME} (${TAG})..."
curl -fL "$ASSET_URL" -o "${TMP_DIR}/${ASSET_NAME}"
curl -fL "$CHECKSUMS_URL" -o "${TMP_DIR}/${CHECKSUMS_FILE}"

EXPECTED_HASH="$(awk -v name="$ASSET_NAME" '$2 == name {print $1; exit}' "${TMP_DIR}/${CHECKSUMS_FILE}")"
if [[ -z "${EXPECTED_HASH}" ]]; then
  echo "Error: checksum for ${ASSET_NAME} not found in ${CHECKSUMS_FILE}" >&2
  exit 1
fi

ACTUAL_HASH="$(sha256_file "${TMP_DIR}/${ASSET_NAME}")"
if [[ "${EXPECTED_HASH}" != "${ACTUAL_HASH}" ]]; then
  echo "Error: checksum mismatch for ${ASSET_NAME}" >&2
  echo "Expected: ${EXPECTED_HASH}" >&2
  echo "Actual:   ${ACTUAL_HASH}" >&2
  exit 1
fi

tar -xzf "${TMP_DIR}/${ASSET_NAME}" -C "${TMP_DIR}"
if [[ ! -f "${TMP_DIR}/bitloops" ]]; then
  echo "Error: extracted archive did not contain 'bitloops' binary" >&2
  exit 1
fi

if [[ ! -d "${INSTALL_DIR}" ]]; then
  mkdir -p "${INSTALL_DIR}" 2>/dev/null || true
fi

if [[ ! -w "${INSTALL_DIR}" ]]; then
  FALLBACK_DIR="${HOME}/.local/bin"
  mkdir -p "${FALLBACK_DIR}"
  echo "No write permission for ${INSTALL_DIR}; using ${FALLBACK_DIR}"
  INSTALL_DIR="${FALLBACK_DIR}"
fi

install -m 0755 "${TMP_DIR}/bitloops" "${INSTALL_DIR}/bitloops"

echo "Installed bitloops ${TAG} to ${INSTALL_DIR}/bitloops"
if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
  echo "Note: ${INSTALL_DIR} is not in PATH."
  echo "Add this to your shell profile:"
  echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
