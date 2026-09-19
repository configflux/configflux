#!/usr/bin/env bash
# Process-level test for the `cfx diff` review journey — the "Reviewing a
# change before it ships" workflow in docs/service-integration-guide.md
# (ADR-0059 D4, ADR-0061 D3).
#
# `cfx diff` answers the question `resolve_hash` cannot: WHICH deployments did
# my change touch, and how. Until now that was covered only in process, by
# cfx/src/diff_tests.rs. This test drives the REAL binaries the way a reviewer
# does — compile the sources on the main branch, compile the sources carrying
# the change, then ask `cfx diff` what moved — over the shipped example packs.
#
# What it pins, per ADR-0059 D4: which cells the report visits and in which
# order (sorted on BOTH axes); the cell statuses, against edits engineered to
# produce each one; the change lines under a `changed` cell; the JSON summary
# and the per-cell hash pair that motivates the whole verb — `resolve_hash`
# rotates for EVERY cell when the model changes at all, while
# `resolved_output_hash` moves only for the cells whose delivered payload
# actually moved; byte-for-byte determinism across two identical runs; and the
# exit contract — 0 = every cell unchanged, 1 = at least one differs, 2 =
# usage/IO, never 3, because unsatisfiability is a cell STATUS here and not a
# command failure.
#
# Every assertion parses TOKENS, never fixed column widths, so the report's
# layout can be reflowed without a false failure here. python3 does all JSON
# shaping and asserting — no jq, mirroring examples/e2e_05_compose_fleet_test.sh.
#
# Determinism of the inputs: model_hash is derived from the `--source` argument
# STRINGS plus their content, so BOTH sides are staged into their own directory
# and compiled with FILENAME-ONLY source ids. The two models then differ by
# exactly the edit under review, never by where the test happened to run.
set -euo pipefail

# ---------------------------------------------------------------------------
# Runfiles resolution
# ---------------------------------------------------------------------------
if [[ -z "${TEST_SRCDIR:-}" ]]; then
  echo "ERROR: TEST_SRCDIR is not set; this script must run under bazel test" >&2
  exit 2
fi
RUNFILES_ROOT="${TEST_SRCDIR}/_main"

# One <NAME>=<runfiles path> variable per required <NAME>_RLOCATION, so adding
# an input is a one-word edit here and a one-line edit in BUILD.bazel.
for name in CFX COMPILER DEFS05 COMPONENTS05 MANIFEST05 DEFS00 COMPONENTS00 MANIFEST00; do
  rlocation="${name}_RLOCATION"
  if [[ -z "${!rlocation:-}" ]]; then
    echo "ERROR: ${rlocation} env var is not set" >&2
    exit 2
  fi
  declare "${name}=${RUNFILES_ROOT}/${!rlocation}"
done

for bin in "${CFX}" "${COMPILER}"; do
  [[ -x "${bin}" ]] || { echo "ERROR: binary not executable at ${bin}" >&2; exit 2; }
done
for f in "${DEFS05}" "${COMPONENTS05}" "${MANIFEST05}" \
         "${DEFS00}" "${COMPONENTS00}" "${MANIFEST00}"; do
  [[ -f "${f}" ]] || { echo "ERROR: data file not found at ${f}" >&2; exit 2; }
done

T="${TEST_TMPDIR:-$(mktemp -d)}/diff"
rm -rf "${T}"
mkdir -p "${T}"

fail() { echo "ASSERT FAILED: $*" >&2; exit 1; }
step() { printf '\n=== %s ===\n' "$1"; }

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# compile_model SRC_DIR OUT_DIR — compile the two staged sources with
# FILENAME-ONLY source ids (see the determinism note in the header) and assert
# the compiler reported ok and wrote a CMP manifest.
compile_model() {
  local src_dir="$1" out_dir="$2" result="$2.compile.json"
  (
    cd "${src_dir}"
    "${COMPILER}" compile \
      --source 00_definitions.json \
      --source 10_components.json \
      --out "${out_dir}" \
      > "${result}"
  )
  local status
  status="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "${result}")"
  [[ "${status}" == "ok" ]] \
    || { cat "${result}" >&2; fail "compile ${src_dir}: expected status ok, got '${status}'"; }
  [[ -f "${out_dir}/cmp.manifest.json" ]] \
    || fail "compile ${src_dir}: cmp.manifest.json missing under ${out_dir}"
}

# run_diff PREFIX ARGS... — run `cfx diff`, capturing stdout at PREFIX.out and
# stderr at PREFIX.err, and echo the exit code. A non-zero exit is the EXPECTED
# outcome for most steps here, so `set -e` is lifted around the call only.
run_diff() {
  local prefix="$1"; shift
  local rc=0
  set +e
  "${CFX}" diff "$@" > "${prefix}.out" 2> "${prefix}.err"
  rc=$?
  set -e
  echo "${rc}"
}

# assert_exit WANT GOT PREFIX LABEL — pin one invocation's exit code, dumping
# both captured streams when it does not match.
assert_exit() {
  local want="$1" got="$2" prefix="$3" label="$4"
  if [[ "${got}" != "${want}" ]]; then
    echo "--- ${label} stdout ---" >&2; cat "${prefix}.out" >&2 || true
    echo "--- ${label} stderr ---" >&2; cat "${prefix}.err" >&2 || true
    fail "${label}: expected exit ${want}, got ${got}"
  fi
}

# assert_diff_cells REPORT EXPECTED... — the cell-verdict assertion for the
# whole test (traceability symbol for REQ-CFX-013).
#
# Checks the TEXT report's cell lines against EXPECTED, each spelled
# "<status> <environment> <scope>", in order. A cell line is any line that is
# neither indented (change and rejection lines carry a two-space indent) nor
# the trailing summary; splitting on whitespace keeps column spacing out of the
# contract. This is the assertion with the teeth: the summary counts and the
# exit code are both blind to WHICH cell moved — an edit landing on the wrong
# deployment yields an identical "1 changed, 3 unchanged" verdict.
assert_diff_cells() {
  local report="$1"; shift
  python3 - "${report}" "$@" <<'PY' || fail "diff cell verdicts do not match"
import sys

report, expected = sys.argv[1], sys.argv[2:]
cells = [
    " ".join(line.split())
    for line in open(report, encoding="utf-8").read().splitlines()
    if line and not line.startswith(" ") and not line.startswith("summary:")
]
if cells != expected:
    for label, rows in (("expected", expected), ("actual", cells)):
        print(label + " cells:", file=sys.stderr)
        for row in rows:
            print("  " + row, file=sys.stderr)
    raise SystemExit(1)
print("  -> %d cells, in order: %s" % (len(cells), "; ".join(cells)))
PY
}

# assert_change_lines REPORT WANT_COUNT [SUBSTRING...] — pin the indented lines
# under the cells: how many there are, and that the first carries every
# SUBSTRING. Change lines are `~ <path>: <before> -> <after>`, `+ <path>` or
# `- <path>`; rejection lines are `<side>: <code>: <message>`.
assert_change_lines() {
  local report="$1" want_count="$2"; shift 2
  python3 - "${report}" "${want_count}" "$@" <<'PY' || fail "diff change lines do not match"
import sys

report, want_count, substrings = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
lines = [
    line.strip()
    for line in open(report, encoding="utf-8").read().splitlines()
    if line.startswith("  ")
]
if len(lines) != want_count:
    print("indented lines: %r" % (lines,), file=sys.stderr)
    raise SystemExit("expected %d indented line(s), got %d" % (want_count, len(lines)))
if substrings and not lines[0].startswith(substrings[0]):
    raise SystemExit("line %r does not start with %r" % (lines[0], substrings[0]))
for needle in substrings[1:]:
    if needle not in lines[0]:
        raise SystemExit("line %r does not contain %r" % (lines[0], needle))
print("  -> %d line(s); first: %s" % (len(lines), lines[0] if lines else "(none)"))
PY
}

# ---------------------------------------------------------------------------
# S1/S2 — two models of examples/05-compose-fleet, one parameter apart
# ---------------------------------------------------------------------------
# telemetry_service.tick_interval overrides on `deploy_env`: the robot arm is
# 1000 ms, the local arm 5000 ms. Moving the ROBOT arm to 1500 is the smallest
# realistic review, and it must land on exactly one of the four cells — only
# `robot-alpha` chooses deploy_env=robot, and only telemetry delivers it.
step "S1 base model (05-compose-fleet, as shipped)"
mkdir -p "${T}/base05_src"
cp "${DEFS05}" "${T}/base05_src/00_definitions.json"
cp "${COMPONENTS05}" "${T}/base05_src/10_components.json"
compile_model "${T}/base05_src" "${T}/base05"

step "S2 head model (robot tick_interval override 1000 -> 1500)"
mkdir -p "${T}/head05_src"
cp "${DEFS05}" "${T}/head05_src/00_definitions.json"
python3 - "${COMPONENTS05}" "${T}/head05_src/10_components.json" <<'PY'
import json
import sys

model = json.load(open(sys.argv[1], encoding="utf-8"))
overrides = model["components"]["telemetry_service"]["params"]["tick_interval"]["overrides"]
arm = overrides[0]
assert "robot" in arm["condition"], "override[0] is not the robot arm: %r" % (arm,)
assert arm["value"] == 1000, "override[0] value moved: %r" % (arm,)
arm["value"] = 1500
json.dump(model, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
compile_model "${T}/head05_src" "${T}/head05"

DIFF05=(
  --base "${T}/base05/cmp.manifest.json"
  --head "${T}/head05/cmp.manifest.json"
  --manifest "${MANIFEST05}"
  --scopes component:vision_service,component:telemetry_service
)

# ---------------------------------------------------------------------------
# S3 — the text report: which cells changed, and how
# ---------------------------------------------------------------------------
step "S3 cfx diff --format text (4 cells, one changed)"
S3="${T}/s3"
RC="$(run_diff "${S3}" "${DIFF05[@]}")"
assert_exit 1 "${RC}" "${S3}" "S3 diff"
cat "${S3}.out"
assert_diff_cells "${S3}.out" \
  "unchanged local component:telemetry_service" \
  "unchanged local component:vision_service" \
  "changed robot-alpha component:telemetry_service" \
  "unchanged robot-alpha component:vision_service"
assert_change_lines "${S3}.out" 1 \
  "~ component.telemetry_service.param.tick_interval" "1000" "1500"
grep -q '^summary: unchanged=3 changed=1 now_unsatisfiable=0 now_satisfiable=0 unsatisfiable_both=0$' \
  "${S3}.out" || fail "S3: summary line does not report 3 unchanged / 1 changed"

# ---------------------------------------------------------------------------
# S4 — the JSON envelope, and determinism
# ---------------------------------------------------------------------------
step "S4 cfx diff --format json (summary, hashes, determinism)"
S4="${T}/s4"
RC="$(run_diff "${S4}" "${DIFF05[@]}" --format json)"
assert_exit 1 "${RC}" "${S4}" "S4 diff"
python3 - "${S4}.out" <<'PY' || fail "S4: JSON envelope assertions failed"
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))

want = {"unchanged": 3, "changed": 1, "now_unsatisfiable": 0,
        "now_satisfiable": 0, "unsatisfiable_both": 0}
if report["summary"] != want:
    raise SystemExit("summary %r != %r" % (report["summary"], want))
if report["base_model_hash"] == report["head_model_hash"]:
    raise SystemExit("base and head model_hash are equal; the models are not distinct")

changed = [c for c in report["cells"] if c["status"] == "changed"]
unchanged = [c for c in report["cells"] if c["status"] == "unchanged"]
if (len(changed), len(unchanged)) != (1, 3):
    raise SystemExit("expected 1 changed / 3 unchanged cells, got %d / %d"
                     % (len(changed), len(unchanged)))

cell = changed[0]
if (cell["environment"], cell["scope"]) != ("robot-alpha", "component:telemetry_service"):
    raise SystemExit("wrong cell changed: %r" % ((cell["environment"], cell["scope"]),))
if cell["base_resolved_output_hash"] == cell["head_resolved_output_hash"]:
    raise SystemExit("the changed cell's resolved_output_hash did not move")

# The distinction the verb exists for (ADR-0059 D3): the delivered payload is
# identical on every unchanged cell even though model_hash — and therefore
# resolve_hash, which carries it — moved for all of them.
for cell in unchanged:
    where = (cell["environment"], cell["scope"])
    if cell["base_resolved_output_hash"] != cell["head_resolved_output_hash"]:
        raise SystemExit("unchanged cell %r has a moved resolved_output_hash" % (where,))
    if cell["base_resolve_hash"] == cell["head_resolve_hash"]:
        raise SystemExit("unchanged cell %r kept its resolve_hash across a model edit; "
                         "resolve_hash is supposed to carry model_hash" % (where,))
    if cell["changes"]:
        raise SystemExit("unchanged cell %r carries change entries" % (where,))

print("  -> summary, model hashes and per-cell payload hashes all as expected")
PY

S4B="${T}/s4b"
RC="$(run_diff "${S4B}" "${DIFF05[@]}" --format json)"
assert_exit 1 "${RC}" "${S4B}" "S4 rerun"
cmp -s "${S4}.out" "${S4B}.out" \
  || { diff "${S4}.out" "${S4B}.out" >&2 || true; fail "S4: two identical runs differ"; }
echo "  -> two runs byte-identical"

# ---------------------------------------------------------------------------
# S5 — a model against itself: the exit-0 case a pull-request check relies on
# ---------------------------------------------------------------------------
step "S5 cfx diff base-vs-base (every cell unchanged, exit 0)"
S5="${T}/s5"
RC="$(run_diff "${S5}" \
  --base "${T}/base05/cmp.manifest.json" \
  --head "${T}/base05/cmp.manifest.json" \
  --manifest "${MANIFEST05}")"
assert_exit 0 "${RC}" "${S5}" "S5 diff"
[[ -s "${S5}.out" ]] || fail "S5: a no-op diff printed nothing; it must still list its cells"
# No --scopes here, so each environment contributes its OWN scope from the
# manifest — the default enumeration, pinned alongside the narrowed one above.
assert_diff_cells "${S5}.out" \
  "unchanged local component:vision_service" \
  "unchanged robot-alpha component:vision_service"
assert_change_lines "${S5}.out" 0

# ---------------------------------------------------------------------------
# S6 — a constraint edit: now_unsatisfiable, and its mirror
# ---------------------------------------------------------------------------
# examples/00-service-multi-env forbids debug logging in production. Inverting
# the second half of that constraint makes it DEMAND debug logging there, which
# the shipped `prod` environment (log_level=info) cannot satisfy — while `dev`
# and `staging` still can, because the first half short-circuits for them.
step "S6 base + head models (00-service-multi-env, prod constraint inverted)"
mkdir -p "${T}/base00_src" "${T}/head00_src"
cp "${DEFS00}" "${T}/base00_src/00_definitions.json"
cp "${COMPONENTS00}" "${T}/base00_src/10_components.json"
cp "${COMPONENTS00}" "${T}/head00_src/10_components.json"
python3 - "${DEFS00}" "${T}/head00_src/00_definitions.json" <<'PY'
import json
import sys

model = json.load(open(sys.argv[1], encoding="utf-8"))
constraint = model["constraints"]["prod_forbids_debug"]
before = "environment != 'prod' || log_level != 'debug'"
assert constraint["condition"] == before, "constraint moved: %r" % (constraint,)
constraint["condition"] = "environment != 'prod' || log_level == 'debug'"
json.dump(model, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
compile_model "${T}/base00_src" "${T}/base00"
compile_model "${T}/head00_src" "${T}/head00"

DIFF00=(
  --base "${T}/base00/cmp.manifest.json"
  --head "${T}/head00/cmp.manifest.json"
  --manifest "${MANIFEST00}"
)

step "S6 cfx diff — prod becomes unsatisfiable"
S6="${T}/s6"
RC="$(run_diff "${S6}" "${DIFF00[@]}")"
assert_exit 1 "${RC}" "${S6}" "S6 diff"
cat "${S6}.out"
assert_diff_cells "${S6}.out" \
  "unchanged dev component:webapp" \
  "now_unsatisfiable prod component:webapp" \
  "unchanged staging component:webapp"
assert_change_lines "${S6}.out" 1 "head:" "prod_forbids_debug"

S6J="${T}/s6j"
RC="$(run_diff "${S6J}" "${DIFF00[@]}" --format json)"
assert_exit 1 "${RC}" "${S6J}" "S6 diff json"
python3 - "${S6J}.out" <<'PY' || fail "S6: JSON summary is not one now_unsatisfiable cell"
import json
import sys

summary = json.load(open(sys.argv[1], encoding="utf-8"))["summary"]
if summary["now_unsatisfiable"] != 1:
    raise SystemExit("now_unsatisfiable %r != 1 (summary %r)"
                     % (summary["now_unsatisfiable"], summary))
if summary["now_satisfiable"] or summary["unsatisfiable_both"]:
    raise SystemExit("unexpected mirror counts in %r" % (summary,))
print("  -> now_unsatisfiable=1")
PY

step "S6 mirror — swapping base and head reports now_satisfiable"
S6R="${T}/s6r"
RC="$(run_diff "${S6R}" \
  --base "${T}/head00/cmp.manifest.json" \
  --head "${T}/base00/cmp.manifest.json" \
  --manifest "${MANIFEST00}")"
assert_exit 1 "${RC}" "${S6R}" "S6 mirror diff"
assert_diff_cells "${S6R}.out" \
  "unchanged dev component:webapp" \
  "now_satisfiable prod component:webapp" \
  "unchanged staging component:webapp"

# ---------------------------------------------------------------------------
# S7 — the usage class: exit 2, never a cell verdict
# ---------------------------------------------------------------------------
step "S7 usage errors exit 2"
S7A="${T}/s7a"
RC="$(run_diff "${S7A}" \
  --base "${T}/base05/cmp.manifest.json" \
  --head "${T}/head05/cmp.manifest.json")"
assert_exit 2 "${RC}" "${S7A}" "S7 missing --manifest"
grep -q -- '--manifest' "${S7A}.err" \
  || { cat "${S7A}.err" >&2; fail "S7: the missing-argument diagnostic does not name --manifest"; }
[[ ! -s "${S7A}.out" ]] || { cat "${S7A}.out" >&2; fail "S7: a usage error wrote a report to stdout"; }
echo "  -> missing --manifest: exit 2, diagnostic names the flag"

S7B="${T}/s7b"
RC="$(run_diff "${S7B}" \
  --base "${T}/base05/cmp.manifest.json" \
  --head "${T}/no-such-directory/cmp.manifest.json" \
  --manifest "${MANIFEST05}")"
assert_exit 2 "${RC}" "${S7B}" "S7 unreadable --head"
grep -q 'head' "${S7B}.err" \
  || { cat "${S7B}.err" >&2; fail "S7: the unreadable-model diagnostic does not name the side"; }
[[ ! -s "${S7B}.out" ]] || { cat "${S7B}.out" >&2; fail "S7: an IO error wrote a report to stdout"; }
echo "  -> unreadable --head: exit 2, diagnostic names the side"

step "DONE — cfx diff review journey green"
echo "text cells + change lines -> json summary/hashes -> determinism -> no-op(0) -> unsat pair -> usage(2)"
