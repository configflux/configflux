#!/usr/bin/env bash
# Tests for tools/generate_release_notes.sh.
#
# Each case runs the script against the synthetic fixture CHANGELOG under
# tools/testdata/ with CHANGELOG_PATH overridden, so the test is fully
# hermetic and independent of whether a real CHANGELOG.md exists at the
# workspace root (it does not today — see ADR-0002).
set -euo pipefail

if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
    WORKSPACE_ROOT="${BUILD_WORKSPACE_DIRECTORY}"
else
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
fi

# Under `bazel test`, srcs/data live in the runfiles tree rooted at PWD.
# Outside Bazel, they live in the workspace checkout.
if [[ -f "tools/generate_release_notes.sh" ]]; then
    SCRIPT="$(pwd)/tools/generate_release_notes.sh"
    FIXTURE="$(pwd)/tools/testdata/CHANGELOG.sample.md"
else
    SCRIPT="${WORKSPACE_ROOT}/tools/generate_release_notes.sh"
    FIXTURE="${WORKSPACE_ROOT}/tools/testdata/CHANGELOG.sample.md"
fi

if [[ ! -x "${SCRIPT}" ]]; then
    echo "missing executable: ${SCRIPT}" >&2
    exit 1
fi
if [[ ! -f "${FIXTURE}" ]]; then
    echo "missing fixture: ${FIXTURE}" >&2
    exit 1
fi

PASS_COUNT=0
FAIL_COUNT=0

assert_eq() {
    local expected="$1" actual="$2" label="$3"
    if [[ "${expected}" == "${actual}" ]]; then
        PASS_COUNT=$((PASS_COUNT + 1))
        echo "  PASS: ${label}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        {
            echo "  FAIL: ${label}"
            echo "  ---- expected ----"
            echo "${expected}"
            echo "  ---- actual ----"
            echo "${actual}"
            echo "  ----------------"
        } >&2
    fi
}

assert_contains() {
    local needle="$1" haystack="$2" label="$3"
    # Here-string, NOT `printf '%s' "${haystack}" | grep -qF`. Under `set -o
    # pipefail` the piped form is a parallel-load flake (configflux-t04f): grep
    # -q matches and exits, closing the pipe while printf is still writing;
    # printf takes SIGPIPE (141), pipefail promotes the pipeline to 141, and the
    # `if` wrongly reports a present needle as "not found". The here-string has
    # no writer to kill.
    if grep -qF -- "${needle}" <<<"${haystack}"; then
        PASS_COUNT=$((PASS_COUNT + 1))
        echo "  PASS: ${label}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        {
            echo "  FAIL: ${label}: '${needle}' not found"
            echo "  ---- haystack ----"
            echo "${haystack}"
            echo "  ------------------"
        } >&2
    fi
}

assert_not_contains() {
    local needle="$1" haystack="$2" label="$3"
    # Here-string, NOT `printf '%s' "${haystack}" | grep -qF` (see assert_contains
    # above): the piped form is a SIGPIPE-under-pipefail flake (configflux-t04f).
    # Here the false negative is worse — it would silently mask an unwanted needle
    # that IS present, flipping a real FAIL into a spurious PASS.
    if grep -qF -- "${needle}" <<<"${haystack}"; then
        FAIL_COUNT=$((FAIL_COUNT + 1))
        {
            echo "  FAIL: ${label}: unwanted '${needle}' appeared"
            echo "  ---- haystack ----"
            echo "${haystack}"
            echo "  ------------------"
        } >&2
    else
        PASS_COUNT=$((PASS_COUNT + 1))
        echo "  PASS: ${label}"
    fi
}

assert_exit_code() {
    local expected="$1" actual="$2" label="$3"
    if [[ "${expected}" == "${actual}" ]]; then
        PASS_COUNT=$((PASS_COUNT + 1))
        echo "  PASS: ${label}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        echo "  FAIL: ${label}: expected exit ${expected}, got ${actual}" >&2
    fi
}

run() {
    # Usage: run VERSION_ARG [EXTRA_ENV...]
    # Emits: stdout on fd1, exit code on fd3.
    CHANGELOG_PATH="${FIXTURE}" VERSION_FILE="/nonexistent/VERSION" \
        "${SCRIPT}" "$@"
}

echo "test_generate_release_notes:"

# Case 1: extract the known 0.1.0 entry (unprefixed).
echo "case 1: extract 0.1.0 (unprefixed)"
out_1="$(run 0.1.0)"
rc_1=$?
assert_exit_code 0 "${rc_1}" "case 1: exit 0"
assert_contains "# v0.1.0 — 2026-04-09" "${out_1}" "case 1: title line"
assert_contains "### Added" "${out_1}" "case 1: Added section"
assert_contains "First public release of ConfigFlux." "${out_1}" "case 1: body sentence"
assert_contains "### Security" "${out_1}" "case 1: Security section"
assert_not_contains "motor-limits" "${out_1}" "case 1: no 0.2.0 leak"
assert_not_contains "## [0.0.1]" "${out_1}" "case 1: no prior-version heading leak"
assert_not_contains "Unreleased" "${out_1}" "case 1: no Unreleased leak"

# Case 2: v-prefix accepted and normalized.
echo "case 2: extract v0.1.0 (prefixed)"
out_2="$(run v0.1.0)"
rc_2=$?
assert_exit_code 0 "${rc_2}" "case 2: exit 0"
assert_contains "# v0.1.0 — 2026-04-09" "${out_2}" "case 2: title line"
assert_eq "${out_1}" "${out_2}" "case 2: same body as unprefixed"

# Case 3: middle version (0.2.0) extracts cleanly without bleeding into 0.1.0.
echo "case 3: extract 0.2.0"
out_3="$(run 0.2.0)"
rc_3=$?
assert_exit_code 0 "${rc_3}" "case 3: exit 0"
assert_contains "# v0.2.0 — 2026-05-15" "${out_3}" "case 3: title line"
assert_contains "motor-limits" "${out_3}" "case 3: body content"
assert_contains "PRODUCT_SCHEMA_VERSION" "${out_3}" "case 3: interface-version callout"
assert_not_contains "First public release of ConfigFlux" "${out_3}" "case 3: no 0.1.0 leak"
assert_not_contains "Unreleased" "${out_3}" "case 3: no Unreleased leak"

# Case 4: unknown version returns exit 4.
echo "case 4: unknown version"
set +e
err_4="$(run 9.9.9 2>&1 1>/dev/null)"
rc_4=$?
set -e
assert_exit_code 4 "${rc_4}" "case 4: exit 4"
assert_contains "no entry for version 9.9.9" "${err_4}" "case 4: error message"

# Case 5: invalid SemVer returns exit 1.
echo "case 5: invalid SemVer"
set +e
err_5="$(run "not-a-version" 2>&1 1>/dev/null)"
rc_5=$?
set -e
assert_exit_code 1 "${rc_5}" "case 5: exit 1"
assert_contains "not a SemVer" "${err_5}" "case 5: error message"

# Case 6: missing CHANGELOG returns exit 2.
echo "case 6: missing CHANGELOG"
set +e
err_6="$(CHANGELOG_PATH="/nonexistent/CHANGELOG.md" VERSION_FILE="/nonexistent/VERSION" \
    "${SCRIPT}" 0.1.0 2>&1 1>/dev/null)"
rc_6=$?
set -e
assert_exit_code 2 "${rc_6}" "case 6: exit 2"
assert_contains "CHANGELOG not found" "${err_6}" "case 6: error message"

# Case 7: no argument + no VERSION file returns exit 3.
echo "case 7: missing VERSION file, no arg"
set +e
err_7="$(CHANGELOG_PATH="${FIXTURE}" VERSION_FILE="/nonexistent/VERSION" \
    "${SCRIPT}" 2>&1 1>/dev/null)"
rc_7=$?
set -e
assert_exit_code 3 "${rc_7}" "case 7: exit 3"
assert_contains "VERSION file not found" "${err_7}" "case 7: error message"

# Case 8: no argument, version read from a provided VERSION file.
echo "case 8: read VERSION file"
tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT
printf '0.2.0\n' > "${tmpdir}/VERSION"
out_8="$(CHANGELOG_PATH="${FIXTURE}" VERSION_FILE="${tmpdir}/VERSION" "${SCRIPT}")"
rc_8=$?
assert_exit_code 0 "${rc_8}" "case 8: exit 0"
assert_contains "# v0.2.0 — 2026-05-15" "${out_8}" "case 8: title from VERSION file"

# Case 9: VERSION file with v-prefix and trailing whitespace still works.
echo "case 9: VERSION file with v-prefix and whitespace"
printf '   v0.1.0   \n' > "${tmpdir}/VERSION"
out_9="$(CHANGELOG_PATH="${FIXTURE}" VERSION_FILE="${tmpdir}/VERSION" "${SCRIPT}")"
rc_9=$?
assert_exit_code 0 "${rc_9}" "case 9: exit 0"
assert_contains "# v0.1.0 — 2026-04-09" "${out_9}" "case 9: normalized title"

# ---------------------------------------------------------------------------

echo ""
echo "test_generate_release_notes: ${PASS_COUNT} passed, ${FAIL_COUNT} failed"
if [[ "${FAIL_COUNT}" -gt 0 ]]; then
    exit 1
fi
