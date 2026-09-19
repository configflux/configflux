# SPDX-License-Identifier: BUSL-1.1
"""Hermetic fixture-schema guard for the model explorer (ADR-0043 §4).

Pure-JSON check (no browser, no network, no pipeline build): every committed
explorer fixture must be well-formed JSON and declare the *current* product
schema version, and the explorer's JS ``SUPPORTED_VERSIONS`` constant must agree
with it. When the product schema version is bumped, this test fails until the
fixtures are regenerated (see ``explorer/fixtures/README.md``).

The expected version is **read** from ``compiler/src/product_api.rs``, not
restated here. A hard-coded copy cannot enforce the coupling it claims: when
``PRODUCT_SCHEMA_VERSION`` moved 3 -> 4 this test passed unchanged over fixtures
that still declared 3.

``//compiler:src/product_api.rs`` is a ``data`` dep of this target for a reason
that is easy to lose. Runfiles entries here are symlinks into the source tree and
the sandbox mounts that tree read-only, so the bytes are readable either way —
what the dep edge buys is Bazel *cache invalidation*. Without it a bump leaves
this test cached ``PASSED`` and the guard never runs at all.

``SUPPORTED_VERSIONS`` in ``explorer/js/schema.js`` is still hand-maintained and
is asserted against the derived value below.

``s1-resolve-snapshot.json`` additionally embeds one *authentic* pipeline value,
``resolve_hash``, which is pinned here to the S1 smoke byte-stability baseline.
Its ``model_hash`` and ``selection_state_hash`` are documented stand-ins and are
deliberately left unguarded — see the coupling block below.

The same value is quoted in ``explorer/fixtures/README.md``, which explains its
provenance, and that quotation is pinned to the fixture here too: prose that
names a hash can rot as easily as a fixture can, and did.

This test is wired into ``//...``; the Playwright smoke suite
(``tools/explorer_smoke_test.py``) is deliberately NOT (ADR-0043 §4).
"""

import json
import os
import re
import unittest
from pathlib import Path

# The declaration only. product_api.rs also names the identifier in comparisons,
# `format!` args and struct literals, any of which an unanchored search would hit.
_PRODUCT_SCHEMA_VERSION_DECL = re.compile(
    r"^pub const PRODUCT_SCHEMA_VERSION:\s*u32\s*=\s*(\d+)\s*;", re.MULTILINE
)

# Each fixture, and where its schema_version(s) live.
#   "object" — a single result object: top-level schema_version.
#   "array"  — a JSON array of result objects: each element's schema_version.
FIXTURES = {
    "s1-model-summary.json": "object",
    "s1-facets.json": "array",
    "s1-resolve-snapshot.json": "object",
    "s1-explain-conflict.json": "object",
    "s-requires-resolve-snapshot.json": "object",
}

# Minimal structural expectations per fixture (a light shape check on top of the
# version guard, so a truncated/renamed artifact is caught too).
REQUIRED_KEYS = {
    "s1-model-summary.json": ("status", "source_digest", "query", "summary"),
    "s1-resolve-snapshot.json": ("status", "model_hash", "selection_state_hash", "resolve_hash", "resolved_output"),
    "s1-explain-conflict.json": ("status", "model_hash", "facet", "option", "rejection"),
    "s-requires-resolve-snapshot.json": ("status", "model_hash", "selection_state_hash", "resolve_hash", "resolved_output"),
}

# --- requirement delivery (ADR-0057 D7, configflux-secb.6) -------------------
#
# The Resolution view renders a requirements table, and a table with nothing to
# render is no evidence that it works. ``s-requires-resolve-snapshot.json`` is
# real ``cfx resolve`` output over ``compiler/scenarios/s_requires_delivery``, so
# the block below is the pipeline's rather than an illustration: every field
# traces to the catalogue entry the binding resolved to.
REQUIRES_FIXTURE = "s-requires-resolve-snapshot.json"
REQUIRES_EXPECTED_BINDING = ("container", "line_container", "c1")
REQUIRES_EXPECTED_COMPONENTS = ("compute_service", "vision_service")
REQUIRES_EXPECTED_FIELDS = {"height_mm": 1000, "length_mm": 1200, "width_mm": 800}

# --- byte-stability coupling (configflux-6zvh) -------------------------------
#
# s1-resolve-snapshot.json embeds exactly ONE authentic pipeline value:
# resolve_hash, reused from the S1 smoke model (explorer/fixtures/README.md,
# "Data provenance"). The byte-stability baseline is where that value is
# recorded AND re-derived — compiler/src/scenario_byte_stability_tests.rs runs
# the real compile -> open -> resolve loop against it — which is what makes it
# the source of truth rather than a second copy.
#
# DELIBERATELY EXCLUDED: model_hash and selection_state_hash. README.md
# documents both as deterministic, schema-valid, S1-scoped STAND-INS (sha256 of
# a stable label), not pipeline output; at HEAD each already differs from its
# s1-smoke baseline counterpart. Pinning either to a source of truth would
# produce false failures on every run, so the guard covers resolve_hash alone.
SNAPSHOT_FIXTURE = "s1-resolve-snapshot.json"
BASELINE_SCENARIO = "s1-smoke"
BASELINE_FIXTURE = "compiler/tests/fixtures/byte-stability-baselines.json"
GUARDED_HASHES = frozenset({"resolve_hash"})
# resolved_output_hash (ADR-0059 D3) is classified as UNGUARDED deliberately,
# and this is the decision test_hash_lineage_fields_are_classified exists to
# force. It cannot be guarded against the baseline: the baseline records a
# `component:thermal_control`-scoped resolve, while this fixture's payload is
# `all`-scoped, so the two hashes are of different payloads by construction and
# pinning one to the other would be a false failure on every run — the same
# reason model_hash and selection_state_hash sit here. Unlike those two it is
# not a label digest: it is the real sha256 of THIS fixture's own
# {schema_version, scope, resolved_output}, which is self-consistent with the
# bytes the explorer renders. Adding a resolved_output_hash column to
# byte-stability-baselines.json would make it guardable and is the way to
# promote it into GUARDED_HASHES later.
STAND_IN_HASHES = frozenset(
    {"model_hash", "selection_state_hash", "resolved_output_hash"}
)

# --- README quotation coupling (configflux-p6sw) -----------------------------
#
# fixtures/README.md quotes the snapshot's resolve_hash in abbreviated form
# while explaining why that one value is authentic. Nothing tied the quotation
# to the fixture, so a rotation that regenerated the snapshot left the prose
# citing a hash the file no longer carries — which is how the README came to
# name a value from two rotations earlier. The guard below closes that gap: the
# prefix the prose quotes must still be a prefix of the fixture's hash.
#
# fixtures/README.md is a ``data`` dep of this target for the same
# cache-invalidation reason as product_api.rs and the baselines: without the
# edge, editing the prose alone leaves this test cached PASSED.
SNAPSHOT_README = "README.md"
# ``Its `resolve_hash` (`9b3e63b9…`) is the authentic S1 smoke value`` — the
# quotation is deliberately matched in place rather than searched for loosely,
# so rewording the sentence fails the parse instead of silently unhooking it.
_README_QUOTED_HASH = re.compile(
    r"`resolve_hash`\s*\(\s*`([0-9a-f]{8,64})(?:\u2026|\.\.\.)?`\s*\)"
)

# ADR-0054 gives every declared constraint an identity the solver carries into
# the labeled core, so a model_rule with no constraint_id was never emitted.
EXPLAIN_FIXTURE = "s1-explain-conflict.json"

# --- explain wording parity (configflux-tv5h) --------------------------------
#
# explain_view.js documents itself as byte-faithful to the shared text renderer
# `render_unsat_core` (interpreter/src/explain_renderer.rs) — the one prose
# renderer `cfx explain` invokes. ADR-0054 §5.4 gave a model_rule an optional
# `constraint_id` and the CLI names it ("blocked by constraint <id>: ..."), but
# renderConstraint carried no branch for it, so the explorer rendered every
# authored constraint as an anonymous model rule and the "same wording as the
# CLI" claim quietly stopped holding.
#
# The wording is READ out of the JS below rather than restated, for the same
# reason PRODUCT_SCHEMA_VERSION is read out of product_api.rs: a restated copy
# cannot enforce the parity it claims. Executing the JS is not an option here —
# there is no JS toolchain in the build graph, and a system `node` would be
# neither hermetic nor cheap enough for the per-change gate — so the guard
# extracts renderConstraint's return templates and applies them to the
# fixture's own constraint object. Deleting the branch (no template
# interpolates constraint_id) and rewording a line both turn this red.
EXPLAIN_VIEW_JS = "explain_view.js"

_JS_RENDER_CONSTRAINT_BODY = re.compile(
    r"^function renderConstraint\(constraint\) \{\n(.*?)^\}", re.DOTALL | re.MULTILINE
)
_JS_RETURN_TEMPLATE = re.compile(r"return `([^`]*)`;")
_JS_CONSTRAINT_FIELD = re.compile(r"\$\{constraint\.(\w+)\}")


def _js_return_templates(js_source: str) -> list[str]:
    """Return renderConstraint's template literals, in source order.

    A renamed or restructured function must raise rather than yield an empty
    list: a guard that quietly stops matching still reports green.
    """
    body = _JS_RENDER_CONSTRAINT_BODY.search(js_source)
    if body is None:
        raise AssertionError(
            "could not find `function renderConstraint(constraint)` in "
            f"explorer/js/{EXPLAIN_VIEW_JS}; the wording guard is anchored to it"
        )
    templates = _JS_RETURN_TEMPLATE.findall(body.group(1))
    if not templates:
        raise AssertionError("renderConstraint returns no template literal")
    return templates


def _render_js_template(template: str, constraint: dict) -> str:
    """Apply one ``${constraint.field}`` template to a constraint entry.

    An interpolation the entry cannot satisfy raises: the JS would render
    ``undefined`` there, which is a divergence worth failing on, not papering
    over with a default.
    """

    def field(match: re.Match) -> str:
        name = match.group(1)
        if name not in constraint:
            raise AssertionError(
                f"renderConstraint interpolates ${{constraint.{name}}}, which the "
                f"{EXPLAIN_FIXTURE} entry does not carry"
            )
        return str(constraint[name])

    return _JS_CONSTRAINT_FIELD.sub(field, template)


def _explorer_dir() -> Path:
    """Resolve the explorer/ dir under both `bazel test` (runfiles) and a direct
    `python3` run from the workspace."""
    here = Path(__file__).resolve().parent
    if (here / "fixtures").is_dir():
        return here
    ws = os.environ.get("BUILD_WORKSPACE_DIRECTORY")
    if ws and (Path(ws) / "explorer" / "fixtures").is_dir():
        return Path(ws) / "explorer"
    raise AssertionError(f"cannot locate explorer/ fixtures from {here}")


def _product_api_path() -> Path:
    """Resolve compiler/src/product_api.rs under both `bazel test` (runfiles) and
    a direct `python3` run from the workspace.

    `absolute()` is tried before `resolve()`: runfiles entries are symlinks into
    the source tree, and resolving one first would read bytes Bazel never
    declared as an input to this target.
    """
    here = Path(__file__)
    roots = [here.absolute().parent.parent, here.resolve().parent.parent]
    ws = os.environ.get("BUILD_WORKSPACE_DIRECTORY")
    if ws:
        roots.append(Path(ws))
    for root in roots:
        candidate = root / "compiler" / "src" / "product_api.rs"
        if candidate.is_file():
            return candidate
    raise AssertionError(
        "cannot locate compiler/src/product_api.rs; searched "
        + ", ".join(str(r) for r in roots)
    )


def _baselines_path() -> Path:
    """Resolve ``compiler/tests/fixtures/byte-stability-baselines.json`` under
    both ``bazel test`` (runfiles) and a direct ``python3`` run.

    Same root search as ``_product_api_path()``, and ``absolute()`` precedes
    ``resolve()`` for the same reason: runfiles entries are symlinks into the
    source tree, and resolving one first would read bytes Bazel never declared
    as an input to this target.
    """
    here = Path(__file__)
    roots = [here.absolute().parent.parent, here.resolve().parent.parent]
    ws = os.environ.get("BUILD_WORKSPACE_DIRECTORY")
    if ws:
        roots.append(Path(ws))
    for root in roots:
        candidate = root / BASELINE_FIXTURE
        if candidate.is_file():
            return candidate
    raise AssertionError(
        f"cannot locate {BASELINE_FIXTURE}; searched "
        + ", ".join(str(r) for r in roots)
    )


def _baseline_hash(source: str, scenario: str, field: str) -> str:
    """Return one recorded hash from byte-stability-baselines.json source text.

    A missing scenario or field raises rather than returning a default. A guard
    that quietly stops matching still reports green, which is worse than no
    guard at all — the same failure mode a renamed ``PRODUCT_SCHEMA_VERSION``
    would cause above.
    """
    try:
        data = json.loads(source)
    except json.JSONDecodeError as exc:
        raise AssertionError(f"{BASELINE_FIXTURE} is not valid JSON: {exc}") from exc
    scenarios = data.get("scenarios")
    if not isinstance(scenarios, dict) or scenario not in scenarios:
        raise AssertionError(
            f"{BASELINE_FIXTURE} has no scenarios[{scenario!r}]; the explorer "
            "fixture hash guard is anchored to it"
        )
    value = scenarios[scenario].get(field)
    if not isinstance(value, str) or not value:
        raise AssertionError(
            f"{BASELINE_FIXTURE} scenarios[{scenario!r}] has no {field!r}"
        )
    return value


def _readme_quoted_resolve_hash(source: str) -> str:
    """Return the resolve_hash prefix quoted in ``fixtures/README.md``.

    Exactly one quotation is expected. Zero means the sentence was reworded and
    the coupling is no longer anchored; more than one means the prose grew a
    second claim this guard does not cover. Both raise, for the reason
    ``_baseline_hash`` raises: a guard that quietly stops matching still reports
    green, which is worse than no guard at all.
    """
    found = _README_QUOTED_HASH.findall(source)
    if not found:
        raise AssertionError(
            f"fixtures/{SNAPSHOT_README} no longer quotes the snapshot's "
            "resolve_hash in the form `resolve_hash` (`<prefix>\u2026`); the "
            "prose-to-fixture guard is unanchored until the sentence or this "
            "pattern is restored"
        )
    if len(found) > 1:
        raise AssertionError(
            f"fixtures/{SNAPSHOT_README} quotes {len(found)} resolve_hash "
            "values; this guard checks one, so the extra quotation is "
            "unguarded prose"
        )
    return found[0]


def _parse_product_schema_version(source: str) -> int:
    """Return PRODUCT_SCHEMA_VERSION as declared in product_api.rs source text.

    Exactly one declaration must be present. Zero (a rename) or several must
    raise rather than fall back to a default — a guard that quietly stops
    matching is worse than no guard, because it still reports green.
    """
    found = _PRODUCT_SCHEMA_VERSION_DECL.findall(source)
    if len(found) != 1:
        raise AssertionError(
            "expected exactly one `pub const PRODUCT_SCHEMA_VERSION: u32 = N;` "
            f"in compiler/src/product_api.rs, found {len(found)}"
        )
    return int(found[0])


# Read from the product, never restated here: a PRODUCT_SCHEMA_VERSION bump must
# turn this test red with no edit to this file.
EXPECTED_PRODUCT_SCHEMA_VERSION = _parse_product_schema_version(
    _product_api_path().read_text(encoding="utf-8")
)


class FixtureSchemaTest(unittest.TestCase):
    def setUp(self) -> None:
        self.root = _explorer_dir()
        self.fixtures = self.root / "fixtures"

    def test_all_expected_fixtures_present(self) -> None:
        present = {p.name for p in self.fixtures.glob("*.json")}
        self.assertEqual(
            present,
            set(FIXTURES),
            "explorer/fixtures/*.json drifted from the guarded set; update FIXTURES "
            "and the schema test when adding or removing a fixture",
        )

    def test_fixtures_are_valid_json_and_current_schema(self) -> None:
        for name, shape in FIXTURES.items():
            path = self.fixtures / name
            with self.subTest(fixture=name):
                self.assertTrue(path.is_file(), f"missing fixture {name}")
                try:
                    data = json.loads(path.read_text(encoding="utf-8"))
                except json.JSONDecodeError as exc:
                    self.fail(f"{name} is not valid JSON: {exc}")

                versions = self._schema_versions(data, shape)
                self.assertTrue(versions, f"{name} declares no schema_version")
                for v in versions:
                    self.assertEqual(
                        v,
                        EXPECTED_PRODUCT_SCHEMA_VERSION,
                        f"{name} declares schema_version {v}, expected "
                        f"{EXPECTED_PRODUCT_SCHEMA_VERSION}; regenerate the fixtures "
                        f"(explorer/fixtures/README.md)",
                    )

    def test_required_keys_present(self) -> None:
        for name, keys in REQUIRED_KEYS.items():
            path = self.fixtures / name
            with self.subTest(fixture=name):
                data = json.loads(path.read_text(encoding="utf-8"))
                for key in keys:
                    self.assertIn(key, data, f"{name} missing required key '{key}'")

    def test_resolve_hash_matches_byte_stability_baseline(self) -> None:
        """The snapshot's one authentic hash must track the pipeline.

        Without this, a rotation that updates the baseline and the scenario
        golden but forgets the explorer fixture leaves the explorer
        demonstrating a resolve_hash the pipeline no longer produces, and the
        gate stays green.
        """
        snapshot = json.loads((self.fixtures / SNAPSHOT_FIXTURE).read_text(encoding="utf-8"))
        baselines = _baselines_path().read_text(encoding="utf-8")
        for field in sorted(GUARDED_HASHES):
            with self.subTest(hash=field):
                self.assertEqual(
                    snapshot.get(field),
                    _baseline_hash(baselines, BASELINE_SCENARIO, field),
                    f"{SNAPSHOT_FIXTURE} {field} disagrees with {BASELINE_FIXTURE} "
                    f"scenarios.{BASELINE_SCENARIO}.{field}; regenerate the fixture "
                    f"(explorer/fixtures/README.md)",
                )

    def test_readme_quotes_the_hash_the_fixture_carries(self) -> None:
        """The prose must quote the hash the fixture actually holds.

        The README explains the snapshot's provenance by naming its
        resolve_hash, and that quotation was free to rot: it named a value from
        an earlier rotation while every automated check stayed green. Pinning
        the quoted prefix to the fixture makes the next rotation fail here.
        """
        snapshot = json.loads((self.fixtures / SNAPSHOT_FIXTURE).read_text(encoding="utf-8"))
        readme = (self.fixtures / SNAPSHOT_README).read_text(encoding="utf-8")
        quoted = _readme_quoted_resolve_hash(readme)
        actual = snapshot.get("resolve_hash")
        self.assertTrue(
            isinstance(actual, str) and actual.startswith(quoted),
            f"fixtures/{SNAPSHOT_README} quotes resolve_hash {quoted!r}, but "
            f"{SNAPSHOT_FIXTURE} carries {actual!r}; correct the prose when the "
            "fixture is regenerated (the abbreviation must stay a prefix)",
        )

    def test_hash_lineage_fields_are_classified(self) -> None:
        """Every top-level ``*_hash`` field is guarded or a known stand-in.

        Only top-level fields are the result envelope's hash lineage; a nested
        ``resolved_artifacts.*.hash`` is an artifact content address and is not
        classified here. A new lineage field must join one of the two sets — so
        someone decides whether it is authentic — rather than silently landing
        in the fixture unguarded, which is the gap this test exists to close.
        """
        snapshot = json.loads((self.fixtures / SNAPSHOT_FIXTURE).read_text(encoding="utf-8"))
        present = {key for key in snapshot if key.endswith("_hash")}
        self.assertEqual(
            present,
            set(GUARDED_HASHES | STAND_IN_HASHES),
            f"{SNAPSHOT_FIXTURE} hash lineage drifted from the classified set; "
            "decide whether the new field is authentic (guard it against "
            f"{BASELINE_FIXTURE}) or a stand-in (add it to STAND_IN_HASHES)",
        )

    def test_requires_fixture_carries_a_delivered_catalogue_entry(self) -> None:
        """The Resolution view's requirements table must have real data to show.

        Without this, the fixture set would demonstrate schema 5 while showing
        nothing the schema bump was for, and a regression that stopped emitting
        the block would leave every explorer check green.
        """
        snapshot = json.loads((self.fixtures / REQUIRES_FIXTURE).read_text(encoding="utf-8"))
        scopes = snapshot["resolved_output"]
        self.assertTrue(scopes, f"{REQUIRES_FIXTURE} carries no resolved scope")
        slot, binding, entry = REQUIRES_EXPECTED_BINDING
        for scope_root, scope in scopes.items():
            components = scope["components"]
            for component_id in REQUIRES_EXPECTED_COMPONENTS:
                with self.subTest(scope=scope_root, component=component_id):
                    requires = components[component_id].get("requires")
                    self.assertIsNotNone(
                        requires,
                        f"{component_id} carries no requires block; regenerate the "
                        "fixture (explorer/fixtures/README.md)",
                    )
                    delivered = requires[slot]
                    self.assertEqual(delivered["binding"], binding)
                    self.assertEqual(delivered["entry"], entry)
                    self.assertEqual(delivered["fields"], REQUIRES_EXPECTED_FIELDS)

    def test_requires_view_renders_every_field_of_a_delivered_entry(self) -> None:
        """resolution_view.js must read the block the fixture carries.

        Read out of the JS rather than restated, for the same reason the explain
        wording is: a renamed key in the view would otherwise leave the fixture
        guard green while the table rendered empty cells.
        """
        js = (self.root / "js" / "resolution_view.js").read_text(encoding="utf-8")
        for key in ("requires", "binding", "entry", "fields"):
            self.assertIn(
                key,
                js,
                f"resolution_view.js no longer reads '{key}'; the requirements "
                "table cannot render a delivered catalogue entry without it",
            )

    def test_explain_core_names_the_constraint_it_came_from(self) -> None:
        """The checks above pass on any well-formed object; this one is what stops
        an invented explanation from shipping."""
        explain = json.loads((self.fixtures / EXPLAIN_FIXTURE).read_text(encoding="utf-8"))
        core = explain.get("rejection", {}).get("unsat_core") or {}
        rules = [c for c in core.get("conflicting_constraints", []) if c.get("kind") == "model_rule"]
        self.assertTrue(rules, f"{EXPLAIN_FIXTURE} names no model_rule constraint")
        for rule in rules:
            self.assertTrue(rule.get("constraint_id"), f"{EXPLAIN_FIXTURE} model_rule has no "
                            "constraint_id; regenerate it (explorer/fixtures/README.md)")

    def _attributed_model_rule(self) -> dict:
        """The fixture's one model_rule that carries a constraint_id."""
        explain = json.loads((self.fixtures / EXPLAIN_FIXTURE).read_text(encoding="utf-8"))
        core = explain.get("rejection", {}).get("unsat_core") or {}
        attributed = [
            c
            for c in core.get("conflicting_constraints", [])
            if c.get("kind") == "model_rule" and c.get("constraint_id")
        ]
        self.assertEqual(
            len(attributed),
            1,
            f"{EXPLAIN_FIXTURE} must carry exactly one attributed model_rule for "
            "the wording guard to render",
        )
        return attributed[0]

    def _explain_view_js(self) -> str:
        return (self.root / "js" / EXPLAIN_VIEW_JS).read_text(encoding="utf-8")

    def test_explain_view_names_the_constraint_the_cli_names(self) -> None:
        """An attributed clause must render exactly as `cfx explain` renders it.

        interpreter/src/explain_renderer.rs::render_constraint emits
        ``  blocked by constraint {id}: {summary}`` for a model_rule carrying a
        constraint_id (ADR-0054 §5.4). renderConstraint had no such branch, so
        the explorer rendered the fixture's authored constraint as an anonymous
        model rule — the same bytes for every declared policy, naming none of
        them.
        """
        rule = self._attributed_model_rule()
        attributed = [t for t in _js_return_templates(self._explain_view_js()) if "constraint_id" in t]
        self.assertEqual(
            len(attributed),
            1,
            "expected exactly one renderConstraint branch interpolating "
            "${constraint.constraint_id}; without it the explorer cannot name "
            "an authored constraint the way `cfx explain` does",
        )
        self.assertEqual(
            _render_js_template(attributed[0], rule),
            f"  blocked by constraint {rule['constraint_id']}: {rule['summary']}",
            "explorer wording drifted from interpreter/src/explain_renderer.rs "
            "(two-space indent, no outer quotes around the id or the condition)",
        )

    def test_explain_view_keeps_model_rule_wording_without_an_id(self) -> None:
        """A clause with no constraint_id keeps the generic model-rule wording.

        ADR-0054 §5.4 forbids naming a synthesized cardinality conjunct as if it
        were authored policy, so the CLI's None arm must survive the branch
        added above rather than being replaced by it.
        """
        rule = dict(self._attributed_model_rule())
        rule.pop("constraint_id")
        generic = [
            t
            for t in _js_return_templates(self._explain_view_js())
            if "constraint_id" not in t and "constraint.summary" in t
        ]
        self.assertEqual(
            len(generic),
            1,
            "expected exactly one renderConstraint branch rendering a summary "
            "without a constraint_id (the CLI's None arm)",
        )
        self.assertEqual(
            _render_js_template(generic[0], rule),
            f"  blocked by model rule: {rule['summary']}",
            "the unattributed model-rule wording drifted from "
            "interpreter/src/explain_renderer.rs",
        )

    def test_js_supported_versions_matches(self) -> None:
        """The UI guard and the fixtures must agree on the supported version."""
        schema_js = (self.root / "js" / "schema.js").read_text(encoding="utf-8")
        match = re.search(r"SUPPORTED_VERSIONS\s*=\s*Object\.freeze\(\[([^\]]*)\]\)", schema_js)
        self.assertIsNotNone(match, "could not find SUPPORTED_VERSIONS in explorer/js/schema.js")
        listed = [int(tok) for tok in re.findall(r"\d+", match.group(1))]
        self.assertIn(
            EXPECTED_PRODUCT_SCHEMA_VERSION,
            listed,
            "explorer/js/schema.js SUPPORTED_VERSIONS must include "
            f"{EXPECTED_PRODUCT_SCHEMA_VERSION}",
        )

    def _schema_versions(self, data, shape):
        if shape == "array":
            self.assertIsInstance(data, list, "expected a JSON array")
            self.assertTrue(data, "array fixture is empty")
            return [e.get("schema_version") for e in data]
        self.assertIsInstance(data, dict, "expected a JSON object")
        return [data.get("schema_version")]


class ProductSchemaVersionParseTest(unittest.TestCase):
    """The parse that couples this guard to ``compiler/src/product_api.rs``."""

    def test_parses_the_declaration(self) -> None:
        self.assertEqual(
            _parse_product_schema_version("pub const PRODUCT_SCHEMA_VERSION: u32 = 7;\n"),
            7,
        )

    def test_ignores_non_declaration_uses(self) -> None:
        """``product_api.rs`` names the identifier ~20 times — comparisons,
        ``format!`` args, struct literals. Only the ``pub const`` may match."""
        source = (
            "// `PRODUCT_SCHEMA_VERSION` guards the REQUEST.\n"
            "pub const PRODUCT_SCHEMA_VERSION: u32 = 7;\n"
            "    if request.schema_version != PRODUCT_SCHEMA_VERSION {\n"
            '        hint: Some(format!("Set schema_version to {}", PRODUCT_SCHEMA_VERSION)),\n'
            "            schema_version: PRODUCT_SCHEMA_VERSION,\n"
        )
        self.assertEqual(_parse_product_schema_version(source), 7)

    def test_rejects_source_without_declaration(self) -> None:
        """A rename must fail this guard loudly, never disable it silently."""
        with self.assertRaises(AssertionError):
            _parse_product_schema_version("    schema_version: PRODUCT_SCHEMA_VERSION,\n")

    def test_rejects_duplicate_declarations(self) -> None:
        with self.assertRaises(AssertionError):
            _parse_product_schema_version(
                "pub const PRODUCT_SCHEMA_VERSION: u32 = 7;\n"
                "pub const PRODUCT_SCHEMA_VERSION: u32 = 8;\n"
            )

    def test_locates_product_api_at_runtime(self) -> None:
        """The ``//compiler:src/product_api.rs`` data dep must be reachable when
        the test runs; an unreachable source would leave the guard unanchored."""
        path = _product_api_path()
        self.assertTrue(path.is_file(), f"product_api.rs not reachable at {path}")
        self.assertGreater(EXPECTED_PRODUCT_SCHEMA_VERSION, 0)


class BaselineHashParseTest(unittest.TestCase):
    """The parse that couples this guard to the byte-stability baseline."""

    _SOURCE = json.dumps(
        {"scenarios": {"s1-smoke": {"resolve_hash": "abc123", "bom_hash": ""}}}
    )

    def test_reads_the_recorded_hash(self) -> None:
        self.assertEqual(_baseline_hash(self._SOURCE, "s1-smoke", "resolve_hash"), "abc123")

    def test_rejects_missing_scenario(self) -> None:
        """A renamed scenario must fail this guard loudly, never disable it."""
        with self.assertRaises(AssertionError):
            _baseline_hash(self._SOURCE, "s1-renamed", "resolve_hash")

    def test_rejects_missing_field(self) -> None:
        with self.assertRaises(AssertionError):
            _baseline_hash(self._SOURCE, "s1-smoke", "selection_state_hash")

    def test_rejects_empty_field(self) -> None:
        """An empty string would compare equal to nothing useful; it must raise."""
        with self.assertRaises(AssertionError):
            _baseline_hash(self._SOURCE, "s1-smoke", "bom_hash")

    def test_rejects_malformed_source(self) -> None:
        with self.assertRaises(AssertionError):
            _baseline_hash("{not json", "s1-smoke", "resolve_hash")

    def test_locates_baselines_at_runtime(self) -> None:
        """The ``//compiler:tests/fixtures/...`` data dep must be reachable when
        the test runs; an unreachable baseline leaves the coupling unanchored."""
        path = _baselines_path()
        self.assertTrue(path.is_file(), f"baselines not reachable at {path}")
        self.assertTrue(
            _baseline_hash(path.read_text(encoding="utf-8"), BASELINE_SCENARIO, "resolve_hash")
        )

    def test_stand_ins_are_not_guarded(self) -> None:
        """The unguarded fields must stay out of the guarded set.

        model_hash and selection_state_hash are documented in README.md as
        sha256 of a stable label, not pipeline output. resolved_output_hash IS
        pipeline-shaped but is unguardable here for a different reason: the
        baseline records a component:thermal_control resolve while this fixture
        is all-scoped, so the two hash different payloads by construction.
        Guarding any of the three against the baseline would fail on every run.

        Pinning the exact set is what makes adding a field a decision: a new
        lineage field cannot reach STAND_IN_HASHES without this assertion — and
        so a human — being updated.
        """
        self.assertFalse(GUARDED_HASHES & STAND_IN_HASHES)
        self.assertEqual(
            STAND_IN_HASHES,
            frozenset({"model_hash", "selection_state_hash", "resolved_output_hash"}),
        )


class ReadmeQuotedHashParseTest(unittest.TestCase):
    """The parse that couples the fixture to the prose describing it."""

    def test_reads_an_abbreviated_quotation(self) -> None:
        self.assertEqual(
            _readme_quoted_resolve_hash("Its `resolve_hash` (`9b3e63b9\u2026`) is the"),
            "9b3e63b9",
        )

    def test_reads_an_ascii_abbreviation(self) -> None:
        self.assertEqual(
            _readme_quoted_resolve_hash("Its `resolve_hash` (`9b3e63b9...`) is the"),
            "9b3e63b9",
        )

    def test_reads_a_full_hash(self) -> None:
        full = "9b3e63b9" * 8
        self.assertEqual(
            _readme_quoted_resolve_hash(f"Its `resolve_hash` (`{full}`) is the"), full
        )

    def test_rejects_a_reworded_sentence(self) -> None:
        """A rewrite that drops the quotation must fail loudly, never disable
        the guard."""
        with self.assertRaises(AssertionError):
            _readme_quoted_resolve_hash("Its resolve_hash is the authentic value.")

    def test_rejects_a_second_quotation(self) -> None:
        with self.assertRaises(AssertionError):
            _readme_quoted_resolve_hash(
                "`resolve_hash` (`9b3e63b9\u2026`) and `resolve_hash` (`0a6a3d6e\u2026`)"
            )

    def test_bites_on_a_stale_quotation(self) -> None:
        """The assertion the guard exists for: a prefix from an earlier
        rotation is not a prefix of the hash the fixture now carries."""
        snapshot = json.loads(
            (_explorer_dir() / "fixtures" / SNAPSHOT_FIXTURE).read_text(encoding="utf-8")
        )
        stale = _readme_quoted_resolve_hash("`resolve_hash` (`1f93f566\u2026`)")
        self.assertFalse(snapshot["resolve_hash"].startswith(stale))

if __name__ == "__main__":
    unittest.main()
