// SPDX-License-Identifier: BUSL-1.1
//
// `cfx diff` end-to-end tests (configflux-dkmm.5, ADR-0059 D4/M5/M6).
//
// The pure change-list rules are pinned in `diff.rs`'s own test module against
// hand-built JSON. These are the other half: they drive the REAL CLI over REAL
// models compiled in-process, because what is under test here is a verdict
// about two compiled packages plus an exit code, and stubbing either side would
// test the stub. They share `tests.rs`'s harness (`run_args`, `compile_fixture*`,
// `tree`, `write_fixture_file`) rather than carrying a second copy of it.
//
// A separate module from `tests.rs` only because that file is already at its
// lint cap; the harness stays in one place.

use super::*;

use compiler::product_api::PRODUCT_SCHEMA_VERSION;

use crate::tests::{
    compile_fixture, compile_fixture_chunks, run_args, tree, write_fixture_file, ModelFixture,
    HERO_COMPONENTS, HERO_DEFS, HERO_MANIFEST,
};

const POLY_CATALOGUE: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/catalogue/00_catalogue.json");
const POLY_VISION: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/vision/10_vision.json");
const POLY_COMPUTE: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/compute/10_compute.json");
const POLY_SORTER: &str =
    include_str!("../../examples/06-catalogue-polyrepo/repos/sorter/20_sorter.json");
const POLY_MANIFEST: &str =
    include_str!("../../examples/06-catalogue-polyrepo/environments.json");

/// The three service scopes these tests sweep — every unit the example ships
/// (configflux-gmla). The sorter is not padding: it reads the SAME `containers`
/// catalogue as the other two, but through `sorter_container`, a free per-site
/// decision the manifest pins to `c3`. An edit to `c2` therefore MUST leave its
/// cells alone while the site that resolves to `c2` reports the change — the
/// negative half of "the report follows the selection, not the file that
/// changed", which two scopes sharing one derived binding cannot state.
const POLY_SCOPES: &str =
    "component:vision_service,component:compute_service,component:sorter_service";

/// Replace the single occurrence of `needle` with `replacement`, asserting it
/// really was single — a silently-missed edit would compile the UNCHANGED model
/// and turn a `changed` assertion into a confusing `unchanged`.
fn edit_once(source: &str, needle: &str, replacement: &str) -> String {
    assert_eq!(
        source.matches(needle).count(),
        1,
        "fixture edit anchor '{needle}' must occur exactly once"
    );
    source.replace(needle, replacement)
}

/// `cfx diff` over one manifest, both models, every environment.
fn diff_args<'a>(base: &'a str, head: &'a str, manifest: &'a str) -> Vec<&'a str> {
    vec![
        "cfx", "diff", "--base", base, "--head", head, "--manifest", manifest,
    ]
}

/// The `<status>  <environment>  <scope>` header lines, in emission order.
fn status_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| !line.starts_with(' ') && !line.starts_with("summary: "))
        .collect()
}

/// The indented lines belonging to the cell whose header is `header`.
fn cell_body<'a>(stdout: &'a str, header: &str) -> Vec<&'a str> {
    stdout
        .lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim_start())
        .collect()
}

fn summary_line(stdout: &str) -> &str {
    stdout
        .lines()
        .find(|line| line.starts_with("summary: "))
        .expect("every report ends with a summary line")
}

// ---------------------------------------------------------------------------
// The hero model: one service, three environments
// ---------------------------------------------------------------------------

#[test]
fn diff_same_model_is_all_unchanged_exit_0() {
    // T2 — the trivial oracle (ADR-0059 D4). Comparing a model with ITSELF must
    // report every cell unchanged and exit 0. Anything else means the two sides
    // were asked different questions, which is the one failure a diff must
    // never have.
    let fixture = compile_fixture("diff-identity", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", HERO_MANIFEST);

    // A5: `cfx diff` writes NOTHING. Snapshot the fixture tree around the run —
    // stronger than checking one directory, because it would catch a write
    // anywhere the model or the manifest lives.
    let before = tree(&fixture.dir);
    let (code, stdout, err) = run_args(&diff_args(&fixture.manifest, &fixture.manifest, &manifest));
    assert_eq!(code, EXIT_OK, "a model against itself must exit 0: {err}");
    assert_eq!(
        tree(&fixture.dir),
        before,
        "cfx diff must create no file; the fixture tree changed"
    );

    assert_eq!(
        status_lines(&stdout),
        vec![
            "unchanged  dev  component:webapp",
            "unchanged  prod  component:webapp",
            "unchanged  staging  component:webapp",
        ],
        "stdout:\n{stdout}"
    );
    assert_eq!(
        summary_line(&stdout),
        "summary: unchanged=3 changed=0 now_unsatisfiable=0 now_satisfiable=0 \
         unsatisfiable_both=0"
    );
}

/// The hero model with the `prod` request-timeout override retuned — one value,
/// in one environment.
fn retuned_prod_timeout(label: &str) -> ModelFixture {
    compile_fixture(
        label,
        HERO_DEFS,
        &edit_once(HERO_COMPONENTS, "\"value\": 5000", "\"value\": 4000"),
    )
}

#[test]
fn diff_detects_a_value_change_in_one_cell_only() {
    // T3 (REQ-CFX-005): the headline behaviour. A change guarded by
    // `environment == 'prod'` reaches exactly ONE deployment, and the report
    // must say so — naming the parameter and both values — while the other two
    // environments stay silent. Exit 1 because something changed.
    let base = compile_fixture("diff-base", HERO_DEFS, HERO_COMPONENTS);
    let head = retuned_prod_timeout("diff-head");
    let manifest = write_fixture_file(&base, "environments.json", HERO_MANIFEST);

    let (code, stdout, err) = run_args(&diff_args(&base.manifest, &head.manifest, &manifest));
    assert_eq!(code, EXIT_DIFFERENCES, "a changed target must exit 1: {err}");

    assert_eq!(
        status_lines(&stdout),
        vec![
            "unchanged  dev  component:webapp",
            "changed  prod  component:webapp",
            "unchanged  staging  component:webapp",
        ],
        "only prod may change; stdout:\n{stdout}"
    );
    assert_eq!(
        cell_body(&stdout, "changed  prod  component:webapp"),
        vec!["~ component.webapp.param.request_timeout_ms: 5000 -> 4000"],
        "exactly one change line, with both values; stdout:\n{stdout}"
    );
    assert_eq!(
        summary_line(&stdout),
        "summary: unchanged=2 changed=1 now_unsatisfiable=0 now_satisfiable=0 \
         unsatisfiable_both=0"
    );
}

#[test]
fn diff_is_deterministic() {
    // T8 (ADR-0042 §3): two runs over the same two models print byte-identical
    // stdout. No timestamps, no absolute paths, no map iteration order leaking
    // out — the report is a function of the inputs alone, which is what lets CI
    // compare it against a committed expectation.
    let base = compile_fixture("diff-determinism-base", HERO_DEFS, HERO_COMPONENTS);
    let head = retuned_prod_timeout("diff-determinism-head");
    let manifest = write_fixture_file(&base, "environments.json", HERO_MANIFEST);
    let args = diff_args(&base.manifest, &head.manifest, &manifest);

    let (first_code, first, _) = run_args(&args);
    let (second_code, second, _) = run_args(&args);
    assert_eq!(first_code, second_code);
    assert_eq!(first, second, "two runs must print identical stdout");

    // ...and neither model path appears in it: a model's identity is its hash,
    // and an absolute path would make the report machine-specific.
    assert!(
        !first.contains(&base.manifest) && !first.contains(&head.manifest),
        "the --base/--head paths must never be echoed: {first}"
    );
}

#[test]
fn diff_unrelated_edit_is_unchanged() {
    // T4 — the epic's headline property. `resolve_hash` folds `model_hash` into
    // its pre-image, so ANY edit rotates it for EVERY target; a diff built on
    // it would flag all three environments here. The extra chunk declares a
    // definition nothing inherits, so `model_hash` moves and not one delivered
    // byte does. Exit 0 is the whole point: an unrelated edit must not fail a
    // pull-request check.
    let base = compile_fixture("diff-unrelated-base", HERO_DEFS, HERO_COMPONENTS);
    let head = compile_fixture_chunks(
        "diff-unrelated-head",
        &[
            ("00_definitions.json", HERO_DEFS),
            ("10_components.json", HERO_COMPONENTS),
            (
                "20_extra_definitions.json",
                r#"{"package":"extra_defs","version":"1.0.0","definitions":{
                    "unused_knob":{"type":"integer","unit":"ms","lifecycle":"startup",
                    "access":"integrator","doc":"Declared and inherited by nothing."}}}"#,
            ),
        ],
    );
    let manifest = write_fixture_file(&base, "environments.json", HERO_MANIFEST);

    let mut args = diff_args(&base.manifest, &head.manifest, &manifest);
    args.extend(["--format", "json"]);
    let (code, stdout, err) = run_args(&args);
    assert_eq!(code, EXIT_OK, "an unrelated edit must exit 0: {err}");

    let report: serde_json::Value = serde_json::from_str(&stdout).expect("report parses");
    assert_ne!(
        report["base_model_hash"], report["head_model_hash"],
        "the two models MUST differ — otherwise this proves nothing"
    );
    assert_eq!(report["summary"]["unchanged"], 3);
    assert_eq!(report["summary"]["changed"], 0);
    for cell in report["cells"].as_array().expect("cells is an array") {
        assert_eq!(cell["status"], "unchanged", "cell: {cell}");
        // The delivered payload is identical while the RESOLUTION identity is
        // not — the two hashes disagreeing is the mechanism under test.
        assert_eq!(
            cell["base_resolved_output_hash"], cell["head_resolved_output_hash"],
            "cell: {cell}"
        );
        assert_ne!(cell["base_resolve_hash"], cell["head_resolve_hash"], "cell: {cell}");
    }
}

// ---------------------------------------------------------------------------
// Satisfiability transitions
// ---------------------------------------------------------------------------

/// The hero definitions plus a constraint that forbids `staging`'s own
/// `beta_dashboard=on` choice, so exactly one manifest environment stops being
/// resolvable.
fn staging_forbidding_defs() -> String {
    edit_once(
        HERO_DEFS,
        "\"constraints\": {",
        "\"constraints\": {\n    \"staging_forbids_beta\": {\n      \"condition\": \
         \"environment != 'staging' || beta_dashboard != 'on'\",\n      \"doc\": \
         \"The beta dashboard is not permitted in staging.\"\n    },",
    )
}

#[test]
fn diff_now_unsatisfiable_and_its_mirror() {
    // T5 (ADR-0059 D4): a policy added on the head side makes one committed
    // target unresolvable. That is a CELL STATUS, not a command failure — the
    // other two cells still get verdicts — and the cell must name the
    // diagnostic AND the constraint that rejected it, or the reviewer needs a
    // second command to find out why. Swapping the sides is the same event read
    // backwards: the policy was REMOVED, and the target became resolvable.
    let base = compile_fixture("diff-unsat-base", HERO_DEFS, HERO_COMPONENTS);
    let head = compile_fixture("diff-unsat-head", &staging_forbidding_defs(), HERO_COMPONENTS);
    let manifest = write_fixture_file(&base, "environments.json", HERO_MANIFEST);

    let (code, stdout, err) = run_args(&diff_args(&base.manifest, &head.manifest, &manifest));
    assert_eq!(code, EXIT_DIFFERENCES, "a status change must exit 1: {err}");
    assert_eq!(
        status_lines(&stdout),
        vec![
            "unchanged  dev  component:webapp",
            "unchanged  prod  component:webapp",
            "now_unsatisfiable  staging  component:webapp",
        ],
        "stdout:\n{stdout}"
    );
    let body = cell_body(&stdout, "now_unsatisfiable  staging  component:webapp");
    assert_eq!(body.len(), 1, "one rejected side, one line: {body:?}");
    assert!(body[0].starts_with("head: E_SELECTION_CONFLICT: "), "{body:?}");
    assert!(
        body[0].contains("staging_forbids_beta"),
        "the rejection must name the constraint: {body:?}"
    );
    assert_eq!(
        summary_line(&stdout),
        "summary: unchanged=2 changed=0 now_unsatisfiable=1 now_satisfiable=0 \
         unsatisfiable_both=0"
    );

    // The mirror: the same two models, swapped.
    let (mirror_code, mirror, mirror_err) =
        run_args(&diff_args(&head.manifest, &base.manifest, &manifest));
    assert_eq!(mirror_code, EXIT_DIFFERENCES, "{mirror_err}");
    assert_eq!(
        status_lines(&mirror),
        vec![
            "unchanged  dev  component:webapp",
            "unchanged  prod  component:webapp",
            "now_satisfiable  staging  component:webapp",
        ],
        "stdout:\n{mirror}"
    );
    let mirror_body = cell_body(&mirror, "now_satisfiable  staging  component:webapp");
    assert!(
        mirror_body[0].starts_with("base: E_SELECTION_CONFLICT: "),
        "the rejected side is now `base`: {mirror_body:?}"
    );
    assert_eq!(
        summary_line(&mirror),
        "summary: unchanged=2 changed=0 now_unsatisfiable=0 now_satisfiable=1 \
         unsatisfiable_both=0"
    );
}

// ---------------------------------------------------------------------------
// The JSON envelope and the usage surface
// ---------------------------------------------------------------------------

#[test]
fn diff_json_shape_and_order() {
    // T6 (ADR-0059 D4/M6): the envelope's key order is part of the contract, so
    // it is asserted on the RAW bytes — parsing into a `Value` would re-sort the
    // maps and hide exactly the defect this pins.
    let base = compile_fixture("diff-json-base", HERO_DEFS, HERO_COMPONENTS);
    let head = retuned_prod_timeout("diff-json-head");
    let manifest = write_fixture_file(&base, "environments.json", HERO_MANIFEST);

    let mut args = diff_args(&base.manifest, &head.manifest, &manifest);
    args.extend(["--format", "json"]);
    let (code, stdout, err) = run_args(&args);
    assert_eq!(code, EXIT_DIFFERENCES, "stderr: {err}");
    assert!(stdout.ends_with("\n"), "the envelope carries a trailing newline");
    assert_eq!(
        stdout.lines().count(),
        1,
        "nothing but the envelope on stdout:\n{stdout}"
    );

    let key_order = [
        "\"schema_version\"",
        "\"base_model_hash\"",
        "\"head_model_hash\"",
        "\"cells\"",
        "\"environment\"",
        "\"scope\"",
        "\"status\"",
        "\"changes\"",
        "\"rejections\"",
        "\"summary\"",
    ];
    let mut at = 0;
    for key in key_order {
        let found = stdout[at..]
            .find(key)
            .unwrap_or_else(|| panic!("{key} must appear after the preceding keys:\n{stdout}"));
        at += found;
    }

    let report: serde_json::Value = serde_json::from_str(&stdout).expect("report parses");
    assert_eq!(report["schema_version"], PRODUCT_SCHEMA_VERSION);
    let cells = report["cells"].as_array().expect("cells is an array");
    let seen: Vec<&str> = cells
        .iter()
        .map(|cell| cell["environment"].as_str().expect("environment is a string"))
        .collect();
    assert_eq!(seen, vec!["dev", "prod", "staging"], "cells must be sorted");
    assert_eq!(report["summary"]["unchanged"], 2);
    assert_eq!(report["summary"]["changed"], 1);
    assert_eq!(report["summary"]["now_unsatisfiable"], 0);

    let changes = cells[1]["changes"].as_array().expect("changes is an array");
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0]["path"], "component.webapp.param.request_timeout_ms");
    assert_eq!(changes[0]["field"], "value");
    assert_eq!(changes[0]["kind"], "changed");
    assert_eq!(changes[0]["before"], 5000);
    assert_eq!(changes[0]["after"], 4000);
}

#[test]
fn diff_usage_errors_name_their_cause() {
    // T7: every input problem is exit 2 and says WHAT was wrong. `--select` in
    // particular is REFUSED rather than ignored — a diff compares committed
    // targets, and silently dropping an ad-hoc choice would report on a
    // deployment nobody ships.
    let fixture = compile_fixture("diff-usage", HERO_DEFS, HERO_COMPONENTS);
    let manifest = write_fixture_file(&fixture, "environments.json", HERO_MANIFEST);
    let model = fixture.manifest.as_str();

    let cases: [(&str, Vec<&str>, &str); 4] = [
        (
            "a missing --manifest",
            vec!["cfx", "diff", "--base", model, "--head", model],
            "--manifest",
        ),
        (
            "--select",
            {
                let mut args = diff_args(model, model, &manifest);
                args.extend(["--select", "environment=prod"]);
                args
            },
            "--select",
        ),
        (
            "an unknown --environment",
            {
                let mut args = diff_args(model, model, &manifest);
                args.extend(["--environment", "prd"]);
                args
            },
            "prd",
        ),
        (
            "an unopenable --base",
            diff_args("/nonexistent/cmp.manifest.json", model, &manifest),
            "base",
        ),
    ];

    for (what, args, offender) in cases {
        let (code, stdout, err) = run_args(&args);
        assert_eq!(code, EXIT_USAGE, "{what} must be a usage error; stderr: {err}");
        assert!(
            err.contains(offender),
            "{what} must name '{offender}' on stderr: {err}"
        );
        assert!(stdout.is_empty(), "{what} must print no report: {stdout}");
    }

    // The unknown name also lists the ones that DO exist, so the typo is
    // fixable without opening the manifest.
    let mut args = diff_args(model, model, &manifest);
    args.extend(["--environment", "prd"]);
    let (_code, _out, err) = run_args(&args);
    assert!(err.contains("dev, prod, staging"), "stderr: {err}");
}

// ---------------------------------------------------------------------------
// Four repositories, three services, two sites
// ---------------------------------------------------------------------------

/// The example's model, from all FOUR of its units, with one chunk replaced by
/// the caller's edited copy.
///
/// This list tracks the example's UNITS, not the assertions': these tests drive
/// the SHIPPED `environments.json`, which binds `sorter_lanes` — a facet only
/// the sorter unit declares — so a missing unit kills every cell with
/// `E_SELECTION_UNKNOWN_FACET` before anything resolves. That coupling is
/// asserted, not merely documented, by
/// `polyrepo_model_declares_every_facet_the_shipped_manifest_binds`.
fn polyrepo(label: &str, catalogue: &str, vision: &str) -> ModelFixture {
    compile_fixture_chunks(
        label,
        &[
            ("00_catalogue.json", catalogue),
            ("10_compute.json", POLY_COMPUTE),
            ("10_vision.json", vision),
            ("20_sorter.json", POLY_SORTER),
        ],
    )
}

/// `cfx diff` over the polyrepo model, every site × every service scope.
fn polyrepo_diff(base: &ModelFixture, head: &ModelFixture, manifest: &str) -> (u8, String, String) {
    let mut args = diff_args(&base.manifest, &head.manifest, manifest);
    args.extend(["--scopes", POLY_SCOPES]);
    run_args(&args)
}

#[test]
fn polyrepo_model_declares_every_facet_the_shipped_manifest_binds() {
    // configflux-gmla. The enforcement the comments in `polyrepo` and in
    // CFX_TEST_COMPILE_DATA could only ask for. These tests feed the SHIPPED
    // manifest to a model compiled from a HAND-LISTED chunk set, so the two are
    // coupled: every facet the manifest pins must be one the model declares, or
    // `cfx` rejects the selection before resolving anything. A unit added to the
    // example that declares a newly-bound facet used to surface here as exit 2
    // where 1 was expected, in whichever assertion ran first — a verdict about
    // the diff report, failing for a reason that has nothing to do with diffing.
    // It now fails HERE, naming the facet and the two lists to add the chunk to.
    //
    // Both sides are read, not restated: the bound set is the manifest's own
    // `choices` keys through the parser `cfx diff` uses, and the declared set is
    // `list_selection_facets` — the same facet universe `cfx options` lists.
    let fixture = polyrepo("diff-poly-facets", POLY_CATALOGUE, POLY_VISION);
    let manifest_path = write_fixture_file(&fixture, "environments.json", POLY_MANIFEST);

    let manifest = crate::manifest::load(std::path::Path::new(&manifest_path))
        .expect("the shipped example manifest must parse");
    let bound: std::collections::BTreeSet<&str> = manifest
        .environments
        .values()
        .flat_map(|environment| environment.choices.keys().map(String::as_str))
        .collect();
    // A manifest that pinned nothing would make the assertion below vacuous and
    // the guard silent — exactly the state this test exists to end.
    assert!(
        !bound.is_empty(),
        "the example manifest must pin at least one facet, or this guard proves nothing"
    );

    let handle = crate::pipeline::open(std::path::Path::new(&fixture.manifest))
        .expect("the polyrepo fixture must open");
    let declared: std::collections::BTreeSet<String> =
        compiler::loader_api::list_selection_facets(&handle)
            .expect("a compiled model must list its facets")
            .into_iter()
            .collect();

    let undeclared: Vec<String> = bound
        .iter()
        .filter(|facet| !declared.contains(**facet))
        .map(|facet| {
            format!("manifest binds facet `{facet}` but the compiled model does not declare it")
        })
        .collect();
    assert!(
        undeclared.is_empty(),
        "{}; add the chunk that declares it to `polyrepo` and to CFX_TEST_COMPILE_DATA in \
         cfx/BUILD.bazel. The model declares: {}",
        undeclared.join("; "),
        declared.iter().cloned().collect::<Vec<_>>().join(", ")
    );
}

#[test]
fn diff_isolates_an_edit_to_the_service_whose_scope_contains_it() {
    // T9, first half. The model is composed from four repositories; the edit
    // lands in the vision repository only. Both vision cells must report it and
    // the four cells of the other two services must stay silent — even though
    // `model_hash`, and therefore every cell's `resolve_hash`, rotated. This is
    // the case the epic measured by hand before the verb existed.
    let base = polyrepo("diff-poly-base", POLY_CATALOGUE, POLY_VISION);
    let head = polyrepo(
        "diff-poly-vision",
        POLY_CATALOGUE,
        &edit_once(POLY_VISION, "\"value\": 40", "\"value\": 50"),
    );
    let manifest = write_fixture_file(&base, "environments.json", POLY_MANIFEST);

    let (code, stdout, err) = polyrepo_diff(&base, &head, &manifest);
    assert_eq!(code, EXIT_DIFFERENCES, "stderr: {err}");
    assert_eq!(
        status_lines(&stdout),
        vec![
            "unchanged  factory_a  component:compute_service",
            "unchanged  factory_a  component:sorter_service",
            "changed  factory_a  component:vision_service",
            "unchanged  factory_b  component:compute_service",
            "unchanged  factory_b  component:sorter_service",
            "changed  factory_b  component:vision_service",
        ],
        "stdout:\n{stdout}"
    );
    for site in ["factory_a", "factory_b"] {
        assert_eq!(
            cell_body(&stdout, &format!("changed  {site}  component:vision_service")),
            vec!["~ component.vision_service.param.roi_margin_mm: 40 -> 50"],
            "stdout:\n{stdout}"
        );
    }
    assert_eq!(
        summary_line(&stdout),
        "summary: unchanged=4 changed=2 now_unsatisfiable=0 now_satisfiable=0 \
         unsatisfiable_both=0"
    );
}

#[test]
fn diff_follows_a_shared_catalogue_edit_into_every_site_that_selects_it() {
    // T9, second half. The edit lands in the SHARED catalogue, on the container
    // `c2` — which only `factory_b` is equipped for. Both of that site's
    // services see it, because both require the binding that resolves to `c2`;
    // neither of `factory_a`'s does, because its binding resolves to `c1`. The
    // report follows the selection, not the file that changed.
    //
    // The changed path is the service's OWN requirement slot (ADR-0057 §D7), so
    // the report names the consumer that is affected rather than the catalogue
    // that was edited — and it is the same path the runtime read API uses, so a
    // line can be taken out of the report and read back off a device.
    //
    // The sorter is what makes that statement sharp (configflux-gmla). It reads
    // the SAME `containers` catalogue as the other two services, so a report
    // keyed on the file that changed would flag it; it reaches that table
    // through `sorter_container`, which the manifest pins to `c3` at BOTH sites,
    // so a report keyed on the selection must not. Its two cells stay unchanged
    // while `factory_b`'s other two move — a negative case neither of the other
    // scopes can make, because both derive their container from `site`.
    let base = polyrepo("diff-poly-cat-base", POLY_CATALOGUE, POLY_VISION);
    let head = polyrepo(
        "diff-poly-cat-head",
        &edit_once(POLY_CATALOGUE, "\"width_mm\": 600", "\"width_mm\": 650"),
        POLY_VISION,
    );
    let manifest = write_fixture_file(&base, "environments.json", POLY_MANIFEST);

    let (code, stdout, err) = polyrepo_diff(&base, &head, &manifest);
    assert_eq!(code, EXIT_DIFFERENCES, "stderr: {err}");
    assert_eq!(
        status_lines(&stdout),
        vec![
            "unchanged  factory_a  component:compute_service",
            "unchanged  factory_a  component:sorter_service",
            "unchanged  factory_a  component:vision_service",
            "changed  factory_b  component:compute_service",
            "unchanged  factory_b  component:sorter_service",
            "changed  factory_b  component:vision_service",
        ],
        "stdout:\n{stdout}"
    );
    for service in ["compute_service", "vision_service"] {
        assert_eq!(
            cell_body(
                &stdout,
                &format!("changed  factory_b  component:{service}")
            ),
            vec![format!(
                "~ component.{service}.requires.container.width_mm: 600 -> 650"
            )],
            "stdout:\n{stdout}"
        );
    }

    // Said directly, not only via the header word: the sorter cell at the site
    // whose catalogue entry was edited carries no change line at all.
    assert!(
        cell_body(&stdout, "unchanged  factory_b  component:sorter_service").is_empty(),
        "the sorter resolves to c3 and must report nothing; stdout:\n{stdout}"
    );
}
