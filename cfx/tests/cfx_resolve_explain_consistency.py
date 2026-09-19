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
# The same agreement is asserted a third time on SOLVER-INFERRED BINDING
# (configflux-secb.3, ADR-0057 §D6). A selection that FORCES a second facet is
# satisfiable, and both verbs have to say so — which explain can only do if its
# resolve-context check runs through the `session_compose` seam that performs
# the inference. See `check_implied_agreement`.
#
# Wherever inference fires, that agreement is asserted across all THREE verbs
# (configflux-cvey). `cfx options` never runs the §D6 fixpoint: it asks the
# solver session for each facet's still-valid set. So `options` and `resolve`
# agreeing about a forced facet is two independent computations over one model
# landing on the same answer, and only a check that drives both can say they
# still do. `check_inference_agreement` states it as an equivalence — options
# leaves exactly one value standing for a declared closed facet the user did not
# name if and only if resolve implies that same value — and both directions are
# defects a user meets: a listing narrowed to a value resolve will not bind, or
# a guided walk still asking a question resolve has already answered.
#
# And a fourth time on the LOWERED CONJUNCTS (configflux-secb.5, ADR-0057 §D4).
# A binding's `derive` table and a requirement's `accepts` list are not authored
# constraints; they become root conjuncts through a lowering, and each of the
# three surfaces reads that lowering separately — the emitter for the `.ccm`
# explain queries, and the resolve loader for its own fail-closed evaluation. A
# lowering applied in one place and not the other would leave resolve rejecting
# what explain calls fine, or the reverse. See `check_lowered_agreement`, which
# drives both verbs over the same two rejections and the same legal selection.
#
# Drives the REAL compiler / cfx binaries. Binaries and fixtures come from env
# rlocations resolved by the sh_test wrapper. Standard library only.

import json
import os
import shlex
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
    {
        # configflux-zz2g. The hero rejection again, reached through paths that
        # carry shell metacharacters -- a space and a single quote in both the
        # model directory and the selection file's. The hint is a command the
        # user is told to PASTE; unquoted, a shell re-splits these paths and the
        # pasted line addresses files that do not exist. Neither case above can
        # see that: their tempdir paths are metacharacter-free, so the printed
        # form and the parsed form happen to coincide.
        "label": "hero/copy-pasteable-path",
        "model": "metachar",
        "scope": "all",
        "context_tags": {},
        "choices": {"log_level": "debug"},
        "selects": [("environment", "prod")],
        "expect": "prod_forbids_debug",
        "subdir": "selection dir 'x' with spaces",
    },
    {
        # configflux-egyj. The S1 rejection again, this time through the term
        # zz2g deliberately left raw. A `--select` pair is not a path, but that
        # is not what made the paths dangerous -- a space was. Facet ids and
        # option ids are plain strings on every input this rejection reads (the
        # model's `values` list, and this selection file's context tags), so the
        # metacharacter arrives through the same channel a real user's model or
        # CI config does. The case above cannot see it: its pair is
        # `environment=prod`, and the printed form and the parsed form coincide
        # by luck for anything metacharacter-free.
        "label": "s1/copy-pasteable-select",
        "model": "s1",
        "scope": "component:thermal_control",
        "context_tags": {
            "cooling_brand": "hydra",
            "cooling_model": "x200",
            "pump_type": "dual",
            "region": "us",
            # The immutable tag the pair below contradicts. Its id carries the
            # space, so the metacharacter is on BOTH sides of the `=` and a
            # quoting rule that only wrapped the value would still fail here.
            "re gion": "us",
        },
        "choices": {},
        "selects": [("re gion", "eu 'west'")],
        "expect": "immutable",
    },
)

# configflux-secb.3. The `s_facet_equality` pack declares two closed facets over
# {c1, c2}, both defaulting to c1, tied by `groups_equal`:
# `sorter_container == line_container`. Choosing the NON-default value for one
# leaves the other exactly one valid option, so the constraints decide it.
IMPLIED_SELECTS = [("line_container", "c2")]
IMPLIED_EXPECTED = {"sorter_container": "c2"}
# The constraint explain used to blame when it resolved without the implication.
IMPLIED_CONSTRAINT = "groups_equal"

# configflux-cvey. The shape ADR-0057 §D6 names in its OWN context section
# ("For factory A it is container 1"): the `s_requires_accepts` pack's `derive`
# table maps the site to the container, so `--select site=factory_b` alone
# decides `line_container`. `mode` follows from THAT — the edge service is the
# only consumer whose `accepts` refuses c2, and its `mode == 'x'` condition is
# what would activate it — so this one input implies two facets, the second a
# consequence of the first. The pack reaches the decision through a lowering
# rather than through an authored constraint, which is a different road to the
# same seam than the `s_facet_equality` case above takes.
DERIVED_SELECTS = [("site", "factory_b")]
DERIVED_EXPECTED = {"line_container": "c2", "mode": "y"}
# The control, and the reason this case cannot pass by accident: the SAME pack
# and the SAME two facets, one site over. The derive table decides
# `line_container` again, but nothing forces `mode` — resolve DEFAULTS it. A
# listing that merely narrowed every unbound facet to one value would agree with
# the case above and disagree here, so this is what makes "options offers a
# single value exactly where resolve infers one" a claim with content.
DERIVED_CONTROL_SELECTS = [("site", "factory_a")]
DERIVED_CONTROL_EXPECTED = {"line_container": "c1"}

# configflux-rzyd. The SAME `s_requires_accepts` pack at the two selections that
# separate "impossible" from "not said enough yet" — the pair `cfx resolve` used
# to collapse into one exit code, in the direction that hid the cause.
#
# Both leave `line_container` with no value: a `derive` table decides it, so it
# has no default to fall back on, and neither selection names it. What differs
# is WHY. Under `site=factory_b` the table decides c2 while `mode=x` activates
# the edge service, whose `accepts` takes only c1 — the selection is impossible
# and no value of the binding can rescue it. Under `site=factory_c` the table
# says nothing (that site is deliberately uncovered) and nothing else does
# either — the model is satisfiable the moment the binding is named.
#
# ADR-0042 §3 gives those different exit codes, and the difference IS the
# remediation path: 3 says the selection cannot hold, run explain; 2 says bind
# this facet. Reporting the first as the second sent the reader to add a default
# to a binding that already had a derive table — advice that cannot work.
UNSAT_DERIVED_SELECTS = [("site", "factory_b"), ("mode", "x")]
UNDERSPECIFIED_SELECTS = [("site", "factory_c")]
UNBOUND_BINDING = "line_container"
# The phrase the old rejection used to assert as the CAUSE. True of the binding,
# and never why either of these selections failed.
FALSE_CAUSE = "has no default"

# The `implied: facet=value` lines `cfx resolve` prints (ADR-0057 §D6).
IMPLIED_PREFIX = "implied: "

EXIT_OK = 0
# ADR-0042 §3: valid input the tool could not act on (a bad invocation, or a
# selection that has not said enough yet).
EXIT_USAGE = 2
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


def compile_sources(compiler, sources, out_dir):
    """As `compile_scenario`, for a pack of more than two chunks. The
    configflux-secb.5 fixture is three authoring units (ADR-0057 §D1)."""
    os.makedirs(out_dir, exist_ok=True)
    cmd = [compiler, "compile"]
    for source in sources:
        cmd += ["--source", source]
    cmd += ["--out", out_dir]
    proc = subprocess.run(cmd, capture_output=True, text=True)
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


def cfx_options(cfx, manifest, selection_file, fmt, selects=()):
    cmd = [cfx, "options", "--model", manifest, "--format", fmt]
    if selection_file:
        cmd += ["--selection-file", selection_file]
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


def resolve_snapshot(out_dir):
    """The `resolve_result.<scope>.<tag>.json` `cfx resolve --out` writes. Found
    by listing rather than by rebuilding the name: the name encodes the scope and
    the explicit choices, and a test that re-derived that rule would pass while
    disagreeing with the rule the product applies."""
    names = sorted(
        name
        for name in os.listdir(out_dir)
        if name.startswith("resolve_result.") and name.endswith(".json")
    )
    if len(names) != 1:
        fail(f"expected exactly one resolve snapshot under {out_dir}; got {names!r}")
    with open(os.path.join(out_dir, names[0])) as handle:
        return json.load(handle)


def implied_from_stdout(stdout):
    """The bindings the `implied:` lines report (ADR-0057 §D6)."""
    implied = {}
    for line in stdout.splitlines():
        if line.startswith(IMPLIED_PREFIX):
            facet, _, value = line[len(IMPLIED_PREFIX) :].partition("=")
            implied[facet] = value
    return implied


def check_inference_agreement(cfx, manifest, work, label, selects, expected):
    """configflux-cvey: all THREE verbs must agree on every input where
    inference fires — the claim the cases above only ever make about two of them.

    `cfx explain` reaches the same verdict as `cfx resolve` because its
    resolve-context check runs through `session_compose::resolve`, where ADR-0057
    §D6's inference lives. `cfx options` never calls that fixpoint at all: it asks
    the solver session for each facet's still-valid set. So the three verbs agree
    here only because two independent computations over one model land on the same
    answer, and nothing but a test that runs all three can say they still do.

    The claim is asserted as an EQUIVALENCE over the declared closed facets the
    user did not name: `cfx options` offers exactly one value for such a facet if
    and only if `cfx resolve` implies that same value for it. Both directions are
    defects a user would meet. Left to right is ADR-0054 §2's shape — the listing
    says "only c2" while resolve binds something else or refuses. Right to left is
    the guided walk going stale: resolve has already decided a facet the walk
    still presents as a live question.

    The roster comes from the snapshot's `closed_facet_domains`, which is the
    SAME roster `infer_forced_bindings` iterates. Scoping the claim any other way
    would make it wrong rather than merely weaker: the S1 pack's facets are
    inferred from usage rather than declared, so they are absent from that roster
    and inference is correct not to fire on them, while `cfx options` still
    narrows them and would look like a disagreement.

    Returns the completed `explain` process so a caller can add case-specific
    assertions without driving the verb a second time."""
    print(f"[cfx-consistency] {label}: options, explain and resolve must AGREE where inference fires")

    out_dir = os.path.join(work, f"inference_{label.replace('/', '_')}_out")
    resolve = cfx_resolve(cfx, manifest, selects, out_dir)
    if resolve.returncode != EXIT_OK:
        fail(
            f"{label}: cfx resolve exited {resolve.returncode}, want {EXIT_OK}\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    implied = implied_from_stdout(resolve.stdout)
    # Without this the case would go vacuous the moment inference stopped firing
    # on the fixture: the three verbs would still agree, but about nothing.
    if implied != expected:
        fail(
            f"{label}: cfx resolve must imply exactly {expected!r} — the fixture no longer "
            f"decides what this case is about, so it proves nothing.\n"
            f"got {implied!r}\nstdout={resolve.stdout!r}"
        )

    # The human text and the wire envelope are two renderings of one decision;
    # a consumer that trusts the snapshot must get what the operator was shown.
    snapshot = resolve_snapshot(out_dir)
    if snapshot.get("implied_choices") != implied:
        fail(
            f"{label}: the resolve snapshot disagrees with the text cfx resolve printed; "
            f"snapshot implied_choices={snapshot.get('implied_choices')!r} vs {implied!r}"
        )
    roster = snapshot.get("closed_facet_domains") or {}
    if not roster:
        fail(f"{label}: the snapshot carries no closed_facet_domains, so nothing scopes the claim")
    named = set(snapshot.get("choices") or {}) | set(snapshot.get("context_tags") or {})

    listed = cfx_options(cfx, manifest, None, "json", selects=selects)
    if listed.returncode != EXIT_OK:
        fail(
            f"{label}: cfx options exited {listed.returncode}, want {EXIT_OK}\n"
            f"stdout={listed.stdout!r}\nstderr={listed.stderr!r}"
        )
    offered = {entry["facet"]: entry.get("valid_options", []) for entry in json.loads(listed.stdout)}
    missing = sorted(set(roster) - set(offered))
    if missing:
        fail(
            f"{label}: cfx options omits declared closed facet(s) {missing!r} that cfx resolve "
            f"reasons about; the two verbs are not even discussing the same model"
        )
    forced = {
        facet: offered[facet][0]
        for facet in roster
        if facet not in named and len(offered[facet]) == 1
    }
    if forced != implied:
        fail(
            f"{label}: cfx options and cfx resolve disagree about what the constraints decided.\n"
            f"  options leaves exactly one value standing for: {forced!r}\n"
            f"  cfx resolve implies:                          {implied!r}\n"
            "A facet in the first and not the second is a listing that has narrowed to a value "
            "resolve will not bind; one in the second and not the first is a guided walk still "
            "asking a question resolve has already answered."
        )
    print(f"[cfx-consistency] {label}: cfx options offers one value exactly where resolve infers one OK")

    explain = cfx_explain(cfx, manifest, selects, "text")
    out = explain.stdout.decode(errors="replace")
    err = explain.stderr.decode(errors="replace")
    if explain.returncode != EXIT_UNSAT:
        fail(
            f"{label}: cfx explain exited {explain.returncode}, want {EXIT_UNSAT} "
            f"(satisfiable -> nothing to explain) — it contradicts the selection cfx resolve "
            f"just accepted.\nstdout={out!r}\nstderr={err!r}"
        )
    if "nothing to explain" not in out:
        fail(f"{label}: cfx explain must report nothing to explain; got {out!r}")
    print(f"[cfx-consistency] {label}: all three verbs agree the forced selection is satisfiable OK")
    return explain


def check_implied_agreement(cfx, manifest, work):
    """configflux-secb.3: the two verbs must agree about a binding the
    CONSTRAINTS force, not just about one the user typed.

    `cfx resolve` binds `sorter_container` by inference and succeeds. `cfx
    explain` reaches that same verdict ONLY if its resolve-context check runs
    through `session_compose::resolve`, where the inference lives (ADR-0057
    §D6). A compiler-direct `resolve_from_selection` resolves with NO implied
    bindings, so the declared default binds `sorter_container=c1`, the
    `groups_equal` constraint is violated, and explain calls a selection
    unsatisfiable that resolve just accepted. That is the divergence pinned
    here — and it is reachable in both directions, so neither verb may be
    trusted to speak for the other.

    configflux-cvey brings `cfx options` in through the shared check, so the
    agreement this case asserts covers all three verbs rather than two. What
    stays here is the part specific to THIS fixture: the constraint explain
    blamed while it resolved without the implication."""
    explain = check_inference_agreement(
        cfx, manifest, work, "implied", IMPLIED_SELECTS, IMPLIED_EXPECTED
    )
    combined = explain.stdout.decode(errors="replace") + explain.stderr.decode(errors="replace")
    if IMPLIED_CONSTRAINT in combined:
        fail(
            f"implied: cfx explain contradicts cfx resolve — it blames "
            f"'{IMPLIED_CONSTRAINT}' for a selection resolve accepted by inferring "
            f"the binding.\n{combined!r}"
        )


# configflux-bmjt. The SAME `s_facet_equality` pack, with the deciding value
# delivered as an immutable CONTEXT TAG rather than a `--select` flag. A tag is
# part of the deployment environment — ADR-0057 §D6 ranks it directly below an
# explicit choice, it does not make it invisible — so all THREE verbs have to
# reason over it. See `check_tag_bound_agreement`.
TAG_ENVIRONMENT = {
    "scope": "all",
    "context_tags": {"line_container": "c2"},
    "choices": {},
}
TAG_FACET = "sorter_container"
TAG_EXCLUDED = "c1"
TAG_SURVIVING = "c2"
TAG_IMPLIED_LINE = "implied: sorter_container=c2"


# configflux-secb.5. Each case is a selection the model's LOWERED conjuncts
# forbid, paired with the attribution id both verbs must blame it on.
LOWERED_CASES = [
    (
        "derive",
        [("site", "factory_a"), ("line_container", "c2")],
        "derive:line_container:site=factory_a",
    ),
    (
        "accepts",
        [("line_container", "c3")],
        "accepts:compute_service.container",
    ),
]
# A selection the same model accepts, so the case cannot pass by rejecting
# everything.
LOWERED_SAT_SELECTS = [("site", "factory_a")]


def check_lowered_agreement(cfx, manifest, work):
    """configflux-secb.5: the two verbs must agree about a rule NOBODY WROTE.

    A `derive` table and an `accepts` list are authoring constructs that lower
    to root conjuncts (ADR-0057 §D4). `cfx resolve` evaluates them off the
    resolve model; `cfx explain` names them out of the `.ccm` roster the emitter
    wrote. Those are two independent readings of one lowering, so this is the
    case that catches a lowering applied to one of them and not the other —
    which would show up to a user as resolve refusing a selection explain says
    is fine."""
    for label, selects, attribution in LOWERED_CASES:
        print(f"[cfx-consistency] lowered/{label}: both verbs must blame {attribution}")

        out_dir = os.path.join(work, f"lowered_{label}_out")
        resolve = cfx_resolve(cfx, manifest, selects, out_dir)
        if resolve.returncode != EXIT_UNSAT:
            fail(
                f"lowered/{label}: cfx resolve exited {resolve.returncode}, want "
                f"{EXIT_UNSAT}\nstdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
            )
        if attribution not in resolve.stderr:
            fail(
                f"lowered/{label}: cfx resolve must name '{attribution}'\n"
                f"stderr={resolve.stderr!r}"
            )
        if os.path.exists(out_dir):
            fail(f"lowered/{label}: no snapshot may be written on a rejection: {out_dir}")

        explain = cfx_explain(cfx, manifest, selects, "text")
        out = explain.stdout.decode(errors="replace")
        err = explain.stderr.decode(errors="replace")
        if explain.returncode != EXIT_OK:
            fail(
                f"lowered/{label}: cfx explain exited {explain.returncode}, want "
                f"{EXIT_OK} (a rejection is the success path)\nstdout={out!r}\nstderr={err!r}"
            )
        if "cannot select" not in out:
            fail(
                f"lowered/{label}: cfx explain must explain the rejection cfx resolve "
                f"just refused, not call it satisfiable\nstdout={out!r}"
            )

        # The JSON envelope carries the machine identity of the same rule the
        # resolve diagnostic named, so a consumer can join the two.
        envelope = cfx_explain(cfx, manifest, selects, "json")
        blob = envelope.stdout.decode(errors="replace")
        if attribution not in blob:
            fail(
                f"lowered/{label}: the explain envelope must carry '{attribution}', the "
                f"same id cfx resolve rejected on\n{blob}"
            )

    print("[cfx-consistency] lowered: a legal selection is accepted by both verbs")
    resolve = cfx_resolve(
        cfx, manifest, LOWERED_SAT_SELECTS, os.path.join(work, "lowered_sat_out")
    )
    if resolve.returncode != EXIT_OK:
        fail(
            f"lowered/sat: cfx resolve exited {resolve.returncode}, want {EXIT_OK}\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    explain = cfx_explain(cfx, manifest, LOWERED_SAT_SELECTS, "text")
    if explain.returncode != EXIT_UNSAT:
        fail(
            f"lowered/sat: cfx explain exited {explain.returncode}, want {EXIT_UNSAT} "
            "(satisfiable -> nothing to explain)"
        )


def check_derived_binding_verdicts(cfx, manifest, work):
    """configflux-rzyd: an unsatisfiable selection is exit 3 even when the
    contradiction leaves a DERIVED binding with no value, and a merely
    underspecified one is still exit 2.

    The two cases are the discriminator and only mean anything read together.
    In both, the compiler reaches the same place — a component requires a
    binding nothing has bound — so a check that drove only one of them would
    pass just as well against a tool that answered 2 for everything, or 3.

    What tells them apart is a verdict only the solver holds. It refuses a
    choice on the first selection and accepts every choice on the second, and
    `cfx resolve` is the one verb that has to act on that difference: the other
    two already do. Before the fix it did not, and the message it printed
    blamed the missing default — a fact about the binding that is true in both
    cases and is the reason for neither."""
    print("[cfx-consistency] derived/unsat: an impossible selection is exit 3, not a usage error")

    out_dir = os.path.join(work, "rzyd_unsat_out")
    resolve = cfx_resolve(cfx, manifest, UNSAT_DERIVED_SELECTS, out_dir)
    if resolve.returncode != EXIT_UNSAT:
        fail(
            f"derived/unsat: cfx resolve exited {resolve.returncode}, want {EXIT_UNSAT} — "
            "a selection the solver has refused is unsatisfiable, not a bad invocation\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    if FALSE_CAUSE in resolve.stderr:
        fail(
            f"derived/unsat: the rejection still blames '{FALSE_CAUSE}', which is not why "
            f"this selection failed\nstderr={resolve.stderr!r}"
        )
    if os.path.exists(out_dir):
        fail(f"derived/unsat: no snapshot may be written on a rejection: {out_dir}")

    # The agreement this whole suite is about: explain must EXPLAIN the same
    # selection resolve just refused, not call it satisfiable.
    explain = cfx_explain(cfx, manifest, UNSAT_DERIVED_SELECTS, "text")
    out = explain.stdout.decode(errors="replace")
    if explain.returncode != EXIT_OK:
        fail(
            f"derived/unsat: cfx explain exited {explain.returncode}, want {EXIT_OK} "
            f"(an explanation is the success path)\nstdout={out!r}"
        )
    if "nothing to explain" in out:
        fail(f"derived/unsat: cfx explain must not call the refused selection satisfiable: {out!r}")

    print("[cfx-consistency] derived/underspecified: a merely unbound binding stays exit 2")

    out_dir = os.path.join(work, "rzyd_unbound_out")
    resolve = cfx_resolve(cfx, manifest, UNDERSPECIFIED_SELECTS, out_dir)
    if resolve.returncode != EXIT_USAGE:
        fail(
            f"derived/underspecified: cfx resolve exited {resolve.returncode}, want "
            f"{EXIT_USAGE} — this selection is satisfiable, it just has not named the "
            f"binding yet\nstdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    if UNBOUND_BINDING not in resolve.stderr:
        fail(
            f"derived/underspecified: the rejection must name the binding to bind\n"
            f"stderr={resolve.stderr!r}"
        )
    print("[cfx-consistency] derived: the two verdicts stay distinct OK")


def check_tag_bound_agreement(cfx, manifest, work):
    """configflux-bmjt: the three verbs must agree about a facet a CONTEXT TAG
    decides, not only about one a `--select` flag decides.

    `session_compose` builds a solver session per verb, and only the ADR-0057
    D6 inference fixpoint used to apply `context_tags`; the builder behind
    `options`, `select` and `explain` replayed choices alone. Under a tag the
    verbs therefore answered about different deployments: `options` offered the
    value the tag excludes, and `explain` reached the rejection through the
    resolve-context fallback instead of the solver's labeled core. That is why
    the assertions below read the JSON envelope rather than the text — the text
    can name the same rule either way, the envelope cannot.

    The `groups_equal` constraint ties `sorter_container` to `line_container`,
    so tagging `line_container=c2` decides `sorter_container` without anyone
    choosing it. That is the whole shape: one environment, one answer."""
    print("[cfx-consistency] tag: options, explain and resolve must AGREE under a context tag")

    sel_path = os.path.join(work, "tag_selection.json")
    write_selection_file(sel_path, TAG_ENVIRONMENT)

    listed = cfx_options(cfx, manifest, sel_path, "json")
    if listed.returncode != EXIT_OK:
        fail(
            f"tag: cfx options exited {listed.returncode}, want {EXIT_OK}\n"
            f"stdout={listed.stdout!r}\nstderr={listed.stderr!r}"
        )
    offered = {
        entry.get("facet"): entry.get("valid_options", []) for entry in json.loads(listed.stdout)
    }
    if offered.get(TAG_FACET) != [TAG_SURVIVING]:
        fail(
            f"tag: cfx options must offer only [{TAG_SURVIVING!r}] for {TAG_FACET} — the tag "
            f"has already excluded {TAG_EXCLUDED!r}; got {offered.get(TAG_FACET)!r}"
        )
    print("[cfx-consistency] tag: cfx options omits the value the tag excludes OK")

    explained = subprocess.run(
        [
            cfx,
            "explain",
            "--model",
            manifest,
            "--selection-file",
            sel_path,
            "--select",
            f"{TAG_FACET}={TAG_EXCLUDED}",
            "--format",
            "json",
        ],
        capture_output=True,
        text=True,
    )
    if explained.returncode != EXIT_OK:
        fail(
            f"tag: cfx explain exited {explained.returncode}, want {EXIT_OK} "
            f"(a rejection is the success path)\nstdout={explained.stdout!r}\n"
            f"stderr={explained.stderr!r}"
        )
    envelope = json.loads(explained.stdout)
    rejection = envelope.get("rejection")
    core = rejection.get("unsat_core") if isinstance(rejection, dict) else None
    if not core:
        fail(
            "tag: cfx explain must answer with the explain envelope and a labeled core — "
            "reaching the same verdict through the resolve-context fallback means the "
            f"solver session never saw the tag.\nstdout={explained.stdout!r}"
        )
    if rejection.get("blocking_choices") != TAG_ENVIRONMENT["context_tags"]:
        fail(
            "tag: cfx explain must attribute the rejection to the context tag that caused "
            f"it; blocking_choices={rejection.get('blocking_choices')!r}"
        )
    blamed = [clause.get("constraint_id") for clause in core.get("conflicting_constraints", [])]
    if IMPLIED_CONSTRAINT not in blamed:
        fail(
            f"tag: cfx explain must blame '{IMPLIED_CONSTRAINT}', the authored rule the tag "
            f"activates; blamed={blamed!r}"
        )
    print("[cfx-consistency] tag: cfx explain blames the tag and names the rule OK")

    resolve = subprocess.run(
        [
            cfx,
            "resolve",
            "--model",
            manifest,
            "--selection-file",
            sel_path,
            "--out",
            os.path.join(work, "tag_out"),
        ],
        capture_output=True,
        text=True,
    )
    if resolve.returncode != EXIT_OK:
        fail(
            f"tag: cfx resolve exited {resolve.returncode}, want {EXIT_OK}\n"
            f"stdout={resolve.stdout!r}\nstderr={resolve.stderr!r}"
        )
    if TAG_IMPLIED_LINE not in resolve.stdout:
        fail(
            f"tag: cfx resolve must report {TAG_IMPLIED_LINE!r} — the value the other two "
            f"verbs left standing.\nstdout={resolve.stdout!r}"
        )
    print("[cfx-consistency] tag: all three verbs agree on the tag-bound facet OK")


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
                "schema_version": 5,
                "model_hash": "",
                "scope": case["scope"],
                "context_tags": case["context_tags"],
                "choices": case["choices"],
                "selection_state_hash": "",
            },
            handle,
        )


def suggested_command(stderr):
    """The `run: ...` suffix of the unsat guidance line, VERBATIM.

    Deliberately not split here (configflux-zz2g). The line is a command the
    user is told to paste, so the only parse that means anything is the one a
    POSIX shell performs; splitting on whitespace would make a path with a
    space look fine to the test and broken to the user."""
    for line in stderr.splitlines():
        if not line.startswith("selection is unsatisfiable"):
            continue
        _, marker, command = line.partition("run: ")
        if marker:
            return command.strip()
    return None


def check_hint_reproduces_selection(cfx, manifests, work, case):
    """configflux-0qk2: resolve's "run: cfx explain ..." pointer must reproduce
    the selection it just refused.

    This is the resolve/explain agreement the checks above assert, at the level
    of the REMEDIATION PATH: resolve is entitled to refuse a selection, but the
    command it hands the user must not ask about a different one. Asserted on
    BEHAVIOR, not on flag spelling — the suggested command is parsed out of
    stderr and run verbatim, so any hint that reproduces the refused selection
    passes, whichever way it encodes it.

    configflux-zz2g sharpens "verbatim": the line is parsed the way a POSIX
    shell parses it and executed BY one, so a hint that only works because the
    test rebuilt its argv by hand no longer passes.

    configflux-egyj widens it from the paths to EVERY term the hint
    interpolates: a `--select` pair has to survive that parse as one argument
    too, or the pasted command reaches `cfx explain` as a malformed pair plus a
    positional the verb does not take."""
    label = case["label"]
    print(f"[cfx-consistency] hint {label}: suggested command must explain the same selection")

    # configflux-zz2g: a case may ask for a working directory whose name carries
    # shell metacharacters, so the paths the hint interpolates are ones a shell
    # would re-split.
    work_dir = os.path.join(work, case["subdir"]) if case.get("subdir") else work
    os.makedirs(work_dir, exist_ok=True)
    manifest = manifests[case["model"]]
    # Keyed on the CASE, not on the model: two cases may drive the same pack
    # (configflux-egyj added a second S1 one), and a name keyed on the model
    # would have them overwrite each other's selection file.
    slug = label.replace("/", "_").replace("-", "_")
    sel_path = os.path.join(work_dir, f"hint_{slug}_selection.json")
    write_selection_file(sel_path, case)
    cmd = [
        cfx,
        "resolve",
        "--model",
        manifest,
        "--selection-file",
        sel_path,
        "--out",
        os.path.join(work_dir, f"hint_{slug}_out"),
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

    printed = suggested_command(resolve.stderr)
    if not printed:
        fail(f"hint {label}: unsat stderr carried no `run: ...` guidance line: {resolve.stderr!r}")
    # Parsed the way a POSIX shell parses it. `shlex` is an INDEPENDENT
    # implementation of the quoting rule cfx applies, so this checks the
    # contract instead of echoing `main.rs::shell_quote` back at itself.
    try:
        argv = shlex.split(printed)
    except ValueError as exc:
        # An unbalanced quote. A line a shell cannot even parse is the worst
        # form of the same defect, and it must read as a verdict rather than as
        # a traceback from the harness (configflux-egyj).
        fail(
            f"hint {label}: a POSIX shell cannot parse the printed hint at all ({exc}) -- "
            f"an interpolated term left it unbalanced.\nprinted: {printed}"
        )
    if not argv or argv[0] != "cfx":
        fail(f"hint {label}: guidance line must suggest a cfx command; got {printed!r}")
    # configflux-zz2g: every path the hint names has to survive that parse as
    # ONE argument. Unquoted, a directory with a space in it becomes two argv
    # entries and the pasted command addresses files that do not exist -- and
    # the reason line above it has already refused the user's selection, so this
    # pointer is the only thing between them and the explanation.
    #
    # configflux-egyj: and so does every `--select` pair. The pair is ONE
    # argument by contract -- cfx splits it on the first `=` itself -- so a
    # space anywhere in the facet id or the option id hands `cfx explain` a
    # malformed pair followed by a positional it does not accept. Checked here
    # rather than only through the shell round trip below, because that run is
    # allowed to reach the same rejection by luck; this states which bytes the
    # line owes the reader.
    terms = [("model path", manifest), ("selection-file path", sel_path)]
    terms += [
        (f"--select pair '{facet}={option}'", f"{facet}={option}")
        for facet, option in case["selects"]
    ]
    for what, wanted in terms:
        if wanted not in argv:
            fail(
                f"hint {label}: the {what} did not survive a POSIX shell parse of the "
                f"printed hint -- want {wanted!r} as a single argument.\n"
                f"printed: {printed}\nparsed: {argv!r}"
            )

    # Run it AS PRINTED, through a real shell. Only the leading `cfx` token is
    # replaced -- that one names the user's PATH binary and the one under test
    # is not on PATH -- so every byte after it is the hint's own.
    shell_command = shlex.quote(cfx) + printed[len("cfx") :]
    suggested = subprocess.run(["/bin/sh", "-c", shell_command], capture_output=True, text=True)
    combined = suggested.stdout + suggested.stderr
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

    # One compile serves both facet-equality cases: they differ only in whether
    # the deciding value arrives as an explicit choice or as a context tag.
    equality_manifest = compile_scenario(
        compiler,
        env_path("IMPLIED_DEFS"),
        env_path("IMPLIED_COMPONENTS"),
        os.path.join(workroot, "implied_cmp"),
    )
    check_implied_agreement(cfx, equality_manifest, workroot)
    check_tag_bound_agreement(cfx, equality_manifest, workroot)

    # One compile serves the lowering cases and the derived-inference pair: they
    # are the same three chunks read for a rejection and for an implication.
    lowered_manifest = compile_sources(
        compiler,
        [
            env_path("LOWERED_CATALOGUE"),
            env_path("LOWERED_BINDINGS"),
            env_path("LOWERED_COMPONENTS"),
        ],
        os.path.join(workroot, "lowered_cmp"),
    )
    check_lowered_agreement(cfx, lowered_manifest, workroot)
    check_derived_binding_verdicts(cfx, lowered_manifest, workroot)
    check_inference_agreement(
        cfx, lowered_manifest, workroot, "derived", DERIVED_SELECTS, DERIVED_EXPECTED
    )
    check_inference_agreement(
        cfx,
        lowered_manifest,
        workroot,
        "derived/control",
        DERIVED_CONTROL_SELECTS,
        DERIVED_CONTROL_EXPECTED,
    )

    manifests = {
        "s1": manifest,
        "hero": compile_scenario(
            compiler,
            env_path("HERO_DEFS"),
            env_path("HERO_COMPONENTS"),
            os.path.join(workroot, "hero_cmp"),
        ),
        # configflux-zz2g: the same hero pack, compiled into a directory whose
        # name carries a space and a single quote, so `--model` in the printed
        # hint is a path a shell would re-split. Compiled rather than copied
        # from the entry above: the manifest is what cfx is handed, and copying
        # one would test the copy.
        "metachar": compile_scenario(
            compiler,
            env_path("HERO_DEFS"),
            env_path("HERO_COMPONENTS"),
            os.path.join(workroot, "hero cmp 'v1'"),
        ),
    }
    for case in HINT_CASES:
        check_hint_reproduces_selection(cfx, manifests, workroot, case)

    print("[cfx-consistency] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
