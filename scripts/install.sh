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

detect_platform() {
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
      printf "%s" "aarch64-unknown-linux-musl"
      ;;
    *)
      echo "Error: unsupported platform: ${os}/${arch}" >&2
      exit 1
      ;;
  esac
}

detect_archive_ext() {
  local os
  os="$(uname -s)"

  case "$os" in
    Darwin)
      printf "%s" "zip"
      ;;
    Linux)
      printf "%s" "tar.gz"
      ;;
    *)
      echo "Error: unsupported OS for archive selection: ${os}" >&2
      exit 1
      ;;
  esac
}

extract_archive() {
  local archive="$1"
  local destination="$2"
  local os

  os="$(uname -s)"

  case "$os" in
    Darwin)
      require_cmd ditto
      ditto -x -k "$archive" "$destination"
      ;;
    Linux)
      require_cmd tar
      tar -xzf "$archive" -C "$destination"
      ;;
    *)
      echo "Error: unsupported OS for extraction: ${os}" >&2
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
require_cmd awk
require_cmd uname

find_extracted_binary() {
  local name="$1"
  local direct_path package_path found_path

  direct_path="${TMP_DIR}/${name}"
  package_path="${TMP_DIR}/package/${name}"
  if [[ -f "${direct_path}" ]]; then
    printf "%s" "${direct_path}"
    return 0
  fi
  if [[ -f "${package_path}" ]]; then
    printf "%s" "${package_path}"
    return 0
  fi

  found_path="$(find "${TMP_DIR}" -type f -name "${name}" -print -quit 2>/dev/null || true)"
  if [[ -n "${found_path}" ]]; then
    printf "%s" "${found_path}"
    return 0
  fi

  return 1
}

TARGET="$(detect_platform)"
ARCHIVE_EXT="$(detect_archive_ext)"
ASSET_NAME="bitloops-${TARGET}.${ARCHIVE_EXT}"
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

extract_archive "${TMP_DIR}/${ASSET_NAME}" "${TMP_DIR}"

if ! EXTRACTED_BINARY="$(find_extracted_binary "bitloops")"; then
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

install -m 0755 "${EXTRACTED_BINARY}" "${INSTALL_DIR}/bitloops"

echo "Installed bitloops ${TAG} to ${INSTALL_DIR}/bitloops"
if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
  echo "Note: ${INSTALL_DIR} is not in PATH."
  echo "Add this to your shell profile:"
  echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
