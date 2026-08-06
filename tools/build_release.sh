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
#   tools/build_release.sh --stage-only    # stage layout + license text only,
#                                          # no cross-compile (binaries stubbed);
#                                          # used by build_release_test.sh
#   tools/build_release.sh --root DIR      # read VERSION + license text from DIR
#                                          # and write dist/ under it
#
# Output:
#   dist/configflux-v<version>-<platform>/{compiler,interpreter,runtime,cfx,LICENSE,NOTICE,LICENSING.md}
#   dist/configflux-v<version>-<platform>.tar.gz
#
# The three license files ship VERBATIM at the tarball root. LICENSE and NOTICE
# both cross-reference LICENSING.md by name, and LICENSING.md carries the
# dual-license summary + Change Date table + commercial-licensing contact, so
# all three are load-bearing; a missing or empty one fails the build rather than
# shipping a tarball with omitted/dangling license terms. The check is
# self-contained so it holds when run from the published OSS source tree too.
#
# Exit codes:
#   0  success
#   1  usage error or missing prerequisite
#   2  build failed, or a required license file (LICENSE, NOTICE, LICENSING.md)
#      is missing or empty at the repo root
#
# Reference: docs/adrs/0009-binary-release-packaging.md

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: tools/build_release.sh [PLATFORM] [--stage-only] [--root DIR]

PLATFORM is one of:
  x86_64-linux      Build only x86_64-unknown-linux-musl
  aarch64-linux     Build only aarch64-unknown-linux-musl
  (omitted)         Build both

Options:
  --stage-only      Assemble the tarball layout + license text WITHOUT cross-
                    compiling; the four binaries are stubbed. Lets the license
                    invariant be exercised with no rustup/zig/musl toolchain.
  --root DIR        Read VERSION and the license text from DIR (and write dist/
                    under it) instead of this script's own repo. Used by
                    build_release_test.sh to target a throwaway fixture.

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

# License text shipped verbatim at the tarball root (validated present and
# non-empty below; the build fails closed if any is absent). All three are
# load-bearing under the BUSL-1.1 dual license (ADR-0046) and none is redundant:
# LICENSE is the operative grant, NOTICE carries copyright + third-party
# attribution, and LICENSING.md holds the dual-license summary, the Change Date
# table and the commercial-licensing contact. LICENSE and NOTICE BOTH
# cross-reference LICENSING.md by name, so dropping it would ship two dangling
# pointers.
RELEASE_LICENSE_FILES=(LICENSE NOTICE LICENSING.md)

# ---------------------------------------------------------------------------
# Arg parsing (order-independent: [PLATFORM] [--stage-only] [--root DIR])
# ---------------------------------------------------------------------------
STAGE_ONLY=0
PLATFORM_ARG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)    usage; exit 0 ;;
    --stage-only) STAGE_ONLY=1; shift ;;
    --root)       ROOT="$(cd "${2:?--root needs a value}" && pwd)"; shift 2 ;;
    x86_64-linux|aarch64-linux)
      if [ -n "${PLATFORM_ARG}" ]; then
        echo "error: more than one platform given" >&2
        exit 1
      fi
      PLATFORM_ARG="$1"; shift ;;
    *)
      echo "error: unknown argument '$1'" >&2
      usage >&2
      exit 1 ;;
  esac
done

if [ -z "${PLATFORM_ARG}" ]; then
  PLATFORMS=("${ALL_PLATFORMS[@]}")
else
  PLATFORMS=("${PLATFORM_ARG}")
fi

# ---------------------------------------------------------------------------
# Prerequisite checks
# ---------------------------------------------------------------------------
# The cross-compile toolchain is only needed for a real build; --stage-only
# assembles the tarball layout + license text with no toolchain at all.
if [ "${STAGE_ONLY}" -eq 0 ]; then
  if ! command -v cargo-zigbuild >/dev/null 2>&1; then
    echo "error: cargo-zigbuild not found on PATH" >&2
    echo "       install with: cargo install --locked cargo-zigbuild" >&2
    exit 1
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found on PATH" >&2
    exit 1
  fi
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

# ---------------------------------------------------------------------------
# License-text validation (FAIL CLOSED, up front before the cross-compile).
#
# Every RELEASE_LICENSE_FILES entry must be present and non-empty at the repo
# root. A release tarball that ships binaries with omitted or dangling license
# terms is a licensing defect, not a cosmetic gap — so a missing or empty
# license file aborts the build here rather than shipping silently (the old code
# copied NOTICE only `if [ -f NOTICE ]` and never shipped LICENSING.md at all).
# The check is self-contained (no repo-internal tooling), so it holds identically
# when this script runs from the published OSS source tree (configflux-mmof).
# ---------------------------------------------------------------------------
for l in "${RELEASE_LICENSE_FILES[@]}"; do
  if [ ! -s "${ROOT}/${l}" ]; then
    echo "error: required license file '${l}' is missing or empty at ${ROOT}/${l}" >&2
    echo "       — refusing to ship a release tarball with omitted/dangling license terms" >&2
    exit 2
  fi
done

for platform in "${PLATFORMS[@]}"; do
  target="$(target_for "${platform}")"
  if [ -z "${target}" ]; then
    echo "error: no rust target mapping for platform '${platform}'" >&2
    exit 1
  fi

  echo "==> Building ${platform} (${target})"

  if [ "${STAGE_ONLY}" -eq 0 ]; then
    # Ensure rustup has the target installed (idempotent; requires rustup)
    if command -v rustup >/dev/null 2>&1; then
      rustup target add "${target}" >/dev/null 2>&1 || true
    fi

    cargo zigbuild --release --locked \
      --target "${target}" \
      --package compiler \
      --package interpreter \
      --package runtime \
      --package cfx
  else
    echo "    (stage-only: skipping cross-compile; binaries will be stubbed)"
  fi

  stage_dir="configflux-v${VERSION}-${platform}"
  stage_path="dist/${stage_dir}"
  rm -rf "${stage_path}"
  mkdir -p "${stage_path}"

  for bin in compiler interpreter runtime cfx; do
    if [ "${STAGE_ONLY}" -eq 0 ]; then
      cp "target/${target}/release/${bin}" "${stage_path}/${bin}"
    else
      # Stage-only: a mode-0755 stub stands in for the real binary so the
      # tarball layout + license invariant can be exercised without a build.
      printf '#!/bin/sh\necho "stub %s (stage-only build)" >&2\nexit 0\n' \
        "${bin}" > "${stage_path}/${bin}"
    fi
    chmod +x "${stage_path}/${bin}"
  done

  # License text: all three files, validated present up front, copied verbatim
  # to the tarball root. The post-copy non-empty assertion guards against a
  # future refactor silently dropping one (an empty license file is as much a
  # defect as an absent one).
  for l in "${RELEASE_LICENSE_FILES[@]}"; do
    cp "${ROOT}/${l}" "${stage_path}/${l}"
    [ -s "${stage_path}/${l}" ] || {
      echo "error: license file '${l}' empty after copy" >&2
      exit 2
    }
  done

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
