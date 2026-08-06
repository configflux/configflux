#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# Differential + determinism + exit-code + golden validation for `cfx resolve`
# (configflux-2awb.2 / CFX-1, ADR-0042 §5).
#
# The ANCHOR (ADR-0042 §5): for scenario fixtures S1 and S5, resolving via the
# real `interpreter` envelope path (open -> init-selection-state -> resolve ->
# export-resolved, the solver-gated production seam) and resolving via `cfx`
# must produce BYTE-IDENTICAL exported snapshots and an identical `resolve_hash`.
# `cfx` composes the same compiler loader-API calls the interpreter routes to; a
# satisfiable selection makes the two paths byte-identical by construction.
#
# Also checked here: double-run determinism, the 0/2/3 exit-code contract, and
# committed golden transcripts.
#
# All binaries are located from environment rlocations resolved by the sh_test
# wrapper. JSON shaping uses only the standard library (no jq dependency).

import json
import os
import re
import subprocess
import sys
import tempfile

HASH_RE = re.compile(r"\b[0-9a-f]{64}\b")


def env_path(name):
    value = os.environ.get(name)
    if not value:
        fail(f"required env var {name} is not set")
    if not os.path.exists(value):
        fail(f"{name} points at a missing path: {value}")
    return value


def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def run(cmd, *, stdin=None, check=True):
    proc = subprocess.run(
        cmd,
        input=stdin,
        capture_output=True,
        text=True,
    )
    if check and proc.returncode != 0:
        fail(f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stderr}")
    return proc


def compile_scenario(compiler, defs, components, out_dir):
    """Compile a scenario into a CMP package (with sibling ccm/) and return the
    manifest path."""
    os.makedirs(out_dir, exist_ok=True)
    run(
        [
            compiler,
            "compile",
            "--source",
            defs,
            "--source",
            components,
            "--out",
            out_dir,
        ]
    )
    manifest = os.path.join(out_dir, "cmp.manifest.json")
    if not os.path.exists(manifest):
        fail(f"compiler did not emit {manifest}")
    return manifest


def interp_op(interpreter, verb, request, work):
    """Drive one interpreter envelope command via request/response files and
    return the parsed response."""
    req_path = os.path.join(work, f"{verb}.request.json")
    res_path = os.path.join(work, f"{verb}.response.json")
    with open(req_path, "w") as handle:
        json.dump(request, handle)
    run(
        [
            interpreter,
            verb,
            "--request-file",
            req_path,
            "--response-file",
            res_path,
        ]
    )
    with open(res_path) as handle:
        return json.load(handle)


def interpreter_envelope_path(interpreter, manifest, scope, context_tags, work, snapshot_dir):
    """Run open -> init-selection-state -> resolve -> export-resolved and write
    the exported files into snapshot_dir. Returns the resolve_hash."""
    opened = interp_op(
        interpreter,
        "open",
        {"schema_version": 4, "cmp_manifest_ref": manifest},
        work,
    )
    if opened.get("status") != "ok":
        fail(f"interpreter open failed: {json.dumps(opened)}")
    handle = opened["model_handle"]

    init = interp_op(
        interpreter,
        "init-selection-state",
        {
            "schema_version": 4,
            "model_handle": handle,
            "scope": scope,
            "context_tags": context_tags,
        },
        work,
    )
    if init.get("status") != "ok":
        fail(f"interpreter init-selection-state failed: {json.dumps(init)}")
    selection_state = init["selection_state"]

    resolved = interp_op(
        interpreter,
        "resolve",
        {
            "schema_version": 4,
            "model_handle": handle,
            "scope": scope,
            "selection_state": selection_state,
        },
        work,
    )
    if resolved.get("status") != "ok":
        fail(f"interpreter resolve failed: {json.dumps(resolved)}")

    exported = interp_op(
        interpreter,
        "export-resolved",
        {
            "schema_version": 4,
            "resolve_result": resolved,
            "profile": "cpp_early_binding_v1",
        },
        work,
    )
    if exported.get("status") != "ok":
        fail(f"interpreter export-resolved failed: {json.dumps(exported)}")

    write_artifacts(exported["generated_artifacts"]["files"], snapshot_dir)
    return resolved["resolve_hash"]


def write_artifacts(files, snapshot_dir):
    for entry in files:
        target = os.path.join(snapshot_dir, entry["path"])
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "w") as handle:
            handle.write(entry["contents"])


def selection_file(path, scope, context_tags):
    """Write a SelectionState JSON carrying scope + context_tags (cfx re-derives
    the hash, so model_hash / selection_state_hash are placeholders)."""
    with open(path, "w") as handle:
        json.dump(
            {
                "schema_version": 4,
                "model_hash": "",
                "scope": scope,
                "context_tags": context_tags,
                "choices": {},
                "selection_state_hash": "",
            },
            handle,
        )


def snapshot_tree(root):
    """Map of relative path -> bytes for every file under root."""
    tree = {}
    for dirpath, _dirs, names in os.walk(root):
        for name in names:
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, root)
            with open(full, "rb") as handle:
                tree[rel] = handle.read()
    return tree


def parse_lineage(stdout):
    """Parse a cfx text lineage into a dict of the three hashes."""
    fields = {}
    for line in stdout.splitlines():
        if line.startswith(("model_hash:", "selection_state_hash:", "resolve_hash:")):
            key, _, value = line.partition(":")
            fields[key.strip()] = value.strip()
    return fields


def differential(cfx, interpreter, compiler, label, defs, components, scope, context, workroot):
    print(f"[cfx-diff] scenario {label}: differential vs interpreter envelope path")
    scenario_dir = os.path.join(workroot, label)
    os.makedirs(scenario_dir, exist_ok=True)

    manifest = compile_scenario(
        compiler, defs, components, os.path.join(scenario_dir, "cmp")
    )

    interp_snapshot = os.path.join(scenario_dir, "interp_out")
    os.makedirs(interp_snapshot, exist_ok=True)
    interp_hash = interpreter_envelope_path(
        interpreter, manifest, scope, context, scenario_dir, interp_snapshot
    )

    sel_path = os.path.join(scenario_dir, "selection.json")
    selection_file(sel_path, scope, context)

    cfx_snapshot = os.path.join(scenario_dir, "cfx_out")
    proc = run(
        [cfx, "resolve", "--model", manifest, "--selection-file", sel_path, "--out", cfx_snapshot]
    )
    cfx_hash = parse_lineage(proc.stdout).get("resolve_hash")

    # Exported snapshot bytes must be byte-identical.
    interp_tree = snapshot_tree(interp_snapshot)
    cfx_tree = snapshot_tree(cfx_snapshot)
    if interp_tree.keys() != cfx_tree.keys():
        fail(
            f"{label}: exported file sets differ: "
            f"interpreter={sorted(interp_tree)} cfx={sorted(cfx_tree)}"
        )
    if not interp_tree:
        fail(f"{label}: no exported files produced")
    for rel, data in interp_tree.items():
        if cfx_tree[rel] != data:
            fail(f"{label}: exported file '{rel}' differs between interpreter and cfx")

    # resolve_hash must match.
    if not cfx_hash:
        fail(f"{label}: cfx printed no resolve_hash")
    if interp_hash != cfx_hash:
        fail(
            f"{label}: resolve_hash differs: interpreter={interp_hash} cfx={cfx_hash}"
        )
    print(f"[cfx-diff] scenario {label}: OK (byte-identical, resolve_hash={cfx_hash})")
    return manifest, scenario_dir


def determinism(cfx, manifest, scope, context, workroot):
    print("[cfx-diff] determinism: two cfx runs must be byte-equal")
    sel_path = os.path.join(workroot, "determinism_selection.json")
    selection_file(sel_path, scope, context)
    out_a = os.path.join(workroot, "det_a")
    out_b = os.path.join(workroot, "det_b")
    proc_a = run([cfx, "resolve", "--model", manifest, "--selection-file", sel_path, "--out", out_a])
    proc_b = run([cfx, "resolve", "--model", manifest, "--selection-file", sel_path, "--out", out_b])
    if proc_a.stdout != proc_b.stdout:
        fail("determinism: cfx stdout differed between runs")
    if snapshot_tree(out_a) != snapshot_tree(out_b):
        fail("determinism: cfx exported files differed between runs")
    print("[cfx-diff] determinism: OK")


def normalize_hashes(text):
    return HASH_RE.sub("__HASH__", text)


def golden_happy(cfx, manifest, scope, context, workroot, golden_dir):
    print("[cfx-diff] golden: S1 happy-path stdout")
    sel_path = os.path.join(workroot, "golden_selection.json")
    selection_file(sel_path, scope, context)
    out_dir = os.path.join(workroot, "golden_out")
    proc = run([cfx, "resolve", "--model", manifest, "--selection-file", sel_path, "--out", out_dir])
    got = normalize_hashes(proc.stdout)
    with open(os.path.join(golden_dir, "resolve_s1_happy.stdout")) as handle:
        want = handle.read()
    if got != want:
        fail(f"golden happy-path mismatch:\n--- want ---\n{want}\n--- got ---\n{got}")
    print("[cfx-diff] golden happy-path: OK")


def golden_usage(cfx, manifest, workroot, golden_dir):
    print("[cfx-diff] exit code 2: malformed --select")
    out_dir = os.path.join(workroot, "usage_out")
    proc = run(
        [cfx, "resolve", "--model", manifest, "--out", out_dir, "--select", "bogus"],
        check=False,
    )
    if proc.returncode != 2:
        fail(f"usage error must exit 2, got {proc.returncode} (stderr: {proc.stderr})")
    with open(os.path.join(golden_dir, "resolve_usage.stderr")) as handle:
        want = handle.read()
    if proc.stderr != want:
        fail(f"golden usage stderr mismatch:\n--- want ---\n{want}\n--- got ---\n{proc.stderr}")
    print("[cfx-diff] exit code 2: OK")


def golden_unsat(cfx, manifest, scope, context, workroot, golden_dir):
    print("[cfx-diff] exit code 3: unsatisfiable selection (context-tag conflict)")
    # context has region=us; selecting region=eu conflicts with the immutable
    # context tag -> E_SELECTION_CONFLICT -> exit 3.
    if context.get("region") != "us":
        fail("unsat fixture expects context region=us")
    sel_path = os.path.join(workroot, "unsat_selection.json")
    selection_file(sel_path, scope, context)
    out_dir = os.path.join(workroot, "unsat_out")
    proc = run(
        [
            cfx,
            "resolve",
            "--model",
            manifest,
            "--selection-file",
            sel_path,
            "--out",
            out_dir,
            "--select",
            "region=eu",
        ],
        check=False,
    )
    if proc.returncode != 3:
        fail(f"unsatisfiable selection must exit 3, got {proc.returncode} (stderr: {proc.stderr})")
    # The guidance line echoes the selection file so it reproduces the refused
    # selection (configflux-0qk2); its tempdir path is normalized like the model.
    got = proc.stderr.replace(manifest, "__MODEL__").replace(sel_path, "__SELECTION_FILE__")
    with open(os.path.join(golden_dir, "resolve_unsat.stderr")) as handle:
        want = handle.read()
    if got != want:
        fail(f"golden unsat stderr mismatch:\n--- want ---\n{want}\n--- got ---\n{got}")
    print("[cfx-diff] exit code 3: OK")


def main():
    compiler = env_path("COMPILER")
    interpreter = env_path("INTERPRETER")
    cfx = env_path("CFX")
    s1_defs = env_path("S1_DEFS")
    s1_components = env_path("S1_COMPONENTS")
    s5_defs = env_path("S5_DEFS")
    s5_components = env_path("S5_COMPONENTS")
    golden_dir = env_path("GOLDEN_DIR")

    s1_scope = "component:thermal_control"
    s1_context = {
        "cooling_brand": "hydra",
        "cooling_model": "x200",
        "pump_type": "dual",
        "region": "us",
    }
    s5_scope = "component:climate_controller"
    s5_context = {
        "occupancy_class": "hospital",
        "filtration_grade": "hepa",
        "region": "us",
    }

    workroot = tempfile.mkdtemp(prefix="cfx-diff-", dir=os.environ.get("TEST_TMPDIR"))

    # Anchor: S1 + S5 byte-identical differential.
    s1_manifest, _ = differential(
        cfx, interpreter, compiler, "s1", s1_defs, s1_components, s1_scope, s1_context, workroot
    )
    differential(
        cfx, interpreter, compiler, "s5", s5_defs, s5_components, s5_scope, s5_context, workroot
    )

    # Determinism + exit codes + goldens use the already-compiled S1 model.
    determinism(cfx, s1_manifest, s1_scope, s1_context, workroot)
    golden_happy(cfx, s1_manifest, s1_scope, s1_context, workroot, golden_dir)
    golden_usage(cfx, s1_manifest, workroot, golden_dir)
    golden_unsat(cfx, s1_manifest, s1_scope, s1_context, workroot, golden_dir)

    print("[cfx-diff] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
