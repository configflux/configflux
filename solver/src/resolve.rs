// SPDX-License-Identifier: BUSL-1.1
//
// `Session::resolve()` — materialized resolved configuration + a
// deterministic resolve hash (configflux-i0ne). Split out of
// `session.rs` as a sibling `impl Session<B>` block so neither file
// exceeds the repository line cap; the resolve walk is cohesive enough
// to stand alone.
//
// **Approach: adapter (ADR-0017 §3).** ADR-0017 §3 designates
// `compiler::loader_api::resolve_from_selection` → `Session::resolve()`
// as the M5 supersede mapping but leaves this output's shape to
// implementation. The legacy `loader_api::ResolveResult` builds four
// payload fields from the *compiler's* `schema::Config`
// (`resolved_output`, `resolve_hash`, `resolved_component_dependencies`,
// `resolved_artifacts`) via `resolver::resolve_scoped`. The solver owns
// **only the boolean selection model** — the `{facet}.{value}` BDD
// variables from `ccm.symbols.json` (ADR-0005 §3). It has no component
// dependency graph and no artifact catalog; neither concept exists
// anywhere in `solver/`. So approach (1) — reproducing the legacy fields
// directly — is infeasible from solver data, and `Session::resolve()`
// instead materializes the solver's *authoritative* contribution to the
// contract: `resolved_output` (the resolved configuration, as a canonical
// `facet -> selected-option` map) and a deterministic `resolve_hash`. The
// dual-path wiring task (configflux-g3f.2) supplies the
// component/artifact fields from the compiler API alongside this output.
// That split is what makes the future byte-identical parity test
// achievable: the solver owns the selection + hash, the compiler owns the
// catalog.

use std::collections::{BTreeMap, BTreeSet};

use crate::backend::SolverBackend;
use crate::session::{Error, Session};

/// Materialized resolved configuration per ADR-0003 §4, produced by
/// [`Session::resolve`]. See the module header for the adapter rationale
/// (ADR-0017 §3) — why this is a `facet -> option` map + hash rather than
/// the legacy component/artifact catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveResult {
    /// Whether the current state is satisfiable. `false` iff any
    /// partition's `current` formula reduced to ⊥. When `false`,
    /// `resolved_output` is empty but `resolve_hash` is still computed
    /// over the empty output so it stays deterministic.
    pub satisfiable: bool,
    /// Canonical resolved configuration: `facet -> selected-option`,
    /// facet-sorted (`BTreeMap` is the canonical form). Each option is the
    /// first still-satisfiable value in symbol-table order — the same
    /// surviving-option set `valid_options` returns, so resolve never
    /// drifts from it. Empty for the empty-Ccm path and an unsat model.
    pub resolved_output: BTreeMap<String, String>,
    /// Deterministic content hash of this resolution: same Ccm + same
    /// selection ⇒ identical hash; a different selection ⇒ different output
    /// ⇒ different hash. Pre-image recipe is on [`Session::resolve`].
    pub resolve_hash: [u8; 32],
}

impl<B: SolverBackend> Session<B> {
    /// Materialize the current resolved configuration and a deterministic
    /// resolve hash. Pure query (configflux-i0ne). Adapter approach per
    /// ADR-0017 §3 — see this module's header and [`ResolveResult`].
    ///
    /// Walk: (1) unsat iff any partition's `current` is ⊥ (an unsat model
    /// resolves to an empty output but still hashes deterministically);
    /// (2) enumerate distinct facets in symbol-table order (a facet is the
    /// prefix before the last `.` of a `{facet}.{value}` symbol per
    /// ADR-0005 §3); (3) each facet's option is the first still-satisfiable
    /// value `valid_options(facet)` returns (cross-partition-intersected,
    /// symbol-table order — reusing it means resolve never drifts from
    /// `valid_options`; a facet with no surviving option is omitted);
    /// (4) collect into a facet-sorted `BTreeMap` and hash via
    /// [`Self::compute_resolve_hash`].
    ///
    /// Deterministic-hash pre-image (v1), mirroring the legacy
    /// `compute_resolve_hash` (model identity + canonical resolved output):
    ///   `b"configflux.resolve.v1\n" || ccm_hash || bound_model_hash
    ///    || serde_json(resolved_output: BTreeMap<facet, option>)`.
    /// `BTreeMap` serializes key-sorted, so the bytes are stable for a
    /// given assignment regardless of insertion order.
    pub fn resolve(&self) -> Result<ResolveResult, Error> {
        // Empty-Ccm path: no symbols. Trivially satisfiable, empty output
        // (preserves round-trip-empty); hash still computed for determinism.
        if self.ccm.symbols().is_none() && self.ccm.multi_part().is_none() {
            let resolved_output = BTreeMap::new();
            let resolve_hash = self.compute_resolve_hash(&resolved_output);
            return Ok(ResolveResult {
                satisfiable: true,
                resolved_output,
                resolve_hash,
            });
        }

        // Unsat iff any partition's current formula is ⊥.
        let satisfiable = !self.parts.iter().any(|p| p.backend.is_false(p.current));

        let mut resolved_output: BTreeMap<String, String> = BTreeMap::new();
        if satisfiable {
            // Distinct facets in symbol-table order; `seen` keeps the first
            // appearance so enumeration is deterministic.
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for part in &self.parts {
                for sym in &part.variable_order {
                    let Some((facet, _value)) = sym.rsplit_once('.') else {
                        continue;
                    };
                    if !seen.insert(facet.to_string()) {
                        continue;
                    }
                    if let Some(choice) = self.valid_options(facet)?.options.into_iter().next() {
                        resolved_output.insert(facet.to_string(), choice);
                    }
                }
            }
        }

        let resolve_hash = self.compute_resolve_hash(&resolved_output);
        Ok(ResolveResult {
            satisfiable,
            resolved_output,
            resolve_hash,
        })
    }

    /// Deterministic SHA-256 of a resolved configuration over the pre-image
    /// documented on [`Self::resolve`]. Pure: depends only on the bound
    /// model identity and the canonical (`BTreeMap`-sorted) resolved
    /// output, so it is stable byte-for-byte across runs.
    fn compute_resolve_hash(&self, resolved_output: &BTreeMap<String, String>) -> [u8; 32] {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"configflux.resolve.v1\n");
        hasher.update(self.ccm.ccm_hash());
        hasher.update(self.ccm.bound_model_hash());
        // `serde_json` over a String-keyed BTreeMap is key-ordered and
        // cannot fail; fall back to `{}` to keep the recipe total.
        let canon = serde_json::to_vec(resolved_output).unwrap_or_else(|_| b"{}".to_vec());
        hasher.update(&canon);
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::NullBackend;
    use std::path::Path;

    fn empty_session() -> Session<NullBackend> {
        let ccm = Session::<NullBackend>::load_ccm(Path::new("unused")).expect("empty ccm");
        Session::<NullBackend>::new(ccm).expect("empty session")
    }

    #[test]
    fn empty_ccm_resolves_satisfiable_with_empty_output() {
        let s = empty_session();
        let r = s.resolve().expect("resolve on empty session is Ok");
        assert!(r.satisfiable, "the empty model is trivially satisfiable");
        assert!(r.resolved_output.is_empty(), "no facets to resolve");
    }

    #[test]
    fn empty_ccm_resolve_hash_is_deterministic_and_pure() {
        // Two independent empty sessions hash identically, and resolve is a
        // pure repeatable query on a single session.
        let r1 = empty_session().resolve().expect("resolve a");
        let r2 = empty_session().resolve().expect("resolve b");
        assert_eq!(r1.resolve_hash, r2.resolve_hash);

        let s = empty_session();
        assert_eq!(
            s.resolve().expect("first").resolve_hash,
            s.resolve().expect("second").resolve_hash,
        );
    }

    #[test]
    fn resolve_hash_is_sensitive_to_resolved_output() {
        // White-box: the hash recipe must distinguish two different
        // resolved configurations under the same (empty) model identity, so
        // that a different selection cannot collide on the same hash.
        let s = empty_session();
        let mut a: BTreeMap<String, String> = BTreeMap::new();
        a.insert("engine".to_string(), "v6".to_string());
        let mut b: BTreeMap<String, String> = BTreeMap::new();
        b.insert("engine".to_string(), "v8".to_string());
        assert_ne!(
            s.compute_resolve_hash(&a),
            s.compute_resolve_hash(&b),
            "different resolved_output must produce different hashes",
        );
        // Insertion order must not affect the hash (BTreeMap is canonical).
        let mut c: BTreeMap<String, String> = BTreeMap::new();
        c.insert("z".to_string(), "1".to_string());
        c.insert("a".to_string(), "2".to_string());
        let mut d: BTreeMap<String, String> = BTreeMap::new();
        d.insert("a".to_string(), "2".to_string());
        d.insert("z".to_string(), "1".to_string());
        assert_eq!(
            s.compute_resolve_hash(&c),
            s.compute_resolve_hash(&d),
            "hash must be insertion-order-independent",
        );
    }

    #[test]
    fn resolve_result_default_is_unsat_empty_zero() {
        let r = ResolveResult::default();
        assert!(!r.satisfiable);
        assert!(r.resolved_output.is_empty());
        assert_eq!(r.resolve_hash, [0u8; 32]);
    }
}
