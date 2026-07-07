// SPDX-License-Identifier: BUSL-1.1

// Unit tests for the two-role authority split + act-now / ratify-later review
// items (configflux-ccql.9, ADR-0037 Decision 5, design doc sec 7). Split out of
// `authority.rs` so the production module stays under the 400-line file budget
// (mirroring the runtime_api `tests.rs` split). Wired as a `#[cfg(test)]` module
// from `mod.rs`; `super::*` reaches the flat `runtime_api` namespace the
// `include!`d `authority.rs` contributes to.
//
// These tests pin the access-control exit criteria: the act tier (local commit
// allowed for operator + owner, ALWAYS raising a review item), the bless tier
// (promote-to-base allow-listed to owner ONLY), fail-closed denial of unknown /
// unspecified roles, the totality + no-silent-allow invariants, and the
// topology-neutral wire vocabulary.

use super::*;

const ALL_ROLES: [AuthorityRole; 2] = [AuthorityRole::Operator, AuthorityRole::Owner];
const ALL_ACTIONS: [AuthorityAction; 3] = [
    AuthorityAction::CreateOverride,
    AuthorityAction::LocalCommit,
    AuthorityAction::PromoteToBase,
];

// --- act tier: local commit is allowed and ALWAYS raises a review item -------

#[test]
fn operator_local_commit_allowed_and_raises_review() {
    let decision = authorize(AuthorityRole::Operator, AuthorityAction::LocalCommit);
    assert!(decision.allowed, "an operator may perform a local commit (the act-now path)");
    assert!(
        decision.raises_review_item,
        "an allowed local commit must raise a review item — no silent privileged action"
    );
    assert!(decision.denial.is_none(), "an allowed decision carries no denial");
}

#[test]
fn owner_local_commit_allowed_and_raises_review() {
    let decision = authorize(AuthorityRole::Owner, AuthorityAction::LocalCommit);
    assert!(decision.allowed, "the owner tier may also act (local commit)");
    assert!(
        decision.raises_review_item,
        "even an owner's local commit raises a review item (act-now path is always flagged)"
    );
}

#[test]
fn operator_create_override_allowed() {
    let decision = authorize(AuthorityRole::Operator, AuthorityAction::CreateOverride);
    assert!(decision.allowed, "an operator may open an ephemeral override (act tier)");
}

// --- bless tier: promote-to-base is owner-only -------------------------------

#[test]
fn operator_promote_to_base_denied_no_state() {
    let decision = authorize(AuthorityRole::Operator, AuthorityAction::PromoteToBase);
    assert!(
        !decision.allowed,
        "an operator (non-owner) must be REJECTED for promote-to-base"
    );
    assert!(
        !decision.raises_review_item,
        "a denied action raises no review item — it never happened"
    );
    let denial = decision.denial.expect("a denial carries a reason");
    assert_eq!(denial.code, E_AUTHORITY_ROLE_DENIED, "fixed neutral denial code");
    // The denial message is generic and leaks neither role nor value.
    assert!(!denial.message.contains("operator"), "denial message stays generic");
}

#[test]
fn owner_promote_to_base_allowed_no_review_item() {
    let decision = authorize(AuthorityRole::Owner, AuthorityAction::PromoteToBase);
    assert!(decision.allowed, "the owner tier may promote to base (bless)");
    assert!(
        !decision.raises_review_item,
        "a promote-to-base is the ratification itself; it does not raise a further review item"
    );
}

// --- fail-closed: unknown / unspecified role is denied for everything --------

#[test]
fn unknown_role_denied_for_promote_to_base() {
    // The driving acceptance case (d): an unspecified role cannot promote.
    let decision = authorize_wire_role("", AuthorityAction::PromoteToBase);
    assert!(!decision.allowed, "an unspecified (empty) role is denied promote-to-base");
    assert_eq!(decision.denial.expect("denied").code, E_AUTHORITY_ROLE_DENIED);
}

#[test]
fn unknown_role_denied_for_every_action() {
    // Default-deny, not default-allow: a role outside the known set is denied for
    // EVERY action, including the act-tier ones.
    for bogus in ["", "root", "admin", "superuser", "technician", "developer", "Owner "] {
        for action in ALL_ACTIONS {
            let decision = authorize_wire_role(bogus, action);
            assert!(
                !decision.allowed,
                "unknown role '{bogus}' must be denied action {action:?} (fail closed)"
            );
            assert!(!decision.raises_review_item);
        }
    }
}

#[test]
fn known_wire_roles_round_trip_through_authorization() {
    // The wire entry point agrees with the typed one for known roles.
    for role in ALL_ROLES {
        let wire = serde_json::to_value(role)
            .expect("serialize role")
            .as_str()
            .expect("role serializes to a string")
            .to_string();
        for action in ALL_ACTIONS {
            assert_eq!(
                authorize_wire_role(&wire, action),
                authorize(role, action),
                "wire-role authorization must equal typed authorization for known role {role:?}"
            );
        }
    }
}

// --- totality + the no-silent-allow invariant --------------------------------

#[test]
fn authorize_is_total_and_has_no_silent_allow() {
    // Every (role, action) yields a well-formed decision: allowed iff no denial,
    // and a review item is never raised on a denied decision.
    for role in ALL_ROLES {
        for action in ALL_ACTIONS {
            let decision = authorize(role, action);
            assert_eq!(
                decision.allowed,
                decision.denial.is_none(),
                "decision is exactly one of allow/deny for {role:?}/{action:?}"
            );
            if !decision.allowed {
                assert!(
                    !decision.raises_review_item,
                    "a denied {role:?}/{action:?} raises no review item"
                );
            }
        }
    }
}

#[test]
fn promote_to_base_is_allow_listed_to_owner_only() {
    // The cardinal access-control invariant: across ALL roles, promote-to-base is
    // allowed for exactly the owner tier and no one else.
    for role in ALL_ROLES {
        let allowed = authorize(role, AuthorityAction::PromoteToBase).allowed;
        assert_eq!(
            allowed,
            role == AuthorityRole::Owner,
            "promote-to-base allowed iff Owner; {role:?} got allowed={allowed}"
        );
    }
}

#[test]
fn local_commit_always_raises_review_when_allowed() {
    // Across ALL roles, whenever a local commit is allowed it raises a review item
    // — the act-now / ratify-later rule, with no exceptions.
    for role in ALL_ROLES {
        let decision = authorize(role, AuthorityAction::LocalCommit);
        if decision.allowed {
            assert!(
                decision.raises_review_item,
                "allowed local commit for {role:?} must raise a review item"
            );
        }
    }
}

// --- review-item minting is gated to an allowed act --------------------------

#[test]
fn review_item_minted_for_allowed_local_commit() {
    let decision = authorize(AuthorityRole::Operator, AuthorityAction::LocalCommit);
    let item = local_commit_review_item(
        &decision,
        "tech-7",
        "unit-42",
        "component.brake.param.threshold",
        Some("loosened until part swap".to_string()),
        1_700_000_000_000,
    )
    .expect("an allowed act-now local commit mints a review item");
    assert_eq!(item.actor, "tech-7");
    assert_eq!(item.unit, "unit-42");
    assert_eq!(item.action, AuthorityAction::LocalCommit);
    assert_eq!(item.reason.as_deref(), Some("loosened until part swap"));
}

#[test]
fn no_review_item_minted_for_denied_action() {
    // A denied act mints nothing — no silent privileged action can leave a
    // ratification breadcrumb it did not earn.
    let denied = authorize(AuthorityRole::Operator, AuthorityAction::PromoteToBase);
    assert!(
        local_commit_review_item(&denied, "tech-7", "unit-42", "p", None, 1).is_none(),
        "a denied action mints no review item"
    );
}

// --- serde: neutral wire names + round-trip stability ------------------------

#[test]
fn role_and_action_wire_names_are_topology_neutral_snake_case() {
    assert_eq!(serde_json::to_value(AuthorityRole::Operator).unwrap(), "operator");
    assert_eq!(serde_json::to_value(AuthorityRole::Owner).unwrap(), "owner");
    assert_eq!(
        serde_json::to_value(AuthorityAction::LocalCommit).unwrap(),
        "local_commit"
    );
    assert_eq!(
        serde_json::to_value(AuthorityAction::PromoteToBase).unwrap(),
        "promote_to_base"
    );
    assert_eq!(
        serde_json::to_value(AuthorityAction::CreateOverride).unwrap(),
        "create_override"
    );
    // No banned topology term appears in any identifier's wire form.
    for v in [
        serde_json::to_string(&AuthorityRole::Operator).unwrap(),
        serde_json::to_string(&AuthorityRole::Owner).unwrap(),
        serde_json::to_string(&AuthorityAction::CreateOverride).unwrap(),
        serde_json::to_string(&AuthorityAction::LocalCommit).unwrap(),
        serde_json::to_string(&AuthorityAction::PromoteToBase).unwrap(),
        E_AUTHORITY_ROLE_DENIED.to_string(),
    ] {
        let lower = v.to_lowercase();
        for banned in ["edge", "industrial", "robot", "fleet", "machine", "factory", "field"] {
            assert!(!lower.contains(banned), "wire identifier '{v}' must be topology-neutral");
        }
    }
}

#[test]
fn decision_round_trip_is_byte_stable() {
    let decision = authorize(AuthorityRole::Operator, AuthorityAction::PromoteToBase);
    let first = serde_json::to_vec(&decision).expect("serialize decision");
    let decoded: AuthorityDecision = serde_json::from_slice(&first).expect("deserialize decision");
    let second = serde_json::to_vec(&decoded).expect("re-serialize decision");
    assert_eq!(first, second, "serialize -> deserialize -> serialize is byte-stable");
    assert_eq!(decision, decoded, "round-trip preserves the decision");
}
