#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# JSON-passthrough + golden + determinism validation for `cfx options`
# (configflux-2awb.4 / CFX-3, ADR-0042 §3 + plan-launch-readiness-v2 §4).
#
# The ANCHOR: for scenario fixture S1, with an EMPTY selection AND after one
# `--select`, EVERY per-facet object `cfx options --format json` emits must be
# byte-identical to the interpreter `options` envelope response for that facet —
# no facet excepted. Both tools route `options` through the shared
# `session_compose` crate, so the still-valid set is the solver's authoritative
# answer (`Session::valid_options`, ADR-0017 §4 / ADR-0030) on both sides. This is
# byte-identity BY CONSTRUCTION: cfx no longer calls the compiler's legacy
# `get_selection_options`, which under-reported override-gated facets after a
# selection (S1 `cooling_model` after `cooling_brand=hydra` narrowed to `["x200"]`
# instead of the authoritative `["a9","x200"]`). Proven here on real binaries.
#
# Also checked: committed text goldens (empty + after-select) and double-run
# determinism (json + text byte-equal across two runs).
#
# Binaries/fixtures come from env rlocations resolved by the sh_test wrapper.
# JSON shaping uses only the standard library (no jq dependency).

import json
import os
import subprocess
import sys
import tempfile

# S1 selection context, matching the compiler guided-selection scenario tests: the
# thermal_control scope with no immutable context tags.
S1_SCOPE = "component:thermal_control"
S1_CONTEXT = {}
# A satisfiable partial selection exercised by the "after one --select" case.
AFTER_SELECT_FACET = "cooling_brand"
AFTER_SELECT_OPTION = "hydra"


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
    """Drive one interpreter envelope command via request/response files."""
    req_path = os.path.join(work, f"{tag}.{verb}.request.json")
    res_path = os.path.join(work, f"{tag}.{verb}.response.json")
    with open(req_path, "w") as handle:
        json.dump(request, handle)
    run([interpreter, verb, "--request-file", req_path, "--response-file", res_path])
    with open(res_path) as handle:
        return json.load(handle)


def interpreter_state(interpreter, manifest, scope, context, selects, work, tag):
    """open -> init-selection-state -> (select)* and return (handle, state)."""
    opened = interp_op(
        interpreter, "open", {"schema_version": 4, "cmp_manifest_ref": manifest}, work, tag
    )
    if opened.get("status") != "ok":
        fail(f"interpreter open failed: {json.dumps(opened)}")
    handle = opened["model_handle"]

    init = interp_op(
        interpreter,
        "init-selection-state",
        {"schema_version": 4, "model_handle": handle, "scope": scope, "context_tags": context},
        work,
        tag,
    )
    if init.get("status") != "ok":
        fail(f"interpreter init failed: {json.dumps(init)}")
    state = init["selection_state"]

    for facet, option in selects:
        applied = interp_op(
            interpreter,
            "select",
            {
                "schema_version": 4,
                "model_handle": handle,
                "scope": scope,
                "selection_state": state,
                "selection_delta": {"facet": facet, "option": option},
            },
            work,
            f"{tag}.{facet}",
        )
        if applied.get("status") != "ok":
            fail(f"interpreter select {facet}={option} failed: {json.dumps(applied)}")
        state = applied["selection_state"]

    return handle, state


def interpreter_options(interpreter, handle, scope, state, facet, work, tag):
    resp = interp_op(
        interpreter,
        "options",
        {
            "schema_version": 4,
            "model_handle": handle,
            "scope": scope,
            "selection_state": state,
            "facet": facet,
        },
        work,
        f"{tag}.{facet}",
    )
    if resp.get("status") != "ok":
        fail(f"interpreter options for facet {facet} failed: {json.dumps(resp)}")
    return resp


def write_selection_file(path, scope, context):
    with open(path, "w") as handle:
        json.dump(
            {
                "schema_version": 4,
                "model_hash": "",
                "scope": scope,
                "context_tags": context,
                "choices": {},
                "selection_state_hash": "",
            },
            handle,
        )


def cfx_options(cfx, manifest, sel_path, selects, fmt):
    cmd = [cfx, "options", "--model", manifest, "--selection-file", sel_path, "--format", fmt]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    return run(cmd)


def canonical(obj):
    """Compact, field-order-preserving serialization. Both cfx and the
    interpreter serialize the SAME Rust struct via serde, so equal parsed objects
    imply byte-identical serialized output for these hash-free result structs."""
    return json.dumps(obj, separators=(",", ":"))


def check_passthrough(cfx, interpreter, manifest, scope, context, selects, work, tag):
    # EVERY facet must be byte-identical between cfx and the interpreter envelope,
    # both EMPTY and AFTER a partial selection — no facet excepted. Both tools now
    # route `options` through `session_compose`, so the still-valid set is the
    # solver's authoritative answer on both sides (ADR-0017 §4 / ADR-0030). The
    # override-gated facet `cooling_model` after `cooling_brand=hydra` is the
    # authoritative `["a9","x200"]` on both sides; the earlier compiler-path
    # divergence (cfx narrowing to `["x200"]`) is gone by construction — cfx no
    # longer calls the legacy `get_selection_options`.
    print(f"[cfx-opts] {tag}: JSON passthrough byte-equality vs interpreter envelope")
    sel_path = os.path.join(work, f"{tag}.selection.json")
    write_selection_file(sel_path, scope, context)

    cfx_json = cfx_options(cfx, manifest, sel_path, selects, "json").stdout
    cfx_arr = json.loads(cfx_json)
    if not isinstance(cfx_arr, list) or not cfx_arr:
        fail(f"{tag}: cfx options --format json must emit a non-empty array")

    # cfx is the source of the facet universe; the interpreter is driven for the
    # exact same facets and each per-facet result must match byte-for-byte.
    handle, state = interpreter_state(interpreter, manifest, scope, context, selects, work, tag)
    facets = [elem["facet"] for elem in cfx_arr]
    if facets != sorted(facets):
        fail(f"{tag}: cfx facet order must be sorted; got {facets}")

    matched = 0
    for elem in cfx_arr:
        facet = elem["facet"]
        interp = interpreter_options(interpreter, handle, scope, state, facet, work, tag)
        if canonical(elem) != canonical(interp):
            fail(
                f"{tag}: facet '{facet}' JSON differs between cfx and interpreter envelope\n"
                f"  cfx  = {canonical(elem)}\n  interp = {canonical(interp)}"
            )
        matched += 1
    print(f"[cfx-opts] {tag}: OK ({matched} facets byte-identical)")
    return facets


def check_golden(cfx, manifest, scope, context, selects, work, golden_dir, golden_name, tag):
    print(f"[cfx-opts] {tag}: text golden {golden_name}")
    sel_path = os.path.join(work, f"{tag}.golden.selection.json")
    write_selection_file(sel_path, scope, context)
    got = cfx_options(cfx, manifest, sel_path, selects, "text").stdout
    with open(os.path.join(golden_dir, golden_name)) as handle:
        want = handle.read()
    if got != want:
        fail(f"{tag}: text golden mismatch:\n--- want ---\n{want}\n--- got ---\n{got}")
    print(f"[cfx-opts] {tag}: text golden OK")


def check_determinism(cfx, manifest, scope, context, selects, work):
    print("[cfx-opts] determinism: two cfx runs must be byte-equal (json + text)")
    sel_path = os.path.join(work, "determinism.selection.json")
    write_selection_file(sel_path, scope, context)
    for fmt in ("json", "text"):
        a = cfx_options(cfx, manifest, sel_path, selects, fmt).stdout
        b = cfx_options(cfx, manifest, sel_path, selects, fmt).stdout
        if a != b:
            fail(f"determinism: cfx options --format {fmt} differed between runs")
    print("[cfx-opts] determinism: OK")


def main():
    compiler = env_path("COMPILER")
    interpreter = env_path("INTERPRETER")
    cfx = env_path("CFX")
    s1_defs = env_path("S1_DEFS")
    s1_components = env_path("S1_COMPONENTS")
    golden_dir = env_path("GOLDEN_DIR")

    workroot = tempfile.mkdtemp(prefix="cfx-opts-", dir=os.environ.get("TEST_TMPDIR"))
    manifest = compile_scenario(compiler, s1_defs, s1_components, os.path.join(workroot, "cmp"))

    empty_selects = []
    after_selects = [(AFTER_SELECT_FACET, AFTER_SELECT_OPTION)]

    # Empty partial selection.
    check_passthrough(cfx, interpreter, manifest, S1_SCOPE, S1_CONTEXT, empty_selects, workroot, "empty")
    check_golden(
        cfx, manifest, S1_SCOPE, S1_CONTEXT, empty_selects, workroot, golden_dir,
        "options_s1_empty.stdout", "empty",
    )

    # After one --select (a satisfiable partial selection): byte-equal for EVERY
    # facet, including the override-gated `cooling_model` — both tools route
    # `options` through `session_compose`, so cfx now reports the solver's
    # authoritative valid set exactly as the interpreter envelope does.
    check_passthrough(
        cfx, interpreter, manifest, S1_SCOPE, S1_CONTEXT, after_selects, workroot,
        "after-select",
    )
    check_golden(
        cfx, manifest, S1_SCOPE, S1_CONTEXT, after_selects, workroot, golden_dir,
        "options_s1_after_select.stdout", "after-select",
    )

    check_determinism(cfx, manifest, S1_SCOPE, S1_CONTEXT, empty_selects, workroot)

    print("[cfx-opts] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
