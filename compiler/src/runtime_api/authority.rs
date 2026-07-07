// SPDX-License-Identifier: BUSL-1.1

// Two-role authority split + act-now / ratify-later review items
// (configflux-ccql.9, ADR-0037 Decision 5, design doc sec 7 "Authority model").
//
// This is pure access-control decision logic, layered as a NEW governance axis
// — exactly as `OverrideIntent` (override_intent.rs) was layered on top of the
// existing `Lifecycle` immutability axis. It is deliberately NOT folded into
// `crate::schema::Role`: that enum is the parameter *access* axis (who may
// read/write a modeled parameter); authority here is the *act-vs-bless* tier
// (who may perform an override action and who may promote it fleet-wide). The
// two concerns are orthogonal and conflating them would both mis-model the
// ladder and drag non-neutral role names into a public surface.
//
// `authorize` is total, side-effect-free, and FAIL-CLOSED: there is no
// wildcard "allow" arm, promote-to-base is allow-listed to the owner tier only,
// and an unrecognized / unspecified role parses to a denial. No I/O, no
// network, no state mutation happens here — the actual recording of a raised
// review item is the ledger's job (`ledger::review`); this module only DECIDES
// (allow/deny + whether a review item must be raised) and MINTS the neutral
// review-item payload when an act is allowed.
//
// Topology-neutral by hard repo constraint: the shipped identifiers and error
// strings use a generic operator-vs-owner vocabulary. ADR-0037 phrases the
// tiers as "technician / developer / product owner" internally; that framing
// stays in doc-comments only. A `developer` back-home hotfix and an on-site
// `technician` are both operator-tier actors; the `product owner` is the owner
// tier. No edge / industrial / robot / field framing appears in any public
// name or message.
//
// This file is `include!`d into the `runtime_api` flat namespace via
// `operations.rs` (so `//!` module docs are not permitted here); it shares the
// crate-root `serde` / `serde_json` imports and references sibling types
// without `use`.

/// Fixed, generic denial code for an authority check. Topology-neutral and
/// payload-free: it names only the failure category (a role was not authorized
/// for an action), never the role, the unit, or any value. Matches the
/// `E_*`-prefixed fail-closed taxonomy used across the runtime and ledger.
pub const E_AUTHORITY_ROLE_DENIED: &str = "E_AUTHORITY_ROLE_DENIED";

/// The authority tier of an actor (ADR-0037 Decision 5, design doc sec 7). This
/// is the governance axis for override *actions*, distinct from the parameter
/// *access* axis (`crate::schema::Role`).
///
/// - `Operator`: the **act** tier. May create a unit-scoped override and perform
///   a LOCAL COMMIT at most (the act-now path) — even offline. Covers both the
///   on-site technician and the back-home developer applying a remote hotfix
///   (ADR-0037 phrases these as "technician / developer"; a developer is an
///   operator-tier actor). An operator may **never** promote to base.
/// - `Owner`: the **bless** tier. May extend / normalize / **promote to base**
///   (fleet-scoped). ADR-0037 phrases this as the "product owner".
///
/// Deliberately two-valued: the ladder is act vs. bless. There is no third
/// "super" tier — promotion permanence belongs to the model (a PR), not to an
/// ever-more-privileged role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRole {
    /// Act tier: create override / local-commit at most. Allowed offline.
    Operator,
    /// Bless tier: extend / normalize / promote-to-base (fleet-scoped).
    Owner,
}

impl AuthorityRole {
    /// Parse a wire role string into a known [`AuthorityRole`], or `None` for an
    /// unrecognized / unspecified role. The `None` case is the FAIL-CLOSED hook:
    /// an unknown role can never be authorized for any action (see
    /// [`authorize_wire_role`]). Accepts exactly the serde `snake_case` forms.
    pub fn from_wire(role: &str) -> Option<Self> {
        match role {
            "operator" => Some(AuthorityRole::Operator),
            "owner" => Some(AuthorityRole::Owner),
            _ => None,
        }
    }
}

/// An override governance action whose authority is being checked (ADR-0037
/// Decision 5, promotion ladder in design doc sec 6 / override_intent.rs).
///
/// - `CreateOverride`: open an ephemeral, unit-scoped override (act tier).
/// - `LocalCommit`: persist the override as "current for this unit" (the
///   act-now path, the second ladder rung). Allowed offline, but it is
///   **flagged** and MUST raise a review item home — no silent privileged
///   action.
/// - `PromoteToBase`: make the value "the value for everything", fleet-scoped
///   (the top ladder rung). The irreversible, fleet-wide step — gated to the
///   owner tier ONLY.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityAction {
    /// Open an ephemeral unit-scoped override (act tier).
    CreateOverride,
    /// Persist a unit-scoped override (act-now path) — flagged, raises a review
    /// item home.
    LocalCommit,
    /// Promote a value to base, fleet-scoped (owner tier only).
    PromoteToBase,
}

/// Why an authority check denied an action. Carries the fixed neutral code
/// [`E_AUTHORITY_ROLE_DENIED`] and a generic message; it never echoes the role,
/// unit, or value. A denial mutates no state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityDenial {
    pub code: String,
    pub message: String,
}

impl AuthorityDenial {
    fn role_denied() -> Self {
        Self {
            code: E_AUTHORITY_ROLE_DENIED.to_string(),
            message: "role is not authorized for this action".to_string(),
        }
    }
}

/// The outcome of an authority check (ADR-0037 Decision 5). A pure value: it
/// records whether the action is `allowed`, whether performing it MUST raise a
/// review item home (`raises_review_item` — the act-now / ratify-later rule),
/// and the `denial` when not allowed.
///
/// Invariants (held by [`authorize`], asserted in tests):
/// - `allowed == denial.is_none()` (a decision is exactly one of allow/deny);
/// - `raises_review_item` is only ever `true` when `allowed` is `true` (a denied
///   action raises nothing — it never happened);
/// - a local commit that is allowed ALWAYS sets `raises_review_item == true`
///   (no silent privileged action).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityDecision {
    /// Whether the action is permitted for the role.
    pub allowed: bool,
    /// Whether performing the (allowed) action MUST raise a review item home.
    /// Always `false` when `allowed` is `false`.
    pub raises_review_item: bool,
    /// The denial reason when `allowed` is `false`; `None` when allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denial: Option<AuthorityDenial>,
}

impl AuthorityDecision {
    fn allow(raises_review_item: bool) -> Self {
        Self {
            allowed: true,
            raises_review_item,
            denial: None,
        }
    }

    fn deny() -> Self {
        Self {
            allowed: false,
            raises_review_item: false,
            denial: Some(AuthorityDenial::role_denied()),
        }
    }

    /// Whether the action is permitted.
    pub fn is_allowed(&self) -> bool {
        self.allowed
    }
}

/// A review item raised home by an act-now local commit (ADR-0037 Decision 5,
/// design doc sec 7 "act-now, ratify-later"). Topology-neutral payload: it
/// carries who acted, the unit and path, the action, an optional reason, and
/// when — enough for a product owner to ratify later, with no edge/industrial
/// framing.
///
/// A review item is minted ONLY for an allowed act (see
/// [`local_commit_review_item`]); a denied action produces no review item,
/// because it never happened. The ledger (`ledger::review`) is the home that
/// records these; this struct is the shared contract carried across that seam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaisedReviewItem {
    /// Who performed the act (operator identity: technician or developer).
    pub actor: String,
    /// The unit (device) the act was performed on.
    pub unit: String,
    /// The configuration path the override applies to.
    pub path: String,
    /// The action that raised this item (a `LocalCommit` for the act-now path).
    pub action: AuthorityAction,
    /// Why the act was performed, if recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the act happened, unix milliseconds.
    pub timestamp_unix_ms: u64,
}

/// The pure authority decision: may `role` perform `action`, and does performing
/// it raise a review item (ADR-0037 Decision 5)?
///
/// FAIL-CLOSED by construction: the match is exhaustive over the closed
/// `(AuthorityRole, AuthorityAction)` product, every permitted pair is
/// explicitly allow-listed, and there is NO wildcard "allow" arm. (An
/// unrecognized / unspecified role never reaches this function as a typed value;
/// it is denied at the parse boundary — see [`authorize_wire_role`].)
///
/// The act-now / ratify-later rule is encoded directly: a `LocalCommit` allowed
/// for any actor ALWAYS raises a review item; `PromoteToBase` is allow-listed to
/// `Owner` ONLY and an `Operator` attempting it is denied with no state change.
pub fn authorize(role: AuthorityRole, action: AuthorityAction) -> AuthorityDecision {
    match (role, action) {
        // Act tier — both an operator and an owner may act. Opening an ephemeral
        // unit-scoped override raises no review item by itself; persisting it
        // (a LOCAL COMMIT) is the act-now path and ALWAYS raises a review item
        // home, because no privileged action may be silent.
        (AuthorityRole::Operator, AuthorityAction::CreateOverride)
        | (AuthorityRole::Owner, AuthorityAction::CreateOverride) => {
            AuthorityDecision::allow(false)
        }
        (AuthorityRole::Operator, AuthorityAction::LocalCommit)
        | (AuthorityRole::Owner, AuthorityAction::LocalCommit) => AuthorityDecision::allow(true),

        // Bless tier — promote-to-base is allow-listed to the OWNER tier ONLY.
        // It is the fleet-wide ratification itself, so it raises no further
        // review item.
        (AuthorityRole::Owner, AuthorityAction::PromoteToBase) => AuthorityDecision::allow(false),

        // FAIL CLOSED: every pair not explicitly allow-listed above is denied.
        // The only member that falls here is an operator attempting
        // promote-to-base; spelling the deny arm exhaustively (rather than a
        // wildcard) keeps the allow-list auditable and additive-safe.
        (AuthorityRole::Operator, AuthorityAction::PromoteToBase) => AuthorityDecision::deny(),
    }
}

/// Fail-closed wire-role entry point: parse `role` and authorize, denying any
/// unrecognized / unspecified role for EVERY action. This is the boundary that
/// makes "an unknown role is denied" structural rather than advisory — the
/// parse failure short-circuits to a denial before any allow-listing runs.
pub fn authorize_wire_role(role: &str, action: AuthorityAction) -> AuthorityDecision {
    match AuthorityRole::from_wire(role) {
        Some(known) => authorize(known, action),
        None => AuthorityDecision::deny(),
    }
}

/// Mint the review item an allowed act-now local commit must raise home
/// (ADR-0037 Decision 5). Returns `Some(item)` ONLY when `decision` is an
/// allowed decision that raises a review item; returns `None` otherwise (a
/// denied or non-raising decision raises nothing). Tying minting to the decision
/// value makes "no silent privileged action" structural: a caller cannot mint a
/// review item for an act the authority check did not allow.
pub fn local_commit_review_item(
    decision: &AuthorityDecision,
    actor: impl Into<String>,
    unit: impl Into<String>,
    path: impl Into<String>,
    reason: Option<String>,
    timestamp_unix_ms: u64,
) -> Option<RaisedReviewItem> {
    if !(decision.allowed && decision.raises_review_item) {
        return None;
    }
    Some(RaisedReviewItem {
        actor: actor.into(),
        unit: unit.into(),
        path: path.into(),
        action: AuthorityAction::LocalCommit,
        reason,
        timestamp_unix_ms,
    })
}
