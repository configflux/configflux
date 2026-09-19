#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# End-to-end check that a component's `requires` / `accepts` and a binding's
# `derive` table reach the operator as rules with names — configflux-secb.5 /
# ADR-0057 §D4.
#
# Three claims, and the feature is worthless without any of them:
#
#   1. An entry no component accepts is never OFFERED. `cfx options` over the
#      whole model must list the two containers `compute_service` accepts and
#      not the third.
#   2. A `derive` table DECIDES. Naming the site must collapse the binding to
#      the entry the table fixes, without the environment repeating it.
#   3. A rejection is EXPLAINED BY NAME. `cfx explain` must say which derive
#      rule or which requirement ruled the choice out, in the vocabulary the
#      author used — a binding and a site, or a component and a slot — never as
#      the machine implication the model actually asserts.
#
# The pack is three authoring UNITS (ADR-0057 §D1): the plant's catalogue and
# facets, the line's binding, and the services. Nothing in the chain would
# notice if they were one file, which is why they are three — a requirement
# whose binding is declared in another unit is the normal shape, and it is what
# ADR-0058's linker will have to resolve from headers alone.
#
# `edge_service` carries a `condition` on purpose. ADR-0054 §3 says a component
# that is not included asserts nothing, so its `accepts: [c1]` must narrow the
# binding only where `mode == 'x'` holds. A lowering that dropped the guard
# would let an excluded component veto c2 for everybody, and the two `mode`
# cases below are what catch it.
#
# Drives the REAL compiler and cfx binaries over the committed
# `s_requires_accepts` pack and compares text output to committed goldens.

import json
import os
import subprocess
import sys
import tempfile

# The pack has no component-scoped facets, so the selection scope is the whole
# model.
SCOPE = "all"
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


def compile_scenario(compiler, sources, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    cmd = [compiler, "compile"]
    for source in sources:
        cmd += ["--source", source]
    cmd += ["--out", out_dir]
    proc = subprocess.run(cmd, capture_output=True, text=True)
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


def cfx(binary, verb, manifest, sel_path, selects=(), fmt="text", extra=()):
    cmd = [binary, verb, "--model", manifest, "--selection-file", sel_path, "--format", fmt]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    cmd += list(extra)
    return subprocess.run(cmd, capture_output=True, text=True)


def options_for(binary, manifest, sel_path, facet, selects=()):
    """The offered options of one facet, read out of the JSON envelope."""
    proc = cfx(binary, "options", manifest, sel_path, selects, "json")
    if proc.returncode != EXIT_OK:
        fail(f"cfx options exited {proc.returncode}, want {EXIT_OK}\n{proc.stderr}")
    envelope = json.loads(proc.stdout)
    for entry in envelope if isinstance(envelope, list) else [envelope]:
        if entry.get("facet") == facet:
            return entry.get("valid_options", [])
    fail(f"cfx options envelope carries no facet '{facet}':\n{proc.stdout}")


def compare_golden(actual, golden_path, label):
    with open(golden_path) as handle:
        expected = handle.read()
    if actual != expected:
        fail(
            f"{label} does not match its golden.\n"
            f"--- expected ---\n{expected}\n--- actual ---\n{actual}"
        )


def check_options(binary, manifest, sel_path, golden_site):
    print("[requires-accepts] an entry no requirement accepts is never offered")
    offered = options_for(binary, manifest, sel_path, "line_container")
    if offered != ["c1", "c2"]:
        fail(f"want line_container [c1, c2] with nothing selected, got {offered}")

    print("[requires-accepts] a derive table decides the binding from the site")
    proc = cfx(binary, "options", manifest, sel_path, [("site", "factory_a")], "text")
    if proc.returncode != EXIT_OK:
        fail(f"cfx options exited {proc.returncode}, want {EXIT_OK}\n{proc.stderr}")
    compare_golden(proc.stdout, golden_site, "cfx options --select site=factory_a")
    offered = options_for(
        binary, manifest, sel_path, "line_container", [("site", "factory_a")]
    )
    if offered != ["c1"]:
        fail(f"want line_container [c1] at factory_a, got {offered}")

    # The other covered site, so the golden above cannot pass by naming the
    # first catalogue entry for every input.
    offered = options_for(
        binary, manifest, sel_path, "line_container", [("site", "factory_b")]
    )
    if offered != ["c2"]:
        fail(f"want line_container [c2] at factory_b, got {offered}")

    # factory_c is absent from the derive table: a partial table implies
    # nothing about the values it omits (ADR-0057 §D3).
    offered = options_for(
        binary, manifest, sel_path, "line_container", [("site", "factory_c")]
    )
    if offered != ["c1", "c2"]:
        fail(f"an uncovered site must decide nothing, got {offered}")


def check_conditional_accepts(binary, manifest, sel_path):
    print("[requires-accepts] a conditional component's accepts list is guarded")
    included = options_for(
        binary, manifest, sel_path, "line_container", [("mode", "x")]
    )
    if included != ["c1"]:
        fail(f"with the edge service included want [c1], got {included}")
    excluded = options_for(
        binary, manifest, sel_path, "line_container", [("mode", "y")]
    )
    if excluded != ["c1", "c2"]:
        fail(
            "with the edge service excluded its accepts list must assert "
            f"nothing; want [c1, c2], got {excluded}"
        )


def check_explain(binary, manifest, sel_path, golden_derive, golden_accepts):
    print("[requires-accepts] a derive rejection is explained as the derive rule")
    proc = cfx(
        binary,
        "explain",
        manifest,
        sel_path,
        [("site", "factory_a"), ("line_container", "c2")],
    )
    if proc.returncode != EXIT_OK:
        fail(f"cfx explain exited {proc.returncode}, want {EXIT_OK}\n{proc.stderr}")
    compare_golden(proc.stdout, golden_derive, "cfx explain (derive)")
    # Asserted independently of the golden so a careless refresh cannot quietly
    # drop the two facts that matter.
    if "binding line_container, derived from site" not in proc.stdout:
        fail(f"the derive rejection must name the binding and its source:\n{proc.stdout}")
    if "'factory_a' -> 'c1'" not in proc.stdout:
        fail(f"the derive rejection must name the pair that forced it:\n{proc.stdout}")

    print("[requires-accepts] an accepts rejection is explained as the requirement")
    proc = cfx(binary, "explain", manifest, sel_path, [("line_container", "c3")])
    if proc.returncode != EXIT_OK:
        fail(f"cfx explain exited {proc.returncode}, want {EXIT_OK}\n{proc.stderr}")
    compare_golden(proc.stdout, golden_accepts, "cfx explain (accepts)")
    if "requirement compute_service.container: accepts c1, c2" not in proc.stdout:
        fail(f"the accepts rejection must name the requirement and its list:\n{proc.stdout}")

    print("[requires-accepts] the JSON envelope carries the attribution id")
    proc = cfx(binary, "explain", manifest, sel_path, [("line_container", "c3")], "json")
    if proc.returncode != EXIT_OK:
        fail(f"cfx explain --format json exited {proc.returncode}, want {EXIT_OK}")
    blob = json.dumps(json.loads(proc.stdout))
    if "accepts:compute_service.container" not in blob:
        fail(f"the envelope must carry the attribution id verbatim:\n{blob}")


def check_resolve_agreement(binary, manifest, sel_path, work):
    """`cfx resolve` must reject on exactly the rules `cfx explain` names."""
    print("[requires-accepts] resolve refuses a forced violation and writes nothing")
    for selects, expected_id in (
        ([("site", "factory_a"), ("line_container", "c2")], "derive:line_container:site=factory_a"),
        ([("line_container", "c3")], "accepts:compute_service.container"),
    ):
        out = os.path.join(work, "snapshot.json")
        if os.path.exists(out):
            os.remove(out)
        proc = cfx(binary, "resolve", manifest, sel_path, selects, "text", ["--out", out])
        if proc.returncode != EXIT_UNSAT:
            fail(
                f"resolve of {selects} exited {proc.returncode}, want {EXIT_UNSAT}\n"
                f"{proc.stdout}\n{proc.stderr}"
            )
        if "E_SELECTION_CONFLICT" not in proc.stderr:
            fail(f"resolve must reject with E_SELECTION_CONFLICT:\n{proc.stderr}")
        if expected_id not in proc.stderr:
            fail(f"resolve must name '{expected_id}':\n{proc.stderr}")
        if os.path.exists(out):
            fail(f"no snapshot may be written on a rejected resolve; {out} exists")

    print("[requires-accepts] resolve infers the derived binding on a legal selection")
    out = os.path.join(work, "ok.json")
    proc = cfx(
        binary, "resolve", manifest, sel_path, [("site", "factory_a")], "json", ["--out", out]
    )
    if proc.returncode != EXIT_OK:
        fail(f"resolve exited {proc.returncode}, want {EXIT_OK}\n{proc.stderr}")
    envelope = json.loads(proc.stdout)
    implied = envelope.get("implied_choices") or {}
    if implied.get("line_container") != "c1":
        fail(
            "naming the site must imply the container the derive table fixes; "
            f"implied_choices={implied}"
        )


def main():
    compiler = env_path("COMPILER")
    binary = env_path("CFX")
    sources = [
        env_path("PACK_CATALOGUE"),
        env_path("PACK_BINDINGS"),
        env_path("PACK_COMPONENTS"),
    ]
    golden_site = env_path("GOLDEN_OPTIONS_SITE")
    golden_derive = env_path("GOLDEN_EXPLAIN_DERIVE")
    golden_accepts = env_path("GOLDEN_EXPLAIN_ACCEPTS")

    with tempfile.TemporaryDirectory(prefix="cfx-requires-accepts-") as work:
        manifest = compile_scenario(compiler, sources, os.path.join(work, "cmp"))
        sel_path = os.path.join(work, "selection.json")
        write_selection_file(sel_path)

        check_options(binary, manifest, sel_path, golden_site)
        check_conditional_accepts(binary, manifest, sel_path)
        check_explain(binary, manifest, sel_path, golden_derive, golden_accepts)
        check_resolve_agreement(binary, manifest, sel_path, work)

    print("[requires-accepts] OK")


if __name__ == "__main__":
    main()
