// SPDX-License-Identifier: BUSL-1.1

// Override-intent governance axis and lifecycle invariant (configflux-ccql.2,
// ADR-0037, design doc sec 6).
//
// This is pure data-model + decision logic. The `intent` field it introduces
// lives on `DirtyEntryMetadata` (defined in `contracts.rs`); the decision
// functions here are total, side-effect-free, and consumed by sibling tasks
// (the active cap/lease driver, ccql.4, and ledger ingest). No agent, no
// network, no active timer is built here.
//
// This file is `include!`d into the `runtime_api` flat namespace via
// `operations.rs` (so `//!` module docs are not permitted here); it shares the
// crate-root `serde` / `serde_json` imports and references sibling types
// (`DirtyEntryMetadata`) without `use`.

/// The governance intent of an override (ADR-0037 Decision 1, design doc
/// sec 6). This is the net-new governance axis layered on top of the existing
/// `Lifecycle` immutability axis (`crate::schema::Lifecycle`):
///
/// - `Experimental`: a tweak that may be abandoned. It may end **silently**
///   (lease expiry, hard cap, software upgrade) — cleaning up abandoned
///   experiments is the safe default.
/// - `Compensating`: a proxy for an unresolved physical condition (e.g.
///   loosening a threshold because a part is degraded "until the part is
///   changed"). It may **never** end silently — every terminal path is a
///   human-acknowledged decision or a state-gated, logged actuation, because
///   the system must never assume the physical condition resolved itself.
///
/// The default is `Experimental`: abandoned-experiment cleanup is the safe
/// default, and a compensating override must be declared explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideIntent {
    Experimental,
    Compensating,
}

impl Default for OverrideIntent {
    fn default() -> Self {
        OverrideIntent::Experimental
    }
}

/// A terminal event in an override's lifecycle (design doc sec 6 table). These
/// are the four ways an override can reach an end-of-life decision point. The
/// prescribed outcome for each `(TerminalEvent, OverrideIntent)` pair is given
/// by [`override_terminal_outcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEvent {
    /// Short session-lease expiry — abandoned-experiment cleanup.
    SessionLeaseExpiry,
    /// Long hard cap (<= 1 year, renewable with re-justification).
    HardCap,
    /// Software upgrade — the config version travels with the software version,
    /// so overrides are reconciled against the new model.
    SoftwareUpgrade,
    /// Upstream convergence — the override value now equals the new base.
    ValueEqualsBase,
}

/// The result of re-validating a compensating override against the new model on
/// a software upgrade (design doc sec 6 row 3, ADR-0037 Decision 6).
/// Experimental overrides are always wiped on upgrade and never reach this
/// branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeRevalidation {
    /// Still valid against the new model — carry the override forward.
    ValidAgainstNewModel,
    /// Needs human review against the new model — surface, do not silent-drop.
    NeedsReview,
    /// The parameter no longer exists in the new model — surface, do not
    /// silent-drop an obsoleted parameter.
    ParameterObsoleted,
    /// The override value now equals the new base — absorb, but keep the
    /// work-order linkage (a number match is not the physical condition being
    /// resolved).
    ConvergedToBase,
}

/// The prescribed outcome of an override terminal event (design doc sec 6).
///
/// The set is deliberately minimal but distinguishes the two properties the
/// governing invariant turns on:
/// - **silent vs. never-silent** (see [`TerminalOutcome::is_silent`]), and
/// - **keep-linkage** on absorb (whether the originating work-order / lineage
///   linkage is preserved when the value is absorbed into the base).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    /// Revert with no human-visible event (experimental lease / cap).
    SilentRevert,
    /// The override survives the event — it is needed, not abandoned
    /// (compensating lease expiry).
    Survive,
    /// Escalate for a human decision. Any resulting revert is observable and
    /// state-gated — never silent (compensating hard cap).
    Escalate,
    /// Discard the override entirely (experimental software upgrade).
    Wipe,
    /// Carry the override forward against the new model (compensating upgrade,
    /// still valid).
    Carry,
    /// Surface for human attention — never a silent drop (compensating upgrade
    /// needing review, or an obsoleted parameter).
    Surface,
    /// Absorb the value into the base and log it. `keep_linkage` records whether
    /// the originating work-order / lineage linkage is preserved: `false` for an
    /// experimental convergence (a plain silent absorb), `true` for a
    /// compensating convergence (a number match is not proof the physical
    /// condition resolved, so the linkage is kept).
    AbsorbAndLog { keep_linkage: bool },
}

impl TerminalOutcome {
    /// Whether this outcome ends the override **silently** (no human-visible
    /// event). The cardinal invariant of ADR-0037 Decision 2 is that no
    /// compensating terminal path is ever silent: for every `TerminalEvent`,
    /// `override_terminal_outcome(event, Compensating).is_silent()` is `false`.
    ///
    /// Only an experimental revert, an experimental wipe, and an experimental
    /// (non-linkage-keeping) absorb are silent.
    pub fn is_silent(self) -> bool {
        matches!(
            self,
            TerminalOutcome::SilentRevert
                | TerminalOutcome::Wipe
                | TerminalOutcome::AbsorbAndLog { keep_linkage: false }
        )
    }
}

/// The upper bound on the override hard cap: <= 1 year, in milliseconds
/// (ADR-0037 Decision 3). There is no "never roll back" mode — a hard cap can
/// never exceed this bound, and permanence is reachable only through the model
/// (promote-to-base or a per-unit modeled variant).
pub const HARD_CAP_MAX_MS: u64 = 365 * 24 * 60 * 60 * 1000;

/// The two time bounds an override lives under (ADR-0037 Decision 3, design doc
/// sec 6): a short `session_lease_ms` (abandoned-experiment cleanup) and a long
/// `hard_cap_ms` (<= [`HARD_CAP_MAX_MS`], renewable with re-justification).
///
/// There is deliberately **no** infinite / permanent variant: `hard_cap_ms` is
/// a finite `u64` constrained to `<= HARD_CAP_MAX_MS`, so the override layer is
/// strictly temporary. Permanence belongs in the model, not the override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverrideTimeBounds {
    /// Short lease for abandoned-experiment cleanup, in milliseconds.
    pub session_lease_ms: u64,
    /// Long hard cap, in milliseconds; always `<= HARD_CAP_MAX_MS`.
    pub hard_cap_ms: u64,
}

impl OverrideTimeBounds {
    /// Construct bounds, rejecting a hard cap that exceeds the one-year limit.
    /// Returns `None` when `hard_cap_ms > HARD_CAP_MAX_MS` (no "never roll back"
    /// mode is representable).
    pub fn new(session_lease_ms: u64, hard_cap_ms: u64) -> Option<Self> {
        if hard_cap_ms > HARD_CAP_MAX_MS {
            return None;
        }
        Some(Self {
            session_lease_ms,
            hard_cap_ms,
        })
    }

    /// Whether the hard cap is within the one-year limit.
    pub fn hard_cap_within_limit(self) -> bool {
        self.hard_cap_ms <= HARD_CAP_MAX_MS
    }

    /// The absolute hard-cap deadline measured from `now_unix_ms`. Always a
    /// finite instant no more than [`HARD_CAP_MAX_MS`] ahead of `now`.
    pub fn hard_cap_deadline_unix_ms(self, now_unix_ms: u64) -> u64 {
        now_unix_ms.saturating_add(self.hard_cap_ms)
    }

    /// Renew the hard cap from `now_unix_ms` (re-justification), returning the
    /// fresh absolute deadline. Renewal **resets** the cap: the deadline is
    /// recomputed from `now`, it does not extend a prior absolute deadline.
    pub fn renew_hard_cap_deadline_unix_ms(self, now_unix_ms: u64) -> u64 {
        self.hard_cap_deadline_unix_ms(now_unix_ms)
    }
}

/// The serde default for [`DirtyEntryMetadata::intent`]: snapshots serialized
/// before the field existed deserialize as `experimental` (abandoned-experiment
/// cleanup is the safe default; a compensating override must be explicit).
pub(crate) fn default_override_intent() -> OverrideIntent {
    OverrideIntent::Experimental
}

/// The terminal event a scheduled bound represents (ADR-0037 Decision 3,
/// configflux-h6wc). This is the single mapping the active driver uses to
/// SELF-CLASSIFY the due terminal event from a schedule entry's persisted
/// [`AutoResetDeadlineKind`] — no caller-supplied selector is consulted. The
/// session-lease bound is an abandoned-experiment-cleanup expiry; the hard-cap
/// bound is the long (`<= HARD_CAP_MAX_MS`) escalation point.
pub fn terminal_event_for_deadline_kind(kind: AutoResetDeadlineKind) -> TerminalEvent {
    match kind {
        AutoResetDeadlineKind::LeaseExpiry => TerminalEvent::SessionLeaseExpiry,
        AutoResetDeadlineKind::HardCap => TerminalEvent::HardCap,
    }
}

/// The dual absolute deadlines an override lives under, derived from its
/// [`OverrideTimeBounds`] measured from `now_unix_ms` (ADR-0037 Decision 3,
/// configflux-h6wc). This is the wiring point that turns the two relative
/// bounds (`session_lease_ms` + `hard_cap_ms`, the latter already constrained to
/// `<= HARD_CAP_MAX_MS` by [`OverrideTimeBounds::new`]) into the two persisted
/// absolute instants [`DirtyEntryMetadata`] carries. Both are finite — there is
/// no permanent bound — so the hard cap is always a bounded distance ahead of
/// `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideDeadlines {
    /// Absolute session-lease deadline (`now + session_lease_ms`).
    pub session_lease_deadline_unix_ms: u64,
    /// Absolute hard-cap deadline (`now + hard_cap_ms`, `<= HARD_CAP_MAX_MS`
    /// ahead of `now`).
    pub hard_cap_deadline_unix_ms: u64,
}

impl OverrideTimeBounds {
    /// The two absolute deadlines this override's bounds imply, measured from
    /// `now_unix_ms`. Reuses [`OverrideTimeBounds::hard_cap_deadline_unix_ms`]
    /// for the cap so the one-year-bounded, saturating semantics are shared.
    pub fn deadlines_unix_ms(self, now_unix_ms: u64) -> OverrideDeadlines {
        OverrideDeadlines {
            session_lease_deadline_unix_ms: now_unix_ms.saturating_add(self.session_lease_ms),
            hard_cap_deadline_unix_ms: self.hard_cap_deadline_unix_ms(now_unix_ms),
        }
    }
}

/// Pure lifecycle decision: the prescribed outcome for an override terminal
/// event given its governance intent (design doc sec 6 table, ADR-0037
/// Decision 2). No I/O, no mutation — a total function over the four terminal
/// events crossed with the two intents.
///
/// The cardinal invariant (asserted in tests across all events): for
/// `OverrideIntent::Compensating` no outcome is ever silent
/// (`outcome.is_silent() == false`), because a compensating override is a proxy
/// for an unresolved physical condition.
///
/// For the software-upgrade row the compensating branch depends on
/// re-validation against the new model; callers that know the re-validation
/// result should use [`upgrade_reconcile`]. This top-level table function takes
/// the conservative "needs review" stance for a compensating upgrade so that it
/// surfaces rather than silently dropping.
pub fn override_terminal_outcome(event: TerminalEvent, intent: OverrideIntent) -> TerminalOutcome {
    match (event, intent) {
        // Row 1 — session-lease expiry (abandoned-experiment cleanup).
        (TerminalEvent::SessionLeaseExpiry, OverrideIntent::Experimental) => {
            TerminalOutcome::SilentRevert
        }
        (TerminalEvent::SessionLeaseExpiry, OverrideIntent::Compensating) => {
            // Needed, not abandoned — it survives the lease.
            TerminalOutcome::Survive
        }

        // Row 2 — hard cap (<= 1 year, renewable with re-justification).
        (TerminalEvent::HardCap, OverrideIntent::Experimental) => TerminalOutcome::SilentRevert,
        (TerminalEvent::HardCap, OverrideIntent::Compensating) => {
            // Escalate for a human decision; any revert is observable and
            // state-gated, never silent.
            TerminalOutcome::Escalate
        }

        // Row 3 — software upgrade (config version travels with software).
        (TerminalEvent::SoftwareUpgrade, OverrideIntent::Experimental) => {
            // Experimental overrides do not travel across an upgrade.
            upgrade_reconcile(OverrideIntent::Experimental, UpgradeRevalidation::NeedsReview)
        }
        (TerminalEvent::SoftwareUpgrade, OverrideIntent::Compensating) => {
            // Without the re-validation result, take the conservative "surface"
            // stance (never a silent drop). Callers that know the re-validation
            // result use `upgrade_reconcile` directly.
            upgrade_reconcile(OverrideIntent::Compensating, UpgradeRevalidation::NeedsReview)
        }

        // Row 4 — value == new base (upstream convergence).
        (TerminalEvent::ValueEqualsBase, OverrideIntent::Experimental) => {
            // A plain silent absorb + log.
            TerminalOutcome::AbsorbAndLog { keep_linkage: false }
        }
        (TerminalEvent::ValueEqualsBase, OverrideIntent::Compensating) => {
            // A number match is not proof the physical condition resolved —
            // keep the work-order linkage.
            TerminalOutcome::AbsorbAndLog { keep_linkage: true }
        }
    }
}

/// Pure upgrade-reconciliation decision (design doc sec 6 row 3, ADR-0037
/// Decision 6). On a software upgrade an `Experimental` override is always
/// wiped; a `Compensating` override is reconciled against the new model
/// according to the supplied [`UpgradeRevalidation`] result.
pub fn upgrade_reconcile(
    intent: OverrideIntent,
    revalidation: UpgradeRevalidation,
) -> TerminalOutcome {
    match intent {
        // Experimental overrides are wiped on upgrade regardless of how they
        // would re-validate — they do not travel with the software.
        OverrideIntent::Experimental => TerminalOutcome::Wipe,
        OverrideIntent::Compensating => match revalidation {
            // Still valid against the new model — carry it forward.
            UpgradeRevalidation::ValidAgainstNewModel => TerminalOutcome::Carry,
            // Needs human review — surface, do not silent-drop.
            UpgradeRevalidation::NeedsReview => TerminalOutcome::Surface,
            // The parameter no longer exists — surface, do not silent-drop an
            // obsoleted parameter.
            UpgradeRevalidation::ParameterObsoleted => TerminalOutcome::Surface,
            // Value now equals the new base — absorb + log, keeping the
            // work-order linkage.
            UpgradeRevalidation::ConvergedToBase => {
                TerminalOutcome::AbsorbAndLog { keep_linkage: true }
            }
        },
    }
}

#[cfg(test)]
mod override_intent_tests {
    use super::*;

    /// All four terminal events, used by the cross-cutting invariant test.
    const ALL_TERMINAL_EVENTS: [TerminalEvent; 4] = [
        TerminalEvent::SessionLeaseExpiry,
        TerminalEvent::HardCap,
        TerminalEvent::SoftwareUpgrade,
        TerminalEvent::ValueEqualsBase,
    ];

    /// All upgrade re-validation results, used by the upgrade-branch coverage.
    const ALL_UPGRADE_REVALIDATIONS: [UpgradeRevalidation; 4] = [
        UpgradeRevalidation::ValidAgainstNewModel,
        UpgradeRevalidation::NeedsReview,
        UpgradeRevalidation::ParameterObsoleted,
        UpgradeRevalidation::ConvergedToBase,
    ];

    // --- sec 6 table, row 1: session-lease expiry ----------------------------

    #[test]
    fn terminal_lease_expiry_experimental_is_silent_revert() {
        let outcome = override_terminal_outcome(
            TerminalEvent::SessionLeaseExpiry,
            OverrideIntent::Experimental,
        );
        assert_eq!(outcome, TerminalOutcome::SilentRevert);
        assert!(
            outcome.is_silent(),
            "an abandoned experiment is cleaned up silently on lease expiry"
        );
    }

    #[test]
    fn terminal_lease_expiry_compensating_survives() {
        let outcome = override_terminal_outcome(
            TerminalEvent::SessionLeaseExpiry,
            OverrideIntent::Compensating,
        );
        assert_eq!(
            outcome,
            TerminalOutcome::Survive,
            "a compensating override survives lease expiry — it is needed, not abandoned"
        );
        assert!(!outcome.is_silent(), "survival is not a silent termination");
    }

    // --- sec 6 table, row 2: hard cap ---------------------------------------

    #[test]
    fn terminal_hard_cap_experimental_is_silent_revert() {
        let outcome =
            override_terminal_outcome(TerminalEvent::HardCap, OverrideIntent::Experimental);
        assert_eq!(outcome, TerminalOutcome::SilentRevert);
        assert!(outcome.is_silent());
    }

    #[test]
    fn terminal_hard_cap_compensating_escalates_never_silent() {
        let outcome =
            override_terminal_outcome(TerminalEvent::HardCap, OverrideIntent::Compensating);
        assert_eq!(
            outcome,
            TerminalOutcome::Escalate,
            "a compensating hard cap escalates for a human decision"
        );
        assert!(
            !outcome.is_silent(),
            "any compensating hard-cap revert is observable and state-gated, never silent"
        );
    }

    // --- sec 6 table, row 3: software upgrade -------------------------------

    #[test]
    fn terminal_upgrade_experimental_wipes() {
        let outcome =
            override_terminal_outcome(TerminalEvent::SoftwareUpgrade, OverrideIntent::Experimental);
        assert_eq!(
            outcome,
            TerminalOutcome::Wipe,
            "experimental overrides are wiped on software upgrade"
        );
        assert!(outcome.is_silent());
    }

    #[test]
    fn terminal_upgrade_compensating_surfaces_via_table_function() {
        // The top-level table function takes the conservative "surface" stance
        // for a compensating upgrade (it does not know the re-validation result).
        let outcome =
            override_terminal_outcome(TerminalEvent::SoftwareUpgrade, OverrideIntent::Compensating);
        assert_eq!(outcome, TerminalOutcome::Surface);
        assert!(
            !outcome.is_silent(),
            "a compensating override is never silently dropped on upgrade"
        );
    }

    #[test]
    fn upgrade_reconcile_experimental_always_wipes() {
        for revalidation in ALL_UPGRADE_REVALIDATIONS {
            assert_eq!(
                upgrade_reconcile(OverrideIntent::Experimental, revalidation),
                TerminalOutcome::Wipe,
                "experimental overrides are wiped on upgrade regardless of re-validation",
            );
        }
    }

    #[test]
    fn upgrade_reconcile_compensating_carry_or_surface() {
        // Still valid against the new model -> carry.
        assert_eq!(
            upgrade_reconcile(
                OverrideIntent::Compensating,
                UpgradeRevalidation::ValidAgainstNewModel
            ),
            TerminalOutcome::Carry,
        );
        // Needs review -> surface (never silent-drop).
        assert_eq!(
            upgrade_reconcile(OverrideIntent::Compensating, UpgradeRevalidation::NeedsReview),
            TerminalOutcome::Surface,
        );
        // Obsoleted parameter -> surface, do NOT silent-drop.
        assert_eq!(
            upgrade_reconcile(
                OverrideIntent::Compensating,
                UpgradeRevalidation::ParameterObsoleted
            ),
            TerminalOutcome::Surface,
        );
        // Value now equals base -> absorb + log, keeping the work-order linkage.
        assert_eq!(
            upgrade_reconcile(
                OverrideIntent::Compensating,
                UpgradeRevalidation::ConvergedToBase
            ),
            TerminalOutcome::AbsorbAndLog { keep_linkage: true },
        );
    }

    // --- sec 6 table, row 4: value == new base (upstream convergence) -------

    #[test]
    fn terminal_value_equals_base_experimental_absorbs_silently() {
        let outcome =
            override_terminal_outcome(TerminalEvent::ValueEqualsBase, OverrideIntent::Experimental);
        assert_eq!(
            outcome,
            TerminalOutcome::AbsorbAndLog { keep_linkage: false },
            "experimental convergence is a plain silent absorb + log"
        );
        assert!(outcome.is_silent());
    }

    #[test]
    fn terminal_value_equals_base_compensating_absorbs_keeps_linkage() {
        let outcome =
            override_terminal_outcome(TerminalEvent::ValueEqualsBase, OverrideIntent::Compensating);
        assert_eq!(
            outcome,
            TerminalOutcome::AbsorbAndLog { keep_linkage: true },
            "a number match is not proof the physical condition resolved — keep the linkage"
        );
        assert!(
            !outcome.is_silent(),
            "keeping the work-order linkage means the absorb is not a silent termination"
        );
    }

    // --- cardinal cross-cutting invariant -----------------------------------

    #[test]
    fn compensating_is_never_silent_across_all_terminal_events() {
        // ADR-0037 Decision 2: a compensating override may NEVER end silently.
        // This must hold for every terminal event, not just per-cell.
        for event in ALL_TERMINAL_EVENTS {
            let outcome = override_terminal_outcome(event, OverrideIntent::Compensating);
            assert!(
                !outcome.is_silent(),
                "compensating terminal outcome for {event:?} must never be silent (got {outcome:?})"
            );
        }
        // And across every upgrade re-validation branch as well.
        for revalidation in ALL_UPGRADE_REVALIDATIONS {
            let outcome = upgrade_reconcile(OverrideIntent::Compensating, revalidation);
            assert!(
                !outcome.is_silent(),
                "compensating upgrade outcome for {revalidation:?} must never be silent (got {outcome:?})"
            );
        }
    }

    // --- two time bounds: session lease + hard cap (<= 1 year, renewable) ---

    #[test]
    fn hard_cap_rejects_bound_exceeding_one_year() {
        // A hard cap within one year is accepted...
        let ok = OverrideTimeBounds::new(60_000, HARD_CAP_MAX_MS);
        assert!(ok.is_some(), "a hard cap of exactly one year is accepted");
        assert!(ok.unwrap().hard_cap_within_limit());

        // ...but a hard cap past one year is rejected (no unbounded cap).
        let too_long = OverrideTimeBounds::new(60_000, HARD_CAP_MAX_MS + 1);
        assert!(
            too_long.is_none(),
            "a hard cap exceeding one year must be rejected"
        );
    }

    #[test]
    fn hard_cap_renewal_resets_the_cap() {
        let bounds =
            OverrideTimeBounds::new(60_000, 30 * 24 * 60 * 60 * 1000).expect("valid bounds");
        let now = 1_700_000_000_000;
        let first_deadline = bounds.hard_cap_deadline_unix_ms(now);
        assert_eq!(first_deadline, now + bounds.hard_cap_ms);

        // Renewal at a later instant resets (recomputes from the new now), it
        // does not extend the prior absolute deadline.
        let later = now + 10 * 24 * 60 * 60 * 1000;
        let renewed = bounds.renew_hard_cap_deadline_unix_ms(later);
        assert_eq!(
            renewed,
            later + bounds.hard_cap_ms,
            "renewal resets the cap deadline relative to the renewal instant"
        );
        assert!(
            renewed > first_deadline,
            "renewing later moves the deadline forward"
        );
    }

    #[test]
    fn no_permanent_override_mode() {
        // There is no infinite / permanent variant: the hard cap is a finite u64
        // bounded by HARD_CAP_MAX_MS, so the deadline is always a finite instant
        // a bounded distance ahead of now. Permanence belongs in the model.
        let bounds = OverrideTimeBounds::new(60_000, HARD_CAP_MAX_MS).expect("valid bounds");
        let now = 1_700_000_000_000;
        let deadline = bounds.hard_cap_deadline_unix_ms(now);
        assert!(
            deadline <= now.saturating_add(HARD_CAP_MAX_MS),
            "the hard-cap deadline can never exceed one year ahead of now"
        );
        assert!(
            deadline > now,
            "a hard cap always yields a finite future deadline (never permanent)"
        );
    }

    // --- serde: default intent + round-trip stability -----------------------

    #[test]
    fn dirty_metadata_defaults_intent_to_experimental_on_old_snapshot() {
        // A snapshot serialized before `intent` existed has no `intent` key. It
        // must deserialize with the safe default (experimental), not fail.
        let legacy = r#"{
            "actor": "runtime_api.set_parameter",
            "dirty_since_unix_ms": 1700000000000,
            "generation": 1
        }"#;
        let decoded: DirtyEntryMetadata =
            serde_json::from_str(legacy).expect("pre-intent snapshot must still deserialize");
        assert_eq!(
            decoded.intent,
            OverrideIntent::Experimental,
            "missing intent defaults to experimental (abandoned-experiment cleanup is safe)"
        );
    }

    #[test]
    fn dirty_metadata_intent_round_trip_is_byte_stable() {
        let metadata = DirtyEntryMetadata {
            actor: "tech-42".to_string(),
            reason: Some("threshold loosened until part swap".to_string()),
            dirty_since_unix_ms: 1_700_000_000_000,
            reset_deadline_unix_ms: Some(1_700_000_060_000),
            hard_cap_deadline_unix_ms: None,
            generation: 3,
            intent: OverrideIntent::Compensating,
        };

        let first = serde_json::to_vec(&metadata).expect("serialize metadata");
        let decoded: DirtyEntryMetadata =
            serde_json::from_slice(&first).expect("deserialize metadata");
        let second = serde_json::to_vec(&decoded).expect("re-serialize metadata");

        assert_eq!(
            first, second,
            "serialize -> deserialize -> serialize must be byte-stable"
        );
        assert_eq!(metadata, decoded, "round-trip must preserve the metadata");
        assert_eq!(
            decoded.intent,
            OverrideIntent::Compensating,
            "a compensating intent must survive the round-trip"
        );
    }
}
