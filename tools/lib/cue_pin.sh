#!/usr/bin/env bash
# tools/lib/cue_pin.sh
#
# Single, sourced helper that reads the pinned `cue` toolchain coordinates out
# of tools/cue_toolchain.bzl — the repo's single source of truth for the CUE
# evaluator pin (CUE_VERSION + the per-platform sha256 of the official upstream
# release archive).
#
# ADR-0033 Decision 1 requires the toolchain container image to bundle the SAME
# cue as the host build, reading "that same pin rather than introducing a
# parallel version". tools/build_image.sh sources this helper so the cue it
# fetches and bakes into the image is provably the bzl-pinned one (version, URL,
# and sha256). Keeping the parse in one place — covered by the hermetic
# tools/cue_pin_test.sh — means the image build cannot silently drift from the
# Bazel pin or trust an unverified download.
#
# This file is meant to be sourced; it defines functions and runs nothing on
# load. Each function takes the path to cue_toolchain.bzl as its first argument
# so it is testable against a committed fixture with no Bazel/runfiles coupling.
#
# Functions:
#   cue_pin_version <bzl>                 -> echoes CUE_VERSION (e.g. 0.16.1)
#   cue_pin_sha256  <bzl> <platform_key>  -> echoes the 64-hex sha256 for the
#                                            given _CUE_ARCHIVES key
#                                            (e.g. linux_amd64)
#   cue_pin_url     <bzl> <platform_key>  -> echoes the official cue-lang/cue
#                                            release asset URL for the pin
#
# All functions fail (non-zero, message on stderr) on a missing file, an
# unparseable pin, or an unknown platform key — there is no silent empty pin.

# Read CUE_VERSION (the `CUE_VERSION = "x.y.z"` assignment) from the bzl.
cue_pin_version() {
  local bzl="${1:?cue_pin_version: bzl path required}"
  [[ -f "${bzl}" ]] || { echo "cue_pin_version: no such file: ${bzl}" >&2; return 1; }
  local v
  v="$(grep -E '^CUE_VERSION[[:space:]]*=' "${bzl}" \
       | sed -E 's/.*"([^"]+)".*/\1/' | head -n1)"
  if [[ -z "${v}" ]]; then
    echo "cue_pin_version: could not parse CUE_VERSION from ${bzl}" >&2
    return 1
  fi
  printf '%s\n' "${v}"
}

# Read the sha256 for a platform key (e.g. linux_amd64) out of the
# _CUE_ARCHIVES dict. The dict lines look like:
#     "linux_amd64": "….64hex…",
cue_pin_sha256() {
  local bzl="${1:?cue_pin_sha256: bzl path required}"
  local platform="${2:?cue_pin_sha256: platform key required}"
  [[ -f "${bzl}" ]] || { echo "cue_pin_sha256: no such file: ${bzl}" >&2; return 1; }
  local sha
  sha="$(grep -E "\"${platform}\"[[:space:]]*:" "${bzl}" \
         | grep -oE '[0-9a-f]{64}' | head -n1)"
  if [[ ! "${sha}" =~ ^[0-9a-f]{64}$ ]]; then
    echo "cue_pin_sha256: no sha256 for platform '${platform}' in ${bzl}" >&2
    return 1
  fi
  printf '%s\n' "${sha}"
}

# Construct the official upstream release asset URL for the pinned version and
# the given platform key. Mirrors _BASE_URL in cue_toolchain.bzl exactly:
#   https://github.com/cue-lang/cue/releases/download/v{V}/cue_v{V}_{platform}.tar.gz
cue_pin_url() {
  local bzl="${1:?cue_pin_url: bzl path required}"
  local platform="${2:?cue_pin_url: platform key required}"
  # Validate the platform key has a recorded digest before vending a URL, so a
  # typo'd key cannot produce a plausible-looking-but-unpinned download target.
  cue_pin_sha256 "${bzl}" "${platform}" >/dev/null || return 1
  local v
  v="$(cue_pin_version "${bzl}")" || return 1
  printf 'https://github.com/cue-lang/cue/releases/download/v%s/cue_v%s_%s.tar.gz\n' \
    "${v}" "${v}" "${platform}"
}
