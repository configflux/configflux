#!/usr/bin/env bash
# tools/build_release.sh
#
# Build the static-musl Linux release binaries locally for one or both
# target platforms. Mirrors the cross-compile step of
# .github/workflows/release.yml so maintainers can reproduce CI behavior
# before pushing a v-tag.
#
# This script does NOT sign, checksum, upload, or tag. Its only job is to
# produce the tarball payload under `dist/` so you can inspect the
# binaries, run them, and diff them against a CI-built copy.
#
# Requirements:
#   - rustup with `x86_64-unknown-linux-musl` and/or
#     `aarch64-unknown-linux-musl` targets installed
#   - cargo-zigbuild  (cargo install --locked cargo-zigbuild)
#   - zig             (pip install ziglang, or a system `zig` on PATH)
#
# Usage:
#   tools/build_release.sh                 # build all supported platforms
#   tools/build_release.sh x86_64-linux    # build only one platform
#   tools/build_release.sh aarch64-linux
#
# Output:
#   dist/configflux-v<version>-<platform>/{compiler,interpreter,runtime,LICENSE,NOTICE}
#   dist/configflux-v<version>-<platform>.tar.gz
#
# Exit codes:
#   0  success
#   1  usage error or missing prerequisite
#   2  build failed
#
# Reference: docs/adrs/0009-binary-release-packaging.md

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: tools/build_release.sh [PLATFORM]

PLATFORM is one of:
  x86_64-linux      Build only x86_64-unknown-linux-musl
  aarch64-linux     Build only aarch64-unknown-linux-musl
  (omitted)         Build both

See docs/adrs/0009-binary-release-packaging.md for the full decision record.
EOF
}

# ---------------------------------------------------------------------------
# Platform mapping: user-facing name -> rust target triple
# ---------------------------------------------------------------------------
declare -a ALL_PLATFORMS=(x86_64-linux aarch64-linux)
target_for() {
  case "$1" in
    x86_64-linux)  echo "x86_64-unknown-linux-musl" ;;
    aarch64-linux) echo "aarch64-unknown-linux-musl" ;;
    *) echo "" ;;
  esac
}

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------
case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
  "")
    PLATFORMS=("${ALL_PLATFORMS[@]}")
    ;;
  x86_64-linux|aarch64-linux)
    PLATFORMS=("$1")
    ;;
  *)
    echo "error: unknown platform '$1'" >&2
    usage >&2
    exit 1
    ;;
esac

# ---------------------------------------------------------------------------
# Prerequisite checks
# ---------------------------------------------------------------------------
if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "error: cargo-zigbuild not found on PATH" >&2
  echo "       install with: cargo install --locked cargo-zigbuild" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found on PATH" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Resolve version (from VERSION file, or 0.0.0-dev if missing)
# ---------------------------------------------------------------------------
if [ -f "${ROOT}/VERSION" ]; then
  VERSION="$(tr -d '[:space:]' < "${ROOT}/VERSION")"
else
  VERSION="0.0.0-dev"
fi

# ---------------------------------------------------------------------------
# Build loop
# ---------------------------------------------------------------------------
cd "${ROOT}"
mkdir -p dist

for platform in "${PLATFORMS[@]}"; do
  target="$(target_for "${platform}")"
  if [ -z "${target}" ]; then
    echo "error: no rust target mapping for platform '${platform}'" >&2
    exit 1
  fi

  echo "==> Building ${platform} (${target})"

  # Ensure rustup has the target installed (idempotent; requires rustup)
  if command -v rustup >/dev/null 2>&1; then
    rustup target add "${target}" >/dev/null 2>&1 || true
  fi

  cargo zigbuild --release --locked \
    --target "${target}" \
    --package compiler \
    --package interpreter \
    --package runtime

  stage_dir="configflux-v${VERSION}-${platform}"
  stage_path="dist/${stage_dir}"
  rm -rf "${stage_path}"
  mkdir -p "${stage_path}"

  for bin in compiler interpreter runtime; do
    cp "target/${target}/release/${bin}" "${stage_path}/${bin}"
    chmod +x "${stage_path}/${bin}"
  done

  cp LICENSE "${stage_path}/LICENSE"
  if [ -f NOTICE ]; then
    cp NOTICE "${stage_path}/NOTICE"
  fi

  (
    cd dist
    tar --sort=name \
        --owner=0 --group=0 --numeric-owner \
        -czf "${stage_dir}.tar.gz" "${stage_dir}"
    sha256sum "${stage_dir}.tar.gz" > "${stage_dir}.tar.gz.sha256"
  )

  echo "==> Built dist/${stage_dir}.tar.gz"
done

echo
echo "Done. Artifacts under dist/:"
ls -1 dist/*.tar.gz 2>/dev/null || true
