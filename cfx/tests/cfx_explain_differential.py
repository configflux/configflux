#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# Golden + JSON-passthrough + exit-code + determinism validation for
# `cfx explain` (configflux-2awb.3 / CFX-2, ADR-0042 §3 + ADR-0031 D2/D3 +
# plan-launch-readiness-v2 §3).
#
# The ANCHOR: for two KNOWN-UNSAT selections over the committed cross-facet
# `s_labeled_mus` fixture, `cfx explain --format json` must be BYTE-IDENTICAL to
# the interpreter `explain` envelope response for the SAME request. cfx derives
# that request by walking the merged choice map in BTreeMap-sorted order,
# applying each with the solver-authoritative `session_compose::apply`, and
# explaining the FIRST solver-rejected choice against the state accumulated from
# the prior (satisfiable) choices. This driver mirrors that exactly by driving
# the interpreter `select` command over the sorted choices and explaining the
# first rejected one — so any byte difference is a real divergence, not a
# test artifact. Both tools route explain through the shared `session_compose`
# crate, so the labeled unsat core is the solver's authoritative answer
# (`Session::explain_rejection`, ADR-0031 D3) on both sides.
#
# Also checked: committed text goldens (the shared `explain_renderer` wording),
# the satisfiable case (exit 3, "nothing to explain"), and double-run
# determinism (json + text byte-equal across two runs).
#
# Binaries/fixtures come from env rlocations resolved by the sh_test wrapper.
# JSON shaping uses only the standard library (no jq dependency).

import json
import os
import subprocess
import sys
import tempfile

# s_labeled_mus selection context: the `rig` controller scope, no context tags.
SCOPE = "component:rig"

# Two known-unsat selections. Each maps to a distinct minimal core:
#   A: highperf CPU forbids `air` cooling (highperf_requires_liquid).
#   B: highperf CPU forbids `bronze` PSU (highperf_requires_gold).
CASE_A = [("cpu", "highperf"), ("cooling", "air")]
CASE_B = [("cpu", "highperf"), ("psu", "bronze")]
# A satisfiable partial selection — nothing to explain (exit 3).
CASE_SAT = [("cpu", "standard")]

EXIT_OK = 0
EXIT_UNSAT = 3


def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def env_path(name):
    value = os.environ.get(name)
    if not value:
        fail(f"required env var {name} is not set")
    if not os.path.exists(value):
        fail(f"{name} points at a missing path: {value}")
    return value


def run(cmd, *, check=True):
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if check and proc.returncode != 0:
        fail(f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stderr}")
    return proc


def compile_scenario(compiler, defs, components, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    run([compiler, "compile", "--source", defs, "--source", components, "--out", out_dir])
    manifest = os.path.join(out_dir, "cmp.manifest.json")
    if not os.path.exists(manifest):
        fail(f"compiler did not emit {manifest}")
    return manifest


def interp_op(interpreter, verb, request, work, tag):
    """Drive one interpreter envelope command via request/response files.
    Returns (raw_response_bytes, parsed_json, exit_code)."""
    req_path = os.path.join(work, f"{tag}.{verb}.request.json")
    res_path = os.path.join(work, f"{tag}.{verb}.response.json")
    with open(req_path, "w") as handle:
        json.dump(request, handle)
    proc = subprocess.run(
        [interpreter, verb, "--request-file", req_path, "--response-file", res_path],
        capture_output=True,
        text=True,
    )
    with open(res_path, "rb") as handle:
        raw = handle.read()
    return raw, json.loads(raw), proc.returncode


def interpreter_explain_envelope(interpreter, manifest, selects, work, tag):
    """Mirror cfx's culprit search on the interpreter envelope path: open ->
    init -> apply the BTreeMap-sorted choices one at a time; the FIRST rejected
    choice is explained against the state accumulated before it. Returns the raw
    explain response bytes (the byte-equality oracle)."""
    _, opened, _ = interp_op(
        interpreter, "open", {"schema_version": 2, "cmp_manifest_ref": manifest}, work, tag
    )
    if opened.get("status") != "ok":
        fail(f"interpreter open failed: {json.dumps(opened)}")
    handle = opened["model_handle"]

    _, init, _ = interp_op(
        interpreter,
        "init-selection-state",
        {"schema_version": 2, "model_handle": handle, "scope": SCOPE, "context_tags": {}},
        work,
        tag,
    )
    if init.get("status") != "ok":
        fail(f"interpreter init failed: {json.dumps(init)}")
    state = init["selection_state"]

    # cfx merges file+flags into a BTreeMap and walks it sorted by facet.
    for facet, option in sorted(selects):
        _, applied, _ = interp_op(
            interpreter,
            "select",
            {
                "schema_version": 2,
                "model_handle": handle,
                "scope": SCOPE,
                "selection_state": state,
                "selection_delta": {"facet": facet, "option": option},
            },
            work,
            f"{tag}.{facet}",
        )
        if applied.get("status") == "ok":
            state = applied["selection_state"]
            continue
        # First solver REJECT: this is the culprit cfx would explain.
        raw, _, _ = interp_op(
            interpreter,
            "explain",
            {
                "schema_version": 2,
                "model_handle": handle,
                "scope": SCOPE,
                "selection_state": state,
                "rejected_option": {"facet": facet, "option": option},
            },
            work,
            f"{tag}.explain",
        )
        return raw
    fail(f"{tag}: expected a rejected choice but every select was accepted (selection is SAT)")


def cfx_explain(cfx, manifest, sel_path, selects, fmt):
    cmd = [cfx, "explain", "--model", manifest, "--selection-file", sel_path, "--format", fmt]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    return subprocess.run(cmd, capture_output=True, text=False)


def write_selection_file(path):
    with open(path, "w") as handle:
        json.dump(
            {
                "schema_version": 2,
                "model_hash": "",
                "scope": SCOPE,
                "context_tags": {},
                "choices": {},
                "selection_state_hash": "",
            },
            handle,
        )


def check_json_byte_equal(cfx, interpreter, manifest, selects, work, tag):
    print(f"[cfx-explain] {tag}: explain JSON byte-equality vs interpreter envelope")
    sel_path = os.path.join(work, f"{tag}.selection.json")
    write_selection_file(sel_path)

    cfx_proc = cfx_explain(cfx, manifest, sel_path, selects, "json")
    if cfx_proc.returncode != EXIT_OK:
        fail(f"{tag}: cfx explain --format json exited {cfx_proc.returncode}, want 0\n"
             f"{cfx_proc.stderr.decode(errors='replace')}")
    cfx_bytes = cfx_proc.stdout

    interp_bytes = interpreter_explain_envelope(interpreter, manifest, selects, work, tag)
    if cfx_bytes != interp_bytes:
        fail(
            f"{tag}: cfx explain JSON differs from the interpreter envelope\n"
            f"  cfx    = {cfx_bytes.decode(errors='replace')!r}\n"
            f"  interp = {interp_bytes.decode(errors='replace')!r}"
        )
    # Sanity: a genuine conflict carries a populated unsat core (ADR-0031 D3).
    parsed = json.loads(cfx_bytes)
    if parsed.get("status") != "ok" or not parsed["rejection"].get("unsat_core"):
        fail(f"{tag}: expected an ok explanation with a populated unsat_core; got {cfx_bytes!r}")
    print(f"[cfx-explain] {tag}: OK (byte-identical explain envelope)")


def check_golden(cfx, manifest, selects, work, golden_dir, golden_name, tag):
    print(f"[cfx-explain] {tag}: text golden {golden_name}")
    sel_path = os.path.join(work, f"{tag}.golden.selection.json")
    write_selection_file(sel_path)
    proc = cfx_explain(cfx, manifest, sel_path, selects, "text")
    if proc.returncode != EXIT_OK:
        fail(f"{tag}: cfx explain (text) exited {proc.returncode}, want 0\n"
             f"{proc.stderr.decode(errors='replace')}")
    got = proc.stdout.decode()
    with open(os.path.join(golden_dir, golden_name)) as handle:
        want = handle.read()
    if got != want:
        fail(f"{tag}: text golden mismatch:\n--- want ---\n{want}\n--- got ---\n{got}")
    print(f"[cfx-explain] {tag}: text golden OK")


def check_satisfiable(cfx, manifest, work):
    print("[cfx-explain] satisfiable: exit 3 + 'nothing to explain'")
    sel_path = os.path.join(work, "sat.selection.json")
    write_selection_file(sel_path)
    proc = cfx_explain(cfx, manifest, sel_path, CASE_SAT, "text")
    if proc.returncode != EXIT_UNSAT:
        fail(f"satisfiable: cfx explain exited {proc.returncode}, want {EXIT_UNSAT}\n"
             f"{proc.stderr.decode(errors='replace')}")
    out = proc.stdout.decode()
    if "selection is satisfiable; nothing to explain" not in out:
        fail(f"satisfiable: stdout must say nothing to explain; got {out!r}")
    print("[cfx-explain] satisfiable: OK")


def check_determinism(cfx, manifest, work):
    print("[cfx-explain] determinism: two cfx runs byte-equal (json + text)")
    sel_path = os.path.join(work, "determinism.selection.json")
    write_selection_file(sel_path)
    for fmt in ("json", "text"):
        a = cfx_explain(cfx, manifest, sel_path, CASE_A, fmt).stdout
        b = cfx_explain(cfx, manifest, sel_path, CASE_A, fmt).stdout
        if a != b:
            fail(f"determinism: cfx explain --format {fmt} differed between runs")
    print("[cfx-explain] determinism: OK")


def main():
    compiler = env_path("COMPILER")
    interpreter = env_path("INTERPRETER")
    cfx = env_path("CFX")
    defs = env_path("MUS_DEFS")
    components = env_path("MUS_COMPONENTS")
    golden_dir = env_path("GOLDEN_DIR")

    workroot = tempfile.mkdtemp(prefix="cfx-explain-", dir=os.environ.get("TEST_TMPDIR"))
    manifest = compile_scenario(compiler, defs, components, os.path.join(workroot, "cmp"))

    # Case A: cpu=highperf conflicts with cooling=air.
    check_json_byte_equal(cfx, interpreter, manifest, CASE_A, workroot, "case-a")
    check_golden(
        cfx, manifest, CASE_A, workroot, golden_dir,
        "explain_labeled_mus_cpu_cooling.stdout", "case-a",
    )

    # Case B: cpu=highperf conflicts with psu=bronze — a different minimal core.
    check_json_byte_equal(cfx, interpreter, manifest, CASE_B, workroot, "case-b")
    check_golden(
        cfx, manifest, CASE_B, workroot, golden_dir,
        "explain_labeled_mus_cpu_psu.stdout", "case-b",
    )

    check_satisfiable(cfx, manifest, workroot)
    check_determinism(cfx, manifest, workroot)

    print("[cfx-explain] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
