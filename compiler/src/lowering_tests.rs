// SPDX-License-Identifier: BUSL-1.1

//! ADR-0057 §D4 lowering: the exact conjunct text, the exact attribution ids,
//! and the exact order.
//!
//! These are pinned by example rather than described, because all three are
//! contract surfaces: the text is what `cfx explain` quotes back, the ids are
//! what a JSON envelope carries as `constraint_id`, and the order is what fixes
//! each conjunct's `root_index` in the ADR-0054 §5.4 roster. A change to any of
//! them rotates `ccm_hash` for every model that uses the feature.

use super::*;
use crate::schema::{Binding, Component, Requirement};

fn binding_with_derive(catalogue: &str, source: &str, pairs: &[(&str, &str)]) -> Binding {
    let inner: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut table = BTreeMap::new();
    table.insert(source.to_string(), inner);
    Binding {
        catalogue: catalogue.to_string(),
        default: None,
        derive: Some(table),
        doc: None,
    }
}

fn component(condition: Option<&str>, requires: &[(&str, Requirement)]) -> Component {
    Component {
        r#type: None,
        condition: condition.map(str::to_string),
        depends_on: Vec::new(),
        requires: requires
            .iter()
            .map(|(slot, requirement)| (slot.to_string(), requirement.clone()))
            .collect(),
        params: Default::default(),
    }
}

fn accepts(binding: &str, entries: &[&str]) -> Requirement {
    Requirement {
        binding: binding.to_string(),
        accepts: Some(entries.iter().map(|e| e.to_string()).collect()),
    }
}

fn bare(binding: &str) -> Requirement {
    Requirement {
        binding: binding.to_string(),
        accepts: None,
    }
}

fn map<T>(pairs: Vec<(&str, T)>) -> BTreeMap<String, T> {
    pairs
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

// --- derive ------------------------------------------------------------------

#[test]
fn a_derive_pair_lowers_to_an_implication() {
    let bindings = map(vec![(
        "line_container",
        binding_with_derive("containers", "site", &[("factory_a", "c1")]),
    )]);
    let lowered = derive_conjuncts(&bindings);
    assert_eq!(lowered.len(), 1, "{lowered:?}");
    assert_eq!(lowered[0].id, "derive:line_container:site=factory_a");
    assert_eq!(
        lowered[0].condition,
        "site != 'factory_a' || line_container == 'c1'"
    );
    assert_eq!(lowered[0].origin, "line_container");
}

#[test]
fn every_derive_pair_gets_its_own_conjunct_in_source_value_order() {
    let bindings = map(vec![(
        "line_container",
        binding_with_derive(
            "containers",
            "site",
            // Authored out of order on purpose: the walk must sort.
            &[("factory_b", "c2"), ("factory_a", "c1")],
        ),
    )]);
    let ids: Vec<String> = derive_conjuncts(&bindings)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(
        ids,
        vec![
            "derive:line_container:site=factory_a",
            "derive:line_container:site=factory_b"
        ]
    );
}

#[test]
fn a_binding_without_derive_lowers_to_nothing() {
    let bindings = map(vec![(
        "line_container",
        Binding {
            catalogue: "containers".to_string(),
            default: Some("c1".to_string()),
            derive: None,
            doc: None,
        },
    )]);
    assert!(derive_conjuncts(&bindings).is_empty());
}

// --- accepts -----------------------------------------------------------------

#[test]
fn a_multi_entry_accepts_list_lowers_to_any_of_in_authored_order() {
    let components = map(vec![(
        "compute_service",
        // Authored c2 first: `any_of` keeps the AUTHORED order, not a sort.
        component(None, &[("container", accepts("line_container", &["c2", "c1"]))]),
    )]);
    let lowered = accepts_conjuncts(&components);
    assert_eq!(lowered.len(), 1, "{lowered:?}");
    assert_eq!(lowered[0].id, "accepts:compute_service.container");
    assert_eq!(
        lowered[0].condition,
        "any_of(line_container == 'c2', line_container == 'c1')"
    );
    assert_eq!(lowered[0].origin, "compute_service");
}

/// `any_of(...)` requires at least two arguments (ADR-0006 §3), so a
/// single-entry list lowers to the bare predicate — the same proposition, and
/// the only form that parses.
#[test]
fn a_single_entry_accepts_list_lowers_to_a_bare_predicate_that_parses() {
    let components = map(vec![(
        "compute_service",
        component(None, &[("container", accepts("line_container", &["c1"]))]),
    )]);
    let lowered = accepts_conjuncts(&components);
    assert_eq!(lowered[0].condition, "line_container == 'c1'");
    assert!(
        crate::conditions::parse_condition_expr(&lowered[0].condition).is_ok(),
        "the single-entry form must parse: {}",
        lowered[0].condition
    );
}

#[test]
fn a_requirement_without_accepts_lowers_to_nothing() {
    let components = map(vec![(
        "compute_service",
        component(None, &[("container", bare("line_container"))]),
    )]);
    assert!(accepts_conjuncts(&components).is_empty());
}

/// ADR-0054 §3: a component that is not included asserts nothing. The guard is
/// what keeps an excluded component from narrowing the binding for everyone.
#[test]
fn a_conditional_component_guards_its_accepts_conjunct() {
    let components = map(vec![(
        "compute_service",
        component(
            Some("mode == 'x' || mode == 'y'"),
            &[("container", accepts("line_container", &["c1"]))],
        ),
    )]);
    let lowered = accepts_conjuncts(&components);
    assert_eq!(
        lowered[0].condition,
        "!(mode == 'x' || mode == 'y') || line_container == 'c1'"
    );
    assert!(
        crate::conditions::parse_condition_expr(&lowered[0].condition).is_ok(),
        "the guarded form must parse: {}",
        lowered[0].condition
    );
}

#[test]
fn a_blank_condition_leaves_the_conjunct_unguarded() {
    let components = map(vec![(
        "compute_service",
        component(
            Some("   "),
            &[("container", accepts("line_container", &["c1"]))],
        ),
    )]);
    assert_eq!(
        accepts_conjuncts(&components)[0].condition,
        "line_container == 'c1'"
    );
}

// --- order and identity ------------------------------------------------------

/// §D4 fixes the fold order, and `root_index` is a position in that fold.
#[test]
fn derive_conjuncts_precede_accepts_conjuncts() {
    let bindings = map(vec![(
        "line_container",
        binding_with_derive("containers", "site", &[("factory_a", "c1")]),
    )]);
    let components = map(vec![
        (
            "vision_service",
            component(None, &[("container", accepts("line_container", &["c1"]))]),
        ),
        (
            "compute_service",
            component(None, &[("container", accepts("line_container", &["c1"]))]),
        ),
    ]);
    let ids: Vec<String> = lowered_root_conjuncts(&bindings, &components)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(
        ids,
        vec![
            "derive:line_container:site=factory_a",
            // Components id-ascending, so compute before vision regardless of
            // how the map happened to be built.
            "accepts:compute_service.container",
            "accepts:vision_service.container",
        ]
    );
}

#[test]
fn slots_of_one_component_lower_in_slot_ascending_order() {
    let components = map(vec![(
        "compute_service",
        component(
            None,
            &[
                ("secondary", accepts("line_container", &["c1"])),
                ("primary", accepts("line_container", &["c1"])),
            ],
        ),
    )]);
    let ids: Vec<String> = accepts_conjuncts(&components)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(
        ids,
        vec![
            "accepts:compute_service.primary",
            "accepts:compute_service.secondary",
        ]
    );
}

// --- reading a conjunct back --------------------------------------------------

/// The round trip is the claim [`describe_attribution`] rests on: what the
/// generator writes, the reader must recover. Driving it from
/// `lowered_root_conjuncts` rather than from hand-typed text is the point — a
/// change to the emitted shape that broke the reading fails HERE.
#[test]
fn every_lowered_conjunct_reads_back_into_its_authoring_construct() {
    let bindings = map(vec![(
        "line_container",
        binding_with_derive("containers", "site", &[("factory_a", "c1")]),
    )]);
    let components = map(vec![
        (
            "compute_service",
            component(None, &[("container", accepts("line_container", &["c1", "c2"]))]),
        ),
        (
            "edge_service",
            component(
                Some("mode == 'x' || mode == 'y'"),
                &[("container", accepts("line_container", &["c1"]))],
            ),
        ),
    ]);

    let read: Vec<Attribution> = lowered_root_conjuncts(&bindings, &components)
        .iter()
        .map(|c| {
            describe_attribution(&c.id, &c.condition)
                .unwrap_or_else(|| panic!("unreadable: {} => {}", c.id, c.condition))
        })
        .collect();

    assert_eq!(
        read,
        vec![
            Attribution::Derive {
                binding: "line_container".to_string(),
                source: "site".to_string(),
                source_value: "factory_a".to_string(),
                entry: "c1".to_string(),
            },
            Attribution::Accepts {
                component: "compute_service".to_string(),
                slot: "container".to_string(),
                entries: vec!["c1".to_string(), "c2".to_string()],
            },
            // The guard's own literals are not accepted entries — and this
            // guard contains a disjunction, which is what a naive scan for the
            // last `||` would read as the payload.
            Attribution::Accepts {
                component: "edge_service".to_string(),
                slot: "container".to_string(),
                entries: vec!["c1".to_string()],
            },
        ]
    );
}

/// A guard that itself contains `any_of(...)` must not have its arguments read
/// as the accepted entries.
#[test]
fn a_guard_containing_any_of_does_not_leak_into_the_accepted_entries() {
    let components = map(vec![(
        "edge_service",
        component(
            Some("any_of(mode == 'x', mode == 'y')"),
            &[("container", accepts("line_container", &["c2"]))],
        ),
    )]);
    let lowered = &accepts_conjuncts(&components)[0];
    assert_eq!(
        describe_attribution(&lowered.id, &lowered.condition),
        Some(Attribution::Accepts {
            component: "edge_service".to_string(),
            slot: "container".to_string(),
            entries: vec!["c2".to_string()],
        })
    );
}

/// An authored constraint id is not an attribution, and a lowered id whose text
/// has the wrong shape yields nothing rather than a guess.
#[test]
fn an_unreadable_or_authored_id_describes_nothing() {
    assert_eq!(
        describe_attribution("prod_forbids_debug", "environment != 'prod'"),
        None
    );
    assert_eq!(describe_attribution("accepts:broken", "nothing quotable"), None);
    assert_eq!(describe_attribution("derive:b:s=v", "nothing quotable"), None);
}

/// The collision argument, pinned: an authored constraint id is `#snakeId` and
/// cannot contain `:`, so neither reserved prefix can ever name one.
#[test]
fn both_attribution_prefixes_are_unreachable_by_an_authored_id() {
    for id in [
        derive_attribution_id("line_container", "site", "factory_a"),
        accepts_attribution_id("compute_service", "container"),
    ] {
        assert!(id.contains(':'), "{id}");
        assert!(
            !id.chars().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '_'),
            "an attribution id must not be spellable as a snake_case authored id: {id}"
        );
    }
}
