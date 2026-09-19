#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# End-to-end check that `cfx explain` attributes a forced conflict to a
# facet-to-facet equality constraint and quotes the AUTHORED condition text —
# configflux-secb.2 / ADR-0057 §D5.
#
# The claim under test is the one the feature would be worthless without: when
# two facets tied by `sorter_container == line_container` are bound to
# different containers, the operator is told WHICH named policy blocked it, in
# the words they wrote. The expansion into a pairwise equivalence happens
# inside the emitter, so a lowering that leaked the expanded form into the
# ADR-0054 §5.4 manifest roster would show up here as an unreadable machine
# formula in place of the authored line.
#
# Drives the REAL compiler and cfx binaries over the committed
# `s_facet_equality` scenario pack, and compares text output to a committed
# golden. Also pins the two exit codes: explaining a rejection is a successful
# query (exit 0, ADR-0031 D2), while a satisfiable selection has nothing to
# explain (exit 3).

import json
import os
import subprocess
import sys
import tempfile

# The pack has no component-scoped facets, so the selection scope is the whole
# model.
SCOPE = "all"

# Bound differently: the equality constraint forbids the pair.
CONFLICTING = [("line_container", "c1"), ("sorter_container", "c2")]
# Bound alike: nothing to explain.
AGREEING = [("line_container", "c2"), ("sorter_container", "c2")]

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


def compile_scenario(compiler, defs, components, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    proc = subprocess.run(
        [compiler, "compile", "--source", defs, "--source", components, "--out", out_dir],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        fail(f"compile failed ({proc.returncode}):\n{proc.stdout}\n{proc.stderr}")
    manifest = os.path.join(out_dir, "cmp.manifest.json")
    if not os.path.exists(manifest):
        fail(f"compiler did not emit {manifest}")
    return manifest


def write_selection_file(path):
    with open(path, "w") as handle:
        json.dump(
            {
                "schema_version": 5,
                "model_hash": "",
                "scope": SCOPE,
                "context_tags": {},
                "choices": {},
                "selection_state_hash": "",
            },
            handle,
        )


def cfx_explain(cfx, manifest, sel_path, selects, fmt):
    cmd = [cfx, "explain", "--model", manifest, "--selection-file", sel_path, "--format", fmt]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    return subprocess.run(cmd, capture_output=True, text=False)


def main():
    compiler = env_path("COMPILER")
    cfx = env_path("CFX")
    defs = env_path("PACK_DEFS")
    components = env_path("PACK_COMPONENTS")
    golden_path = env_path("GOLDEN_CONFLICT")

    with tempfile.TemporaryDirectory(prefix="cfx-explain-facet-equality-") as work:
        manifest = compile_scenario(compiler, defs, components, os.path.join(work, "cmp"))
        sel_path = os.path.join(work, "selection.json")
        write_selection_file(sel_path)

        print("[facet-equality] conflicting selection: text golden + exit code")
        proc = cfx_explain(cfx, manifest, sel_path, CONFLICTING, "text")
        if proc.returncode != EXIT_OK:
            fail(
                f"cfx explain exited {proc.returncode}, want {EXIT_OK}\n"
                f"{proc.stderr.decode(errors='replace')}"
            )
        actual = proc.stdout.decode()
        with open(golden_path) as handle:
            expected = handle.read()
        if actual != expected:
            fail(
                "cfx explain text output does not match the golden.\n"
                f"--- expected ---\n{expected}\n--- actual ---\n{actual}"
            )

        # The two substantive claims, asserted independently of the golden so a
        # careless golden refresh cannot quietly drop them.
        if "groups_equal" not in actual:
            fail(f"explain output must name the constraint:\n{actual}")
        if "sorter_container == line_container" not in actual:
            fail(
                "explain output must quote the AUTHORED condition, unquoted "
                f"right-hand side and all:\n{actual}"
            )

        print("[facet-equality] JSON envelope attributes the same constraint")
        proc = cfx_explain(cfx, manifest, sel_path, CONFLICTING, "json")
        if proc.returncode != EXIT_OK:
            fail(f"cfx explain --format json exited {proc.returncode}, want {EXIT_OK}")
        envelope = json.loads(proc.stdout)
        blob = json.dumps(envelope)
        if "groups_equal" not in blob:
            fail(f"explain envelope must carry the constraint id:\n{blob}")
        if "sorter_container == line_container" not in blob:
            fail(f"explain envelope must carry the authored condition:\n{blob}")

        print("[facet-equality] agreeing selection has nothing to explain")
        proc = cfx_explain(cfx, manifest, sel_path, AGREEING, "text")
        if proc.returncode != EXIT_UNSAT:
            fail(
                f"a satisfiable selection must exit {EXIT_UNSAT}, got "
                f"{proc.returncode}\n{proc.stdout.decode(errors='replace')}"
            )

    print("[facet-equality] OK")


if __name__ == "__main__":
    main()
