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
}

# Minimal structural expectations per fixture (a light shape check on top of the
# version guard, so a truncated/renamed artifact is caught too).
REQUIRED_KEYS = {
    "s1-model-summary.json": ("status", "source_digest", "query", "summary"),
    "s1-resolve-snapshot.json": ("status", "model_hash", "selection_state_hash", "resolve_hash", "resolved_output"),
    "s1-explain-conflict.json": ("status", "model_hash", "facet", "option", "rejection"),
}

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
STAND_IN_HASHES = frozenset({"model_hash", "selection_state_hash"})

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
        """The two stand-in fields must stay out of the guarded set.

        README.md documents them as sha256 of a stable label, not pipeline
        output. Guarding them against the baseline would fail on every run.
        """
        self.assertFalse(GUARDED_HASHES & STAND_IN_HASHES)
        self.assertEqual(STAND_IN_HASHES, frozenset({"model_hash", "selection_state_hash"}))


if __name__ == "__main__":
    unittest.main()
