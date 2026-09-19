#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# End-to-end check that a BINDING is offered by `cfx options` exactly as the
# facet it is — configflux-secb.4 / ADR-0057 §D3.
#
# The claim under test is the sentence the whole design rests on: "a binding is
# a declared closed facet whose values are the catalogue's entry ids, and
# nothing downstream special-cases it". If that sentence is false anywhere in
# the chain, this test is where it shows: the entries reach the offered list
# only because the binding is projected into the compiler's declared facets and
# so gets `.ccm` symbols and `exactly_one_of` cardinality, and the
# `[closed, default: c2]` label appears only because the loader seeds
# `facet_open` and `facet_defaults` from the same projection.
#
# The pack declares NO facets at all, so every option below comes from the
# catalogue. The default is the SECOND entry on purpose: a default read off
# entry order rather than the declaration would still print `c1` and pass.
#
# Drives the REAL compiler and cfx binaries over the committed
# `s_catalogue_binding` pack, and compares text output to a committed golden.

import json
import os
import subprocess
import sys
import tempfile

SCOPE = "all"
EXIT_OK = 0


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


def cfx_options(cfx, manifest, sel_path, fmt, selects=()):
    cmd = [cfx, "options", "--model", manifest, "--selection-file", sel_path, "--format", fmt]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    return subprocess.run(cmd, capture_output=True, text=True)


def assert_binding_lists_as_a_closed_facet(cfx, manifest, sel_path, golden_path):
    """REQ-CFX-008: the binding appears in the listing as the facet it is."""
    print("[catalogue-binding] text listing matches the golden")
    proc = cfx_options(cfx, manifest, sel_path, "text")
    if proc.returncode != EXIT_OK:
        fail(f"cfx options exited {proc.returncode}, want {EXIT_OK}\n{proc.stderr}")
    with open(golden_path) as handle:
        expected = handle.read()
    if proc.stdout != expected:
        fail(
            "cfx options text output does not match the golden.\n"
            f"--- expected ---\n{expected}\n--- actual ---\n{proc.stdout}"
        )

    # Asserted independently of the golden so a careless refresh cannot quietly
    # drop either half of the claim.
    if "[closed, default: c2]" not in proc.stdout:
        fail(
            "a binding must render as a CLOSED facet carrying its declared "
            f"default:\n{proc.stdout}"
        )

    print("[catalogue-binding] JSON offers the entries in catalogue order")
    proc = cfx_options(cfx, manifest, sel_path, "json")
    if proc.returncode != EXIT_OK:
        fail(f"cfx options --format json exited {proc.returncode}\n{proc.stderr}")
    listings = json.loads(proc.stdout)
    by_facet = {entry["facet"]: entry for entry in listings}
    if list(by_facet) != ["line_container"]:
        fail(f"the pack declares one binding and no facets, got: {list(by_facet)}")
    options = by_facet["line_container"]["valid_options"]
    if options != ["c1", "c2", "c3"]:
        fail(f"entries must be offered in catalogue order, got: {options}")


def assert_only_catalogue_entries_are_selectable(cfx, manifest, sel_path):
    """REQ-CFX-009: the binding's domain is exactly the catalogue's entries."""
    print("[catalogue-binding] a catalogue entry is selectable")
    proc = cfx_options(cfx, manifest, sel_path, "text", [("line_container", "c1")])
    if proc.returncode != EXIT_OK:
        fail(f"selecting entry c1 exited {proc.returncode}\n{proc.stderr}")
    if "[selected: c1, default: c2]" not in proc.stdout:
        fail(f"a selected binding must render like a selected facet:\n{proc.stdout}")

    print("[catalogue-binding] a value outside the catalogue is refused")
    proc = cfx_options(cfx, manifest, sel_path, "text", [("line_container", "c9")])
    if proc.returncode == EXIT_OK:
        fail(
            "an entry the catalogue does not contain must not be selectable:\n"
            f"{proc.stdout}"
        )
    combined = proc.stdout + proc.stderr
    if "c9" not in combined:
        fail(f"the diagnostic must name the rejected value:\n{combined}")


def main():
    compiler = env_path("COMPILER")
    cfx = env_path("CFX")
    defs = env_path("PACK_DEFS")
    components = env_path("PACK_COMPONENTS")
    golden_path = env_path("GOLDEN_OPTIONS")

    with tempfile.TemporaryDirectory(prefix="cfx-options-catalogue-binding-") as work:
        manifest = compile_scenario(compiler, defs, components, os.path.join(work, "cmp"))
        sel_path = os.path.join(work, "selection.json")
        write_selection_file(sel_path)

        assert_binding_lists_as_a_closed_facet(cfx, manifest, sel_path, golden_path)
        assert_only_catalogue_entries_are_selectable(cfx, manifest, sel_path)

    print("[catalogue-binding] OK")


if __name__ == "__main__":
    main()
