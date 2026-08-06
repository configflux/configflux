#!/usr/bin/env python3
# SPDX-License-Identifier: BUSL-1.1
#
# Consistency check: `cfx resolve` and `cfx explain` must AGREE on satisfiability
# for identical input (configflux-sc69). The regression this pins is the
# scope=all resolve-context path: at scope=all a selection of
# cooling_brand+cooling_model+pump_type (region omitted) makes the S1 `eu_label`
# component's `region == 'eu'` condition unevaluable, so `cfx resolve` reports the
# selection unsatisfiable (E_RESOLVE_CONTEXT_UNSATISFIED) and points the user at
# `cfx explain`. Before the fix, `cfx explain` — which only runs the solver's
# per-choice BDD apply gate — saw no conflicting choice and answered "satisfiable;
# nothing to explain", a dead-end remediation path.
#
# After the fix, `cfx explain` runs the SAME resolve-context check `cfx resolve`
# performs and, on the same unsat verdict, EXPLAINS it: it does not claim the
# selection is satisfiable, it exits 0 (an explanation is the success path,
# ADR-0042 §3 / ADR-0031 D2), and it names the unbound tag (`region`). A control
# selection that binds `region` must have BOTH verbs agree the selection is
# satisfiable. `--format json` emits the existing `ResolveResult` schema (the
# resolve failure envelope), and repeated runs are byte-identical.
#
# The same agreement is asserted one level up, on the REMEDIATION PATH
# (configflux-0qk2): the "run: cfx explain ..." command `cfx resolve` prints on a
# rejection must reproduce the selection it just refused. When part of that
# selection arrives via `--selection-file`, a hint rebuilt from the `--select`
# flags alone describes a different selection, so the two verbs contradict each
# other through the pointer even though each is individually correct.
#
# Drives the REAL compiler / cfx binaries. Binaries and fixtures come from env
# rlocations resolved by the sh_test wrapper. Standard library only.

import json
import os
import subprocess
import sys
import tempfile

# scope=all is the default when no --selection-file pins a scope; the repro is a
# scope=all resolve-context divergence, so every cfx call here omits the file.
UNSAT_SELECTS = [
    ("cooling_brand", "hydra"),
    ("cooling_model", "x200"),
    ("pump_type", "dual"),
]
# The same selection plus a binding for the tag the `eu_label` condition needs.
SAT_SELECTS = UNSAT_SELECTS + [("region", "eu")]
# The tag the S1 `eu_label` condition references and this selection leaves unbound.
UNBOUND_TAG = "region"

# configflux-0qk2. Each case splits a rejected selection across the two input
# channels — part in the --selection-file, part in --select flags — so a hint
# rebuilt from the flags alone NECESSARILY describes a different selection. The
# two cases cover the two ways that goes wrong, and `expect` is the token the
# suggested command must produce to prove it explained the same rejection.
HINT_CASES = (
    {
        # Hero example. The file carries a CHOICE; the model's prod_forbids_debug
        # rule needs both halves, so dropping the file leaves a satisfiable
        # selection and the bad hint answers "nothing to explain".
        "label": "hero/policy-constraint",
        "model": "hero",
        "scope": "all",
        "context_tags": {},
        "choices": {"log_level": "debug"},
        "selects": [("environment", "prod")],
        "expect": "prod_forbids_debug",
    },
    {
        # S1. The file carries the SCOPE and immutable CONTEXT TAGS, which no
        # --select pair can express. Dropping it does not make the selection
        # satisfiable — it makes the model fail for an unrelated reason, so the
        # bad hint confidently explains the WRONG failure.
        "label": "s1/immutable-context-tag",
        "model": "s1",
        "scope": "component:thermal_control",
        "context_tags": {
            "cooling_brand": "hydra",
            "cooling_model": "x200",
            "pump_type": "dual",
            "region": "us",
        },
        "choices": {},
        "selects": [("region", "eu")],
        "expect": "immutable",
    },
)

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
        fail(f"compile failed ({proc.returncode}): {proc.stderr}")
    manifest = os.path.join(out_dir, "cmp.manifest.json")
    if not os.path.exists(manifest):
        fail(f"compiler did not emit {manifest}")
    return manifest


def cfx_resolve(cfx, manifest, selects, out_dir):
    cmd = [cfx, "resolve", "--model", manifest, "--out", out_dir]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    return subprocess.run(cmd, capture_output=True, text=True)


def cfx_explain(cfx, manifest, selects, fmt):
    cmd = [cfx, "explain", "--model", manifest, "--format", fmt]
    for facet, option in selects:
        cmd += ["--select", f"{facet}={option}"]
    return subprocess.run(cmd, capture_output=True, text=False)


def check_unsat_agreement(cfx, manifest, work):
    print("[cfx-consistency] unsat: resolve and explain must AGREE it is unsatisfiable")

    resolve = cfx_resolve(cfx, manifest, UNSAT_SELECTS, os.path.join(work, "unsat_out"))
    if resolve.returncode != EXIT_UNSAT:
        fail(
            f"unsat: cfx resolve exited {resolve.returncode}, want {EXIT_UNSAT}\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    if "unsatisfiable" not in resolve.stderr:
        fail(f"unsat: cfx resolve stderr must report unsatisfiable; got {resolve.stderr!r}")
    print("[cfx-consistency] unsat: cfx resolve reports unsatisfiable (exit 3) OK")

    explain = cfx_explain(cfx, manifest, UNSAT_SELECTS, "text")
    out = explain.stdout.decode(errors="replace")
    err = explain.stderr.decode(errors="replace")
    # The core regression: explain must NOT contradict resolve by claiming the
    # selection is satisfiable / that there is nothing to explain.
    if "nothing to explain" in out or "selection is satisfiable" in out:
        fail(
            "unsat: cfx explain contradicts cfx resolve — it reported the selection "
            f"satisfiable while resolve reported it unsatisfiable.\nstdout={out!r}"
        )
    # An explanation is the success path (ADR-0042 §3): exit 0.
    if explain.returncode != EXIT_OK:
        fail(
            f"unsat: cfx explain exited {explain.returncode}, want {EXIT_OK}\n"
            f"stdout={out!r}\nstderr={err!r}"
        )
    # It must actually explain: name the unbound tag the condition needs.
    if UNBOUND_TAG not in out:
        fail(f"unsat: cfx explain must name the unbound tag '{UNBOUND_TAG}'; got {out!r}")
    print("[cfx-consistency] unsat: cfx explain explains it and names the tag (exit 0) OK")


def check_sat_agreement(cfx, manifest, work):
    print("[cfx-consistency] sat: resolve and explain must AGREE it is satisfiable")

    resolve = cfx_resolve(cfx, manifest, SAT_SELECTS, os.path.join(work, "sat_out"))
    if resolve.returncode != EXIT_OK:
        fail(
            f"sat: cfx resolve exited {resolve.returncode}, want {EXIT_OK}\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )

    explain = cfx_explain(cfx, manifest, SAT_SELECTS, "text")
    out = explain.stdout.decode(errors="replace")
    if explain.returncode != EXIT_UNSAT:
        fail(
            f"sat: cfx explain exited {explain.returncode}, want {EXIT_UNSAT} "
            f"(satisfiable -> nothing to explain)\nstdout={out!r}"
        )
    if "nothing to explain" not in out:
        fail(f"sat: cfx explain must report nothing to explain; got {out!r}")
    print("[cfx-consistency] sat: both agree the selection is satisfiable OK")


def check_json_and_determinism(cfx, manifest):
    print("[cfx-consistency] unsat json: existing ResolveResult schema + determinism")

    a = cfx_explain(cfx, manifest, UNSAT_SELECTS, "json")
    b = cfx_explain(cfx, manifest, UNSAT_SELECTS, "json")
    if a.returncode != EXIT_OK:
        fail(
            f"unsat json: cfx explain --format json exited {a.returncode}, want {EXIT_OK}\n"
            f"stderr={a.stderr.decode(errors='replace')!r}"
        )
    if a.stdout != b.stdout:
        fail("unsat json: cfx explain --format json differed between runs (non-deterministic)")
    parsed = json.loads(a.stdout)
    if parsed.get("status") != "error":
        fail(f"unsat json: expected the resolve failure envelope (status error); got {a.stdout!r}")
    codes = [d.get("code") for d in parsed.get("diagnostics", {}).get("diagnostics", [])]
    if "E_RESOLVE_CONTEXT_UNSATISFIED" not in codes:
        fail(f"unsat json: expected E_RESOLVE_CONTEXT_UNSATISFIED in diagnostics; got {codes!r}")

    t1 = cfx_explain(cfx, manifest, UNSAT_SELECTS, "text").stdout
    t2 = cfx_explain(cfx, manifest, UNSAT_SELECTS, "text").stdout
    if t1 != t2:
        fail("unsat text: cfx explain text differed between runs (non-deterministic)")
    print("[cfx-consistency] unsat json: OK (ResolveResult schema, deterministic)")


def write_selection_file(path, case):
    """A SelectionState JSON carrying the case's scope, context tags and base
    choices. cfx re-derives the hashes, so those fields are placeholders."""
    with open(path, "w") as handle:
        json.dump(
            {
                "schema_version": 4,
                "model_hash": "",
                "scope": case["scope"],
                "context_tags": case["context_tags"],
                "choices": case["choices"],
                "selection_state_hash": "",
            },
            handle,
        )


def suggested_command(stderr):
    """Extract the argv from the unsat guidance line's `run: ...` suffix."""
    for line in stderr.splitlines():
        if not line.startswith("selection is unsatisfiable"):
            continue
        _, marker, command = line.partition("run: ")
        if marker:
            return command.strip().split()
    return None


def check_hint_reproduces_selection(cfx, manifests, work, case):
    """configflux-0qk2: resolve's "run: cfx explain ..." pointer must reproduce
    the selection it just refused.

    This is the resolve/explain agreement the checks above assert, at the level
    of the REMEDIATION PATH: resolve is entitled to refuse a selection, but the
    command it hands the user must not ask about a different one. Asserted on
    BEHAVIOR, not on flag spelling — the suggested command is parsed out of
    stderr and run verbatim, so any hint that reproduces the refused selection
    passes, whichever way it encodes it."""
    label = case["label"]
    print(f"[cfx-consistency] hint {label}: suggested command must explain the same selection")

    sel_path = os.path.join(work, f"hint_{case['model']}_selection.json")
    write_selection_file(sel_path, case)
    cmd = [
        cfx,
        "resolve",
        "--model",
        manifests[case["model"]],
        "--selection-file",
        sel_path,
        "--out",
        os.path.join(work, f"hint_{case['model']}_out"),
    ]
    for facet, option in case["selects"]:
        cmd += ["--select", f"{facet}={option}"]
    resolve = subprocess.run(cmd, capture_output=True, text=True)

    if resolve.returncode != EXIT_UNSAT:
        fail(
            f"hint {label}: cfx resolve exited {resolve.returncode}, want {EXIT_UNSAT}\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    if case["expect"] not in resolve.stderr:
        fail(
            f"hint {label}: cfx resolve must name '{case['expect']}' as the reason; "
            f"got {resolve.stderr!r}"
        )

    argv = suggested_command(resolve.stderr)
    if not argv:
        fail(f"hint {label}: unsat stderr carried no `run: ...` guidance line: {resolve.stderr!r}")
    if argv[0] != "cfx":
        fail(f"hint {label}: guidance line must suggest a cfx command; got {argv!r}")
    # The printed `cfx` is the user's PATH binary; run the one under test.
    argv[0] = cfx

    suggested = subprocess.run(argv, capture_output=True, text=True)
    combined = suggested.stdout + suggested.stderr
    printed = " ".join(argv)
    if "nothing to explain" in combined or "selection is satisfiable" in combined:
        fail(
            f"hint {label}: the suggested command describes a DIFFERENT selection than the "
            "one resolve refused — run verbatim it reports that selection satisfiable.\n"
            f"suggested: {printed}\nstdout={suggested.stdout!r}\nstderr={suggested.stderr!r}"
        )
    if suggested.returncode == EXIT_UNSAT:
        fail(
            f"hint {label}: the suggested command exited {EXIT_UNSAT} (satisfiable; nothing "
            f"to explain) — it does not explain the refused selection.\nsuggested: {printed}\n"
            f"stdout={suggested.stdout!r}\nstderr={suggested.stderr!r}"
        )
    # The strongest form of "explains the SAME selection": it must name the very
    # reason resolve gave, not some other unsatisfiability the model has once
    # the selection file is dropped.
    if case["expect"] not in combined:
        fail(
            f"hint {label}: the suggested command explains a different failure — resolve "
            f"gave '{case['expect']}' as the reason but the explanation never mentions it.\n"
            f"resolve stderr={resolve.stderr!r}\nsuggested: {printed}\n"
            f"stdout={suggested.stdout!r}\nstderr={suggested.stderr!r}"
        )
    print(f"[cfx-consistency] hint {label}: OK (explains the refused selection)")


def main():
    compiler = env_path("COMPILER")
    cfx = env_path("CFX")
    defs = env_path("S1_DEFS")
    components = env_path("S1_COMPONENTS")

    workroot = tempfile.mkdtemp(prefix="cfx-consistency-", dir=os.environ.get("TEST_TMPDIR"))
    manifest = compile_scenario(compiler, defs, components, os.path.join(workroot, "cmp"))

    check_unsat_agreement(cfx, manifest, workroot)
    check_sat_agreement(cfx, manifest, workroot)
    check_json_and_determinism(cfx, manifest)

    manifests = {
        "s1": manifest,
        "hero": compile_scenario(
            compiler,
            env_path("HERO_DEFS"),
            env_path("HERO_COMPONENTS"),
            os.path.join(workroot, "hero_cmp"),
        ),
    }
    for case in HINT_CASES:
        check_hint_reproduces_selection(cfx, manifests, workroot, case)

    print("[cfx-consistency] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
