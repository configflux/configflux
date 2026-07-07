#!/usr/bin/env bash
# tools/generate_release_notes.sh
#
# Extract a single version's entry from CHANGELOG.md and emit a GitHub
# Release body in Markdown on stdout.
#
# Usage:
#     tools/generate_release_notes.sh [VERSION] > /tmp/release-notes.md
#     tools/generate_release_notes.sh                # reads VERSION file
#     tools/generate_release_notes.sh 0.1.0          # explicit, unprefixed
#     tools/generate_release_notes.sh v0.1.0         # explicit, prefixed
#     gh release create v0.1.0 --notes-file /tmp/release-notes.md
#
# Overrides (for tests / custom checkouts):
#     CHANGELOG_PATH=path/to/CHANGELOG.md  tools/generate_release_notes.sh 0.1.0
#     VERSION_FILE=path/to/VERSION         tools/generate_release_notes.sh
#
# Exit codes:
#     0  success; release notes written to stdout
#     1  generic failure (I/O error, bad args)
#     2  CHANGELOG.md not found
#     3  VERSION file missing and no arg given
#     4  requested version has no entry in CHANGELOG.md
#
# ---------------------------------------------------------------------------
# Release notes policy (ADR-0002 §3)
# ---------------------------------------------------------------------------
#
# CHANGELOG.md follows Keep a Changelog 1.1.0. Each released version lives
# under a heading of the exact shape:
#
#     ## [X.Y.Z] - YYYY-MM-DD
#
# and contains zero or more sub-sections from this ordered set:
#
#     ### Added
#     ### Changed
#     ### Deprecated
#     ### Removed
#     ### Fixed
#     ### Security
#
# Entries are user-facing sentences, not commit messages.
#
# What goes IN release notes (and therefore in CHANGELOG.md):
#   - User-facing behaviour changes (CLI flags, output format, defaults).
#   - Breaking changes (bump the minor on 0.x, major on >=1.0).
#   - New scenario packs / new public features.
#   - Public API changes (Rust crate API, C ABI, ROS2 topics/services).
#   - Interface version bumps (PRODUCT_SCHEMA_VERSION,
#     CONFIGFLUX_RUNTIME_C_ABI_VERSION_*) — call these out explicitly.
#   - Security fixes with a brief impact summary.
#   - Deprecations with the target removal version.
#
# What stays OUT of release notes:
#   - Internal refactors with no user-visible effect.
#   - Tooling-only changes (Bazel wiring, lint rules, CI tweaks).
#   - Test-only changes.
#   - Planning docs, ADRs, bd workflow changes.
#   - Typo fixes and comment-only edits.
#
# Version normalization (ADR-0002 §7):
#     The script accepts either `0.1.0` or `v0.1.0` as input and normalizes
#     internally to the unprefixed form when matching CHANGELOG headings.
#     The GitHub Release title it emits uses the prefixed form (`v0.1.0`).
#
# ---------------------------------------------------------------------------

set -euo pipefail

err() {
    printf 'generate_release_notes: %s\n' "$*" >&2
}

# Locate the workspace root. In a Bazel sh_binary run the environment
# variable BUILD_WORKSPACE_DIRECTORY is the right answer. Otherwise walk up
# from the script's own directory.
if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
    WORKSPACE_ROOT="${BUILD_WORKSPACE_DIRECTORY}"
else
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
fi

CHANGELOG_PATH="${CHANGELOG_PATH:-${WORKSPACE_ROOT}/CHANGELOG.md}"
VERSION_FILE="${VERSION_FILE:-${WORKSPACE_ROOT}/VERSION}"

# ---------------------------------------------------------------------------
# Resolve the requested version
# ---------------------------------------------------------------------------

raw_version="${1:-}"

if [[ -z "${raw_version}" ]]; then
    if [[ ! -f "${VERSION_FILE}" ]]; then
        err "no version argument given and VERSION file not found at ${VERSION_FILE}"
        exit 3
    fi
    raw_version="$(tr -d '[:space:]' < "${VERSION_FILE}")"
    if [[ -z "${raw_version}" ]]; then
        err "VERSION file ${VERSION_FILE} is empty"
        exit 3
    fi
fi

# Normalize: strip a leading v/V if present.
unprefixed="${raw_version#[vV]}"
prefixed="v${unprefixed}"

# Validate SemVer shape (X.Y.Z, digits only).
if ! [[ "${unprefixed}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    err "not a SemVer X.Y.Z version: '${raw_version}'"
    exit 1
fi

# ---------------------------------------------------------------------------
# Read the CHANGELOG
# ---------------------------------------------------------------------------

if [[ ! -f "${CHANGELOG_PATH}" ]]; then
    err "CHANGELOG not found at ${CHANGELOG_PATH}"
    exit 2
fi

# Locate and parse the heading line for this version. The heading regex is
# pinned by ADR-0002 §3 so a line-oriented match is safe.
heading_regex="^## \[${unprefixed//./\\.}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}\$"
heading_line="$(grep -E "${heading_regex}" "${CHANGELOG_PATH}" | head -1 || true)"

if [[ -z "${heading_line}" ]]; then
    err "no entry for version ${unprefixed} in ${CHANGELOG_PATH}"
    exit 4
fi

# Date is the token after " - ".
release_date="${heading_line##* - }"

# Extract the body: everything after the matching heading, up to (but not
# including) the next "## [" heading or EOF. Keep in-block "### ..." and
# deeper headings intact. Strip leading and trailing blank lines.
body="$(
    awk -v ver="${unprefixed}" '
        BEGIN { in_block = 0 }
        /^## \[[0-9]+\.[0-9]+\.[0-9]+\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$/ {
            if (in_block) { exit }
            cur = $2
            gsub(/[\[\]]/, "", cur)
            if (cur == ver) {
                in_block = 1
                next
            }
        }
        in_block && /^## \[/ { exit }
        in_block { print }
    ' "${CHANGELOG_PATH}" \
    | awk '
        # Strip leading blank lines, then print everything; a final pass
        # trims trailing blanks.
        BEGIN { started = 0 }
        { if (started || $0 != "") { lines[++n] = $0; started = 1 } }
        END {
            # Drop trailing blank lines.
            while (n > 0 && lines[n] == "") { n-- }
            for (i = 1; i <= n; i++) { print lines[i] }
        }
    '
)"

if [[ -z "${body}" ]]; then
    err "version ${unprefixed} heading found but body is empty in ${CHANGELOG_PATH}"
    exit 4
fi

# ---------------------------------------------------------------------------
# Emit the GitHub Release body
# ---------------------------------------------------------------------------

printf '# %s — %s\n\n' "${prefixed}" "${release_date}"
printf '%s\n' "${body}"
