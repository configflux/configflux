// SPDX-License-Identifier: BUSL-1.1

use crate::coded_error::{coded, coded_bail, coded_bail_hint, coded_with_hint};
use crate::conditions;
use crate::interface_summary::{InterfaceSummary, MergedSummary};
// configflux-py7w: the diagnostic code a rule reports travels ON the refusal,
// so every rule below that HAS a code names it here rather than leaving the
// product mapper to guess it from the message text.
use crate::product_api::{
    echo_identifier, E_BINDING_INVALID, E_BINDING_NO_ACCEPTABLE_ENTRY, E_CATALOGUE_INVALID,
    E_COMPILE_INPUT_INVALID, E_COMPONENT_DEP_CYCLE, E_FACET_VALUE_UNDECLARED,
    E_INGEST_DUPLICATE_CATALOGUE, E_INGEST_DUPLICATE_FACET, E_REQUIRES_INVALID,
    E_UNKNOWN_COMPONENT_DEP, HINT_CONSTRAINT_FACET_UNDECLARED, HINT_DUPLICATE_BINDING_ID,
    HINT_FACET_BINDING, HINT_SYMBOL_CHARSET,
};
use crate::schema::{Catalogue, CatalogueFieldType, Component, Constraint, Facet, Parameter, Value};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Copy, Clone, PartialEq)]
enum VisitState {
    Visiting,
    Done,
}

/// How much of the model a validator can see (ADR-0058 §D3).
///
/// Every rule below that REJECTS a reference resolving to nothing takes this,
/// because whether that is an error depends entirely on whether the whole model
/// is present. [`Scope::Complete`] reproduces today's behaviour exactly and is
/// what the wrappers keeping the original names pass, so the compile path and
/// the linker are unchanged by this parameter existing.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Every declaration the model has. A reference that resolves to nothing is
    /// an authoring error — the compile path, and the linker's stage 1.
    Complete,
    /// ONE unit plus the interface objects it was compiled against. A reference
    /// that resolves to nothing is a LINK obligation, recorded in the object
    /// header's `imports` and checked when the objects are linked, so the rule
    /// that would reject it is skipped. A reference that DOES resolve is checked
    /// exactly as it is at link time — the check set is narrower, never weaker.
    UnitLocal,
}

impl Scope {
    /// Is an unresolved reference deferred to link time rather than rejected?
    fn defers_unresolved(self) -> bool {
        matches!(self, Scope::UnitLocal)
    }
}

/// Maximum inheritance / dependency chain depth accepted by the DFS guards
/// below. Cycles are rejected by the Visiting/Done coloring, but a deeply
/// nested ACYCLIC chain still recurses once per hop — without a ceiling, a
/// crafted config with tens of thousands of chained nodes overflows the stack
/// and ABORTS the process instead of failing closed with a diagnostic
/// (configflux-xowl.5). Real models sit well under depth 100; 1_000 gives an
/// order of magnitude of headroom while keeping worst-case recursion inside
/// even a 2 MiB test-thread stack in unoptimized builds (frames are largest at
/// opt-level 0, so the ceiling must be sized for debug, not release).
///
/// NO SIZE BOUND CONSTRAINS THE INPUT THAT REACHES HERE. An earlier revision of
/// this comment claimed such a chain "fits inside the 8 MiB input bound". There
/// is no 8 MiB bound on a compiler chunk, and no bound of any other size either:
/// `Compiler::add_chunk_auto` takes a `&str` and never measures it. The 8 MiB
/// figure is `REQUEST_SIZE_LIMIT_BYTES`, the runtime and interpreter CLI stdin
/// request cap — a different path, a different input, no relation to this guard
/// (configflux-jz33).
///
/// What DOES bound authored nesting is the two parsers' own recursion limits:
/// `serde_json` refuses past 128 levels and `toml` past 80, both far below any
/// depth that could exhaust a stack. Measured on configflux-jz33: the deepest
/// override chain that parses at all is 62 levels via JSON, 39 via a TOML inline
/// table and 78 via TOML dotted headers. Every one of those is an order of
/// magnitude below 1_000, so for NESTED authored shapes this ceiling is never
/// reached — it is defence in depth against the parse limits moving, not the
/// thing holding the line today. The two cargo features that would remove those
/// limits (`serde_json`'s `unbounded_depth`, `toml`'s `unbounded`) are pinned off
/// by `//validation:parser_depth_limit_pin_test`, which is what makes the inherited
/// property ours (configflux-jz33).
///
/// The ceiling still does real work on the definition `inherits` chain, which no
/// parser limit touches: a chain of definitions is a FLAT map of string pointers
/// (`"d0": {...}, "d1": {"inherits": "d0"}, ...`) whose nesting depth is 3
/// however long the chain gets. configflux-l4e7 measured ~30_000 such
/// definitions aborting the process through `inspect parameter`, so the inspect
/// path's own walk, `product_api::apply_definition_to_parameter`, carries this
/// ceiling too.
///
/// configflux-dw9i widened the ceiling from those two string-pointer DFSes to
/// the authored `overrides` chain, which recurses through the same shape:
/// `ConditionalBlock::payload` is a `Box<Parameter>` carrying its own
/// `overrides`. `pub(crate)` because `resolver::apply_overrides_recursive`
/// walks that chain on the resolve side and has to stop at the same number —
/// one ceiling per shape, or the two sides disagree about which models link.
///
/// Six further walks over that same `overrides` chain are deliberately left
/// unguarded (configflux-l4e7): `compiler_core::reject_inherits_in_parameter`,
/// `interface_summary::collect_parameter_links`,
/// `loader_api/shared_ops::register_parameter_conditions`,
/// `product_api::collect_override_conditions`,
/// `product_api::collect_candidate_artifact_values` and
/// `product_api::count_overrides`. They walk NESTED authored structure, so the
/// parser bound above covers them at a depth an order of magnitude below this
/// ceiling; guarding them would mean threading a `Result` through the public
/// infallible `interface_summary::summarize` for no reachable case. The pin test
/// is what keeps that reasoning true.
pub(crate) const MAX_CHAIN_DEPTH: usize = 1_000;

/// The one refusal every walk over an authored `overrides` chain raises, so the
/// three sites cannot drift apart (configflux-dw9i).
///
/// The refusal carries NO diagnostic code, which is how it reports the same
/// `E_COMPILE_INPUT_INVALID` the two sibling ceilings above do: a refusal with
/// no code of its own lands on the product mappers' generic bucket
/// (configflux-py7w). One shared constructor rather than three hand-written
/// copies, so the three ceilings cannot drift apart in wording.
///
/// `at` names the entity the chain hangs off — a definition or parameter id, or
/// a path where the caller already has one.
pub(crate) fn ensure_override_depth(depth: usize, at: &str) -> Result<()> {
    if depth > MAX_CHAIN_DEPTH {
        bail!(
            "Parameter override chain exceeds the maximum supported depth {} at '{}'",
            MAX_CHAIN_DEPTH,
            at
        );
    }
    Ok(())
}

// Inheritance-cycle detection is RETAINED in Rust (ADR-0027 Decision 6
// carve-out, configflux-73fr). ConfigFlux carries `inherits` as a string
// pointer resolved later, so a cyclic chain (`a -> b -> a`) is not necessarily
// a CUE structural cycle. The carve-out's deletion condition is now resolved to
// OUTCOME B and the guard is PERMANENT: the missing evidence was supplied by the
// fixture in `compiler/cue/validate_fixtures.sh` (the "cyclic inherits a->b->a is
// structurally valid" case, configflux-0zql), which measured with the pinned cue
// that `cue vet -d '#Config'` ACCEPTS the cycle (exit 0) and the whole-pack
// `#ResolvePack` resolution path ALSO accepts it (exit 0) — CUE does NOT reject
// an inheritance cycle, because `#ResolveParam` dereferences `inherits` by a
// single string-pointer hop, never structurally. So deterministic rejection is
// this DFS guard's job; `lib_tests::test_link_and_verify_definition_inheritance_cycle`
// proves it bails on `a -> b -> a`. The *target-existence* half
// (`validate_param_inherits` / `collect_param_inherits`) was deleted: unknown-
// definition targets are now owned by the CUE authoring layer, and
// `detect_definition_cycle` itself still rejects an unknown parent on the
// definition `inherits` chain.
pub(crate) fn validate_definition_inheritance(
    definitions: &HashMap<String, Parameter>,
) -> Result<()> {
    validate_definition_inheritance_scoped(definitions, Scope::Complete)
}

/// As [`validate_definition_inheritance`], for a caller that may hold only one
/// unit's definitions (ADR-0058 §D3). Under [`Scope::UnitLocal`] a parent this
/// unit does not declare ends the walk instead of failing it: the target is
/// recorded as a definition import and resolved when the objects are linked.
/// Cycles WITHIN the unit are still rejected — they are visible here, and no
/// later stage sees them more clearly.
pub(crate) fn validate_definition_inheritance_scoped(
    definitions: &HashMap<String, Parameter>,
    scope: Scope,
) -> Result<()> {
    let mut states: HashMap<String, VisitState> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    // Sorted roots keep the traversal — and therefore which diagnostic
    // surfaces first — deterministic across runs (HashMap order is not).
    let mut roots: Vec<&String> = definitions.keys().collect();
    roots.sort();
    for name in roots {
        detect_definition_cycle(name, definitions, &mut states, &mut stack, scope)?;
    }

    Ok(())
}

pub(crate) fn validate_component_dependencies(
    components: &HashMap<String, Component>,
) -> Result<()> {
    validate_component_dependencies_scoped(components, Scope::Complete)
}

/// As [`validate_component_dependencies`], for a caller that may hold only one
/// unit's components (ADR-0058 §D3). Under [`Scope::UnitLocal`] a `depends_on`
/// target this unit does not declare is neither rejected nor condition-checked:
/// it is recorded as a component import, and the linker resolves it against the
/// exports of the object that provides it. Self-dependency, condition
/// compatibility between two components of THIS unit, and cycles within it are
/// all still enforced.
pub(crate) fn validate_component_dependencies_scoped(
    components: &HashMap<String, Component>,
    scope: Scope,
) -> Result<()> {
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for (name, comp) in components {
        let mut seen = HashSet::new();
        let mut deps = Vec::new();
        for dep in &comp.depends_on {
            if seen.insert(dep.as_str()) {
                deps.push(dep.clone());
            }
        }
        edges.insert(name.clone(), deps);
    }

    for (name, deps) in &edges {
        for dep in deps {
            if name == dep {
                coded_bail!(E_COMPONENT_DEP_CYCLE, "Component '{}' depends_on itself", name);
            }
            if !components.contains_key(dep) && !scope.defers_unresolved() {
                coded_bail!(
                    E_UNKNOWN_COMPONENT_DEP,
                    "Component '{}' depends_on unknown component '{}'",
                    name,
                    dep
                );
            }
        }
    }

    // Component-parameter `inherits` target-existence validation was deleted in
    // B-5 (ADR-0027 Decision 5): unknown-definition targets are owned by the CUE
    // authoring layer, and any that slip through are still rejected at resolve
    // time (`resolver::resolve_parameter`). Definition-chain target existence
    // and cycles remain guarded by `detect_definition_cycle`.

    for (name, deps) in &edges {
        let comp = components
            .get(name)
            .with_context(|| format!("Missing component '{}'", name))?;
        for dep in deps {
            // A target outside this unit has no condition here to compare
            // against; the implication check runs at link time, where both
            // components' headers are present.
            let Some(dep_comp) = components.get(dep) else {
                if scope.defers_unresolved() {
                    continue;
                }
                bail!("Missing component '{}'", dep);
            };
            let implies = conditions::condition_implies_typed(
                comp.condition.as_deref(),
                dep_comp.condition.as_deref(),
            )
            .with_context(|| {
                format!(
                    "Failed condition implication check for '{}' -> '{}'",
                    name, dep
                )
            })?;
            if !implies {
                bail!(
                    "Condition incompatibility: '{}' ({}) depends_on '{}' ({})",
                    name,
                    comp.condition.as_deref().unwrap_or("<none>"),
                    dep,
                    dep_comp.condition.as_deref().unwrap_or("<none>")
                );
            }
        }
    }

    let mut states: HashMap<String, VisitState> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    // Sorted roots: deterministic traversal order, see
    // validate_definition_inheritance.
    let mut roots: Vec<&String> = components.keys().collect();
    roots.sort();
    for name in &roots {
        detect_cycle(name, &edges, &mut states, &mut stack)?;
    }

    // Diamond dependencies (a component reached via multiple paths from one
    // root) are permitted: the component graph may be any DAG. The former
    // per-root diamond check was retired by ADR-0048 — it was conservative
    // policy, not a correctness requirement. Acyclicity is still enforced by
    // `detect_cycle` above, and the genuine hazard (a component enabled where a
    // shared dependency is disabled) is caught by the per-edge
    // condition-implication check earlier in this function.

    Ok(())
}


/// Validate the pack's facet declarations over the fully merged model
/// (ADR-0047 §§1,3). Two layers:
///
///   1. Per-facet shape invariants, re-validated in Rust even though CUE also
///      checks the shape (ADR-0021 "CUE authors, Rust re-validates" — the Rust
///      pass is defense-in-depth, not a second authoring path): `values` is
///      non-empty, `values` has no duplicates, and `default` (when present) is
///      one of `values`.
///   2. The closed-domain condition check: a *closed* facet's declared
///      `values` are its exhaustive domain, so an equality predicate on that
///      facet that names a value outside the domain is an authoring error
///      (`E_FACET_VALUE_UNDECLARED`). An *open* facet extends its effective
///      domain with condition-referenced values (no error), and a facet with
///      no declaration at all is not consulted here — its domain stays the
///      condition-inferred set, exactly as before this feature (incremental
///      adoption, §3).
///
/// Only `==` predicates are checked, matching the domain-inference contract:
/// `register_facet_domains` widens a facet's domain on `Eq` atoms only
/// (`selection_eval::for_each_eq_predicate`), so an `Eq` literal is precisely
/// what "the facet's domain must contain this value" means. Conditions that do
/// not parse widen no domain today (mirroring `collect_ccm_clauses`), so they
/// cannot violate a closed domain and are skipped.
// ============================================================================
// Authored symbol charset (ADR-0063)
// ============================================================================

/// Does this authored id match the CUE `#snakeId` regex
/// `^[a-z]([a-z0-9]|_[a-z0-9])*_?$` (`compiler/cue/schema.cue:45`) EXACTLY?
///
/// ADR-0063 D1. `#snakeId` constrains authored ids in the CUE authoring
/// front-end, and `compile --source` never evaluates CUE — so for the four
/// classes the compiler interpolates into a synthesized condition clause
/// (facet keys, binding ids, catalogue ids, catalogue entry ids) the rule is
/// re-stated here. ADR-0027 Decision 4 deleted the general Rust mirror on the
/// principle that CUE owns authored shape; ADR-0063 narrows that decision for
/// exactly those four, because on the JSON-direct path the alternative to
/// duplicating one rule is having no rule at all, and what configflux-mrm6
/// MEASURED there was a silently wrong artifact rather than a poor message.
///
/// A HAND-WRITTEN character walk, not a regex: this crate takes no regex
/// dependency, and the accept/reject table the walk must reproduce — the `__`
/// rejection Decision 4 called out, and the single trailing `_` it permits —
/// is pinned in `link_verify_symbol_charset_tests.rs`. That table is what
/// catches the two spellings of this rule drifting apart.
///
/// Bytes rather than chars on purpose: every accepted byte is ASCII, so any
/// byte of a multi-byte UTF-8 sequence is >= 0x80 and falls into the reject
/// arm without a separate non-ASCII test.
pub(crate) fn is_snake_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    let mut at = 1;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            at += 1;
        } else if byte == b'_' {
            // `_?$`: a single TRAILING underscore ends the id. Anywhere else
            // the underscore must be glued to a following alnum, which is how
            // the regex excludes `__` structurally — RE2 has no lookahead, and
            // neither does this walk.
            if at + 1 == bytes.len() {
                return true;
            }
            let next = bytes[at + 1];
            if !(next.is_ascii_lowercase() || next.is_ascii_digit()) {
                return false;
            }
            at += 2;
        } else {
            return false;
        }
    }
    true
}

/// Is this authored facet value a symbol token — non-empty, ASCII, and every
/// character in `[A-Za-z0-9_.-]`?
///
/// ADR-0063 D2, and deliberately WIDER than [`is_snake_id`]: a value is not an
/// identifier, and `eu-west-1` or `1.5` are shapes authors legitimately
/// declare. It is the same file-safe token set `cfx` enforces for an
/// environment name (`cfx::manifest::is_valid_environment_name`, ADR-0059 D1),
/// so an author learns one token rule for the product rather than two.
///
/// An ALLOWLIST rather than a "no quotes" blacklist. The condition grammar's
/// quoted literal has no escape syntax at all (`conditions::parse_quoted_literal`
/// scans to the next matching quote byte and stops), so there is nothing to
/// escape into and the rule has to be at ingest; and a blacklist has to
/// anticipate every character the grammar gives meaning to now and later.
/// configflux-mrm6 case 3 is what a blacklist that guessed wrong looks like —
/// a value holding BOTH quote characters, the exact class the emitter's old
/// hint text claimed was refused, which was not refused.
pub(crate) fn is_symbol_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// ADR-0063 D1 over one facet's key, then D2 over each of its declared values.
///
/// The ONE home for both messages (ADR-0063 Amendment 1). [`validate_facets`]
/// calls it where its own D1 check stood, and the loader calls it over the maps
/// its chunk walk builds — so a package refused at load and a model refused at
/// ingest say the same thing about the same symbol, rather than two spellings
/// of the rule drifting apart the way `verify_ir_dir` once drifted from the
/// compile path (configflux-2yiq).
///
/// D1 before D2 because the value message interpolates the id, and an id that
/// cannot be written into a clause has to be refused before it is quoted back
/// at anyone (ADR-0063 D4).
pub(crate) fn check_facet_symbols(id: &str, facet: &Facet) -> Result<()> {
    if !is_snake_id(id) {
        coded_bail_hint!(
            E_COMPILE_INPUT_INVALID,
            HINT_SYMBOL_CHARSET,
            "Facet id '{}' is not a snake_case identifier",
            echo_identifier(id)
        );
    }
    for value in &facet.values {
        if !is_symbol_token(value) {
            coded_bail_hint!(
                E_COMPILE_INPUT_INVALID,
                HINT_SYMBOL_CHARSET,
                "Facet '{}' declares the value '{}', which is not a symbol token",
                echo_identifier(id),
                echo_identifier(value)
            );
        }
    }
    Ok(())
}

/// ADR-0063 D1 over one catalogue's id, then over each of its entry ids.
///
/// The ONE home for both messages (ADR-0063 Amendment 1); see
/// [`check_facet_symbols`] for why the two live here rather than inline.
///
/// An entry id is a binding's value domain verbatim (ADR-0057 §D3), so it is
/// held to the ID rule rather than the wider value token set — and it is
/// checked HERE rather than as a facet value, because this is the only place
/// that still knows it is an entry id.
///
/// `entries` is a `BTreeMap`, so the first refused entry is id-ascending and a
/// deterministic function of the catalogue.
pub(crate) fn check_catalogue_symbols(id: &str, catalogue: &Catalogue) -> Result<()> {
    if !is_snake_id(id) {
        coded_bail_hint!(
            E_COMPILE_INPUT_INVALID,
            HINT_SYMBOL_CHARSET,
            "Catalogue id '{}' is not a snake_case identifier",
            echo_identifier(id)
        );
    }
    for entry_id in catalogue.entries.keys() {
        if !is_snake_id(entry_id) {
            coded_bail_hint!(
                E_COMPILE_INPUT_INVALID,
                HINT_SYMBOL_CHARSET,
                "Catalogue '{}' entry id '{}' is not a snake_case identifier",
                echo_identifier(id),
                echo_identifier(entry_id)
            );
        }
    }
    Ok(())
}

/// ADR-0063 D1 over one binding id.
///
/// On the INGEST path a binding reaches [`validate_facets`] as the closed facet
/// it is (ADR-0057 §D3) and is refused there under the facet wording. A loader
/// holds the authored `bindings` map itself, so it can name the class exactly;
/// running this before the projected-facet pass is what keeps the narrower
/// wording rather than "Facet id".
pub(crate) fn check_binding_id(id: &str) -> Result<()> {
    if !is_snake_id(id) {
        coded_bail_hint!(
            E_COMPILE_INPUT_INVALID,
            HINT_SYMBOL_CHARSET,
            "Binding id '{}' is not a snake_case identifier",
            echo_identifier(id)
        );
    }
    Ok(())
}

/// The ADR-0063 rule over whatever facets, catalogues and bindings a caller
/// holds — the CONSUMPTION-side entry point (Amendment 1 Decision 1).
///
/// `loader_api`'s two funnels read a package's chunk files and hand their
/// facets, catalogues and bindings to `lowering::lowered_root_conjuncts`, whose
/// text is re-parsed as a root conjunct. Package hashes are self-consistent and
/// unkeyed, and ADR-0063 kept `PRODUCT_SCHEMA_VERSION` at 5, so a package the
/// rule never saw — a pre-amendment compiler's, or one rewritten with
/// recomputed hashes — is accepted by every integrity check `open_model` makes.
/// This is the pass that stops its symbols reaching the clause synthesizer.
///
/// # Order
///
/// Catalogues, then bindings, then facets. The three id spaces OVERLAP: a
/// catalogue entry id is also a binding's facet value and a binding id is also
/// a facet key (ADR-0057 §D3), so a facet-first walk would answer for both
/// under the wider "facet value" / "facet id" wording. Running the narrower
/// class first is what makes each symbol reported once, as what it is.
///
/// Ids are sorted before the walk because a caller may hand this a `HashMap`,
/// whose iteration order is not a function of the model — and the first
/// surfaced diagnostic must be.
pub(crate) fn validate_symbol_charset<'a>(
    facets: impl IntoIterator<Item = (&'a String, &'a Facet)>,
    catalogues: impl IntoIterator<Item = (&'a String, &'a Catalogue)>,
    bindings: impl IntoIterator<Item = &'a String>,
) -> Result<()> {
    let mut catalogue_entries: Vec<(&String, &Catalogue)> = catalogues.into_iter().collect();
    catalogue_entries.sort_by(|left, right| left.0.cmp(right.0));
    for (id, catalogue) in catalogue_entries {
        check_catalogue_symbols(id, catalogue)?;
    }

    let mut binding_ids: Vec<&String> = bindings.into_iter().collect();
    binding_ids.sort();
    for id in binding_ids {
        check_binding_id(id)?;
    }

    let mut facet_entries: Vec<(&String, &Facet)> = facets.into_iter().collect();
    facet_entries.sort_by(|left, right| left.0.cmp(right.0));
    for (id, facet) in facet_entries {
        check_facet_symbols(id, facet)?;
    }

    Ok(())
}

pub(crate) fn validate_facets(
    facets: &HashMap<String, Facet>,
    components: &HashMap<String, Component>,
    definitions: &HashMap<String, Parameter>,
) -> Result<()> {
    // Sorted ids: the first surfaced diagnostic is deterministic across runs
    // (HashMap iteration order is not), matching the other validators.
    let mut facet_ids: Vec<&String> = facets.keys().collect();
    facet_ids.sort();

    for id in &facet_ids {
        let facet = &facets[*id];
        // ADR-0063 D1 and D2, FIRST and together: every message below
        // interpolates the id or a value, and a symbol the compiler writes into
        // a synthesized clause BARE has to be one that clause can carry before
        // it is quoted back at anyone (D4). The pair lives in
        // `check_facet_symbols` so the loader's consumption-side pass
        // (Amendment 1) refuses the same symbol with the same words rather than
        // acquiring a second spelling of the rule to keep in step by hand —
        // which is the parity configflux-2yiq is the record of losing.
        //
        // A binding arrives here as the closed facet it IS (ADR-0057 §D3),
        // because all three callers hand this function the effective map, so
        // one call covers both id spaces.
        check_facet_symbols(id.as_str(), facet)?;
        if facet.values.is_empty() {
            bail!("Facet '{}' declares an empty value domain", id);
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for value in &facet.values {
            if !seen.insert(value.as_str()) {
                bail!("Facet '{}' declares duplicate value '{}'", id, value);
            }
        }
        if let Some(default) = &facet.default {
            if !facet.values.iter().any(|v| v == default) {
                // configflux-xcrb. A default IS a facet value, read off the
                // declaration rather than off a condition, so this is the
                // closed-domain fault the code's registry line already
                // promises — it simply carried no code and reached the caller
                // as the generic bucket. The code's DEFAULT remedy applies
                // verbatim (`product_api::hint_for`): adding the value,
                // opening the facet, or correcting the reference are the same
                // three fixes here as for a condition naming an undeclared
                // value, so this rule needs no remedy of its own.
                coded_bail!(
                    E_FACET_VALUE_UNDECLARED,
                    "Facet '{}' default '{}' is not one of its declared values [{}]",
                    id,
                    default,
                    facet.values.join(", ")
                );
            }
        }
    }

    // Index the closed facets by id. Open and undeclared facets never gate a
    // condition value here.
    let mut closed: BTreeMap<&str, &Facet> = BTreeMap::new();
    for id in &facet_ids {
        let facet = &facets[*id];
        if !facet.open {
            closed.insert(id.as_str(), facet);
        }
    }
    if closed.is_empty() {
        return Ok(());
    }

    let mut conditions: Vec<String> = Vec::new();
    collect_model_conditions(components, definitions, &mut conditions)?;
    for condition in &conditions {
        let trimmed = condition.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(expr) = conditions::parse_condition_expr(trimmed) else {
            continue;
        };
        let mut violation: Option<(String, String)> = None;
        conditions::for_each_eq_predicate(&expr, |tag, value| {
            if violation.is_some() {
                return;
            }
            if let Some(facet) = closed.get(tag) {
                if !facet.values.iter().any(|v| v == value) {
                    violation = Some((tag.to_string(), value.to_string()));
                }
            }
        });
        if let Some((tag, value)) = violation {
            let facet = closed[tag.as_str()];
            coded_bail!(
                E_FACET_VALUE_UNDECLARED,
                "Condition value '{}' is not in the closed facet '{}' domain [{}]",
                value,
                tag,
                facet.values.join(", ")
            );
        }
    }

    Ok(())
}

/// Validate every declared parameter-to-facet binding (ADR-0064 D2).
///
/// A parameter is a facet's runtime handle ONLY when it declares
/// `facet: <name>`; name coincidence binds nothing. Four rules hold together,
/// because all four are what "this parameter IS this facet's handle" means:
///
///   1. **The facet is declared** — as a `facets` entry or as a binding, which
///      IS a facet (ADR-0057 §D3). All three callers hand this function the
///      EFFECTIVE map, so one rule covers both id spaces exactly as
///      [`validate_facets`] does. Reported as `E_FACET_VALUE_UNDECLARED`, whose
///      registry cause carries this second shape.
///   2. **The effective type is `string`.** A facet value is a symbol token, so
///      a handle of any other type could never carry one. EVERY arm of the
///      override tree is checked rather than the one that happens to be active:
///      which arm applies depends on a selection the compiler has not made yet,
///      and a parameter that is `string` under one condition and `int` under
///      another must be refused at compile time rather than at whichever
///      resolve picks the second.
///   3. **The parameter authors no value of its own**, and no `overrides` entry
///      sets `value` or `facet`. Its value is the facet's (D3); an authored one
///      would be a second source of truth. `overrides` entries that vary OTHER
///      fields (`limits`, `safety`, `access`, `lifecycle`, `unit`, `doc`) stay
///      legal — a bound parameter may still tighten its limits under a
///      condition without giving up the binding.
///   4. **At most one parameter in the whole model binds a given facet.** This
///      is the rule that makes the many-to-one mapping impossible by
///      construction, which is what lets the runtime delete its
///      skip-on-disagreement machinery instead of keeping it as a backstop
///      (ADR-0064 D5). Deliberately stricter than "one per resolved output":
///      two mutually exclusive variant components cannot each bind one facet.
///      Relaxing that later is additive; starting relaxed is not reversible.
///
/// Rules 1-3 are properties of ONE parameter and hold at either scope. Rule 4
/// looks across the whole model, so it is skipped under [`Scope::UnitLocal`]
/// exactly as every other cross-unit rule here is: an object sees one unit, and
/// `link` catches a second binder in another unit when it verifies the merged
/// repository.
///
/// No new `E_*` code: three rules report the ingest-side
/// `E_COMPILE_INPUT_INVALID` and one reports `E_FACET_VALUE_UNDECLARED`, and
/// all four carry [`HINT_FACET_BINDING`], which states the rule set rather than
/// the rule that fired.
pub(crate) fn validate_facet_bindings_scoped(
    components: &HashMap<String, Component>,
    facets: &HashMap<String, Facet>,
    scope: Scope,
) -> Result<()> {
    // Sorted ids: the first surfaced diagnostic is deterministic across runs
    // (HashMap iteration order is not), matching every other validator here.
    let mut component_ids: Vec<&String> = components.keys().collect();
    component_ids.sort();

    // facet id -> the path of the parameter that bound it first. Rule 4's
    // message names both paths, and the walk order above is what makes "first"
    // mean the same thing on every run.
    let mut binders: BTreeMap<&str, String> = BTreeMap::new();

    for component_id in component_ids {
        let component = &components[component_id];
        let mut param_keys: Vec<&String> = component.params.keys().collect();
        param_keys.sort();

        for param_key in param_keys {
            let param = &component.params[param_key];
            let Some(facet) = param.facet.as_deref() else {
                continue;
            };
            // The runtime path spelling (`component.<id>.param.<key>`), not the
            // resolver's `components.<id>.params.<key>`: a binding is read at
            // the path a runtime caller writes to, so naming it that way is
            // what lets the author match the diagnostic to the write.
            let path = format!("component.{}.param.{}", component_id, param_key);

            if !facets.contains_key(facet) {
                coded_bail_hint!(
                    E_FACET_VALUE_UNDECLARED,
                    HINT_FACET_BINDING,
                    "Parameter '{}' binds facet '{}', which the model declares as neither a \
                     facet nor a binding",
                    path,
                    echo_identifier(facet)
                );
            }

            let mut declared_types: Vec<&str> = Vec::new();
            collect_declared_types(param, param_key, 0, &mut declared_types)?;
            match declared_types.iter().find(|t| **t != "string") {
                Some(other) => coded_bail_hint!(
                    E_COMPILE_INPUT_INVALID,
                    HINT_FACET_BINDING,
                    "Parameter '{}' binds facet '{}' but declares type '{}'; a bound \
                     parameter's effective type must be 'string'",
                    path,
                    echo_identifier(facet),
                    echo_identifier(other)
                ),
                None if declared_types.is_empty() => coded_bail_hint!(
                    E_COMPILE_INPUT_INVALID,
                    HINT_FACET_BINDING,
                    "Parameter '{}' binds facet '{}' but declares no type; a bound parameter's \
                     effective type must be 'string'",
                    path,
                    echo_identifier(facet)
                ),
                None => {}
            }

            if param.value.is_some() {
                coded_bail_hint!(
                    E_COMPILE_INPUT_INVALID,
                    HINT_FACET_BINDING,
                    "Parameter '{}' binds facet '{}' and also authors a `value`; a bound \
                     parameter's value is the facet's",
                    path,
                    echo_identifier(facet)
                );
            }
            if let Some(field) = override_sets_value_or_facet(param, param_key, 0)? {
                coded_bail_hint!(
                    E_COMPILE_INPUT_INVALID,
                    HINT_FACET_BINDING,
                    "Parameter '{}' binds facet '{}' and an `overrides` entry sets `{}`; a \
                     binding and the value it carries are properties of the parameter, not of \
                     a variant of it",
                    path,
                    echo_identifier(facet),
                    field
                );
            }

            // Rule 4 only: an object sees one unit, so a second binder in
            // another unit is a LINK obligation rather than an authoring error
            // here (ADR-0058 §D3).
            if scope.defers_unresolved() {
                continue;
            }
            if let Some(first) = binders.get(facet) {
                coded_bail_hint!(
                    E_COMPILE_INPUT_INVALID,
                    HINT_FACET_BINDING,
                    "Facet '{}' is bound by two parameters, '{}' and '{}'; a facet has at most \
                     one handle",
                    echo_identifier(facet),
                    first,
                    path
                );
            }
            binders.insert(facet, path);
        }
    }

    Ok(())
}

/// Every type declared anywhere in a parameter's override tree, in override
/// order then nested-override order.
///
/// `depth` is bounded by [`MAX_CHAIN_DEPTH`] through the same
/// [`ensure_override_depth`] guard [`collect_parameter_conditions`] uses, and
/// for the same reason: a crafted chain would otherwise recurse once per hop
/// and abort the process instead of failing closed.
fn collect_declared_types<'a>(
    parameter: &'a Parameter,
    owner: &str,
    depth: usize,
    out: &mut Vec<&'a str>,
) -> Result<()> {
    ensure_override_depth(depth, owner)?;
    if let Some(declared) = parameter.r#type.as_deref() {
        out.push(declared);
    }
    for block in &parameter.overrides {
        collect_declared_types(block.payload.as_ref(), owner, depth + 1, out)?;
    }
    Ok(())
}

/// The name of the first field an `overrides` entry sets that a bound parameter
/// may not vary — `value` or `facet` — or `None` when every entry varies only
/// fields a binding leaves alone.
fn override_sets_value_or_facet(
    parameter: &Parameter,
    owner: &str,
    depth: usize,
) -> Result<Option<&'static str>> {
    ensure_override_depth(depth, owner)?;
    for block in &parameter.overrides {
        let payload = block.payload.as_ref();
        if payload.value.is_some() {
            return Ok(Some("value"));
        }
        if payload.facet.is_some() {
            return Ok(Some("facet"));
        }
        if let Some(field) = override_sets_value_or_facet(payload, owner, depth + 1)? {
            return Ok(Some(field));
        }
    }
    Ok(None)
}

/// Validate the pack's `constraints` declarations over the fully merged model
/// (ADR-0054 §4). Three rules, checked in constraint-id-ascending order so the
/// first surfaced diagnostic is deterministic:
///
///   1. **Every constraint expression parses.** This is the one place where a
///      constraint and a selector `condition` are treated differently at
///      ingest, and the difference is deliberate: an unparseable *selector*
///      widens no facet domain and is skipped (see `validate_facets`), because
///      the worst case is a filter that includes too much. An unparseable
///      *constraint* is a policy nobody can evaluate — silently dropping it
///      would fail OPEN on the surface whose whole job is to say "no". So it is
///      a hard ingest ERROR (ADR-0054 §1).
///   2. **Every facet it names is DECLARED** (ADR-0047), not merely mentioned
///      by some selector condition (configflux-6j91). A constraint asserts over
///      a domain; it does not create one, and a condition-inferred domain is an
///      artifact of what conditions happened to mention rather than something
///      the author wrote down. The rule is not stylistic — it is what keeps the
///      three surfaces in agreement. ADR-0054 §5.2 synthesizes intra-facet
///      cardinality for DECLARED facets only, so a constraint over an inferred
///      facet gets no at-most-one clauses: a positive equality
///      (`arch == 'x86'`) leaves `root ∧ arch.x86 ∧ arch.arm` satisfiable over
///      independent variables, so `options`/`select` keep offering `arm` while
///      `resolve`, which evaluates the constraint concretely under a total
///      assignment, rejects it. Failing the compile closes that fail-open
///      corner from the validation side; §5.2's refusal to synthesize over an
///      inferred domain closes it from the other.
///
///      Note this rule needs no parseable/unparseable carve-out: unlike
///      `validate_facets`, it never consults conditions at all, so a selector
///      cannot widen a constraint's legal name set whether it parses or not.
///   3. **Every value it names is a member of a closed facet's declared
///      domain** — the existing `E_FACET_VALUE_UNDECLARED` rule, now applied to
///      constraints too. Unlike `validate_facets`, BOTH operators are checked,
///      not just `==`: `validate_facets` checks only `==` because only `==`
///      widens an inferred domain, but ADR-0054 §4 says every value a
///      constraint *names*, and against a closed domain a mistyped
///      `environment != 'prod0'` is not a harmless no-op — it is a tautology
///      that silently voids the policy. Open and undeclared facets are not
///      gated here, exactly as in `validate_facets`.
///
/// Takes no components or definitions on purpose: the legal name set is exactly
/// `facets.keys()`, and a function that cannot see the model's conditions cannot
/// regress into unioning them back in (configflux-6j91).
pub(crate) fn validate_constraints(
    constraints: &HashMap<String, Constraint>,
    facets: &HashMap<String, Facet>,
) -> Result<()> {
    validate_constraints_scoped(constraints, facets, Scope::Complete)
}

/// As [`validate_constraints`], for a caller holding one unit's constraints and
/// the declarations of that unit plus its interfaces (ADR-0058 §D3).
///
/// Rule 1 — every expression parses — is unconditional: an unparseable policy is
/// unparseable wherever it is read, and it is the one rule whose failure mode is
/// fail-open. Rules 2 and 3 differ by scope. A facet the unit and its interfaces
/// both fail to declare is a FACET IMPORT under [`Scope::UnitLocal`], recorded
/// in the header and rejected by the linker if nothing provides it; a facet that
/// IS declared has its values checked here exactly as at link time.
pub(crate) fn validate_constraints_scoped(
    constraints: &HashMap<String, Constraint>,
    facets: &HashMap<String, Facet>,
    scope: Scope,
) -> Result<()> {
    if constraints.is_empty() {
        return Ok(());
    }

    let mut constraint_ids: Vec<&String> = constraints.keys().collect();
    constraint_ids.sort();

    for id in constraint_ids {
        let text = constraints[id].condition.trim();
        let expr = conditions::parse_condition_expr(text).with_context(|| {
            format!(
                "Constraint '{}' expression does not parse: '{}'",
                id, text
            )
        })?;

        // First violation in AST order wins, so the reported diagnostic is a
        // deterministic function of the authored text.
        let mut violation: Option<ConstraintViolation> = None;
        conditions::for_each_condition_operand(&expr, |operand| {
            if violation.is_some() {
                return;
            }
            match operand {
                conditions::ConditionOperand::Symbol(tag, value) => {
                    let Some(facet) = facets.get(tag) else {
                        violation = Some(ConstraintViolation::UndeclaredFacet(tag.to_string()));
                        return;
                    };
                    if !facet.open && !facet.values.iter().any(|v| v == value) {
                        violation = Some(ConstraintViolation::UndeclaredValue(
                            tag.to_string(),
                            value.to_string(),
                        ));
                    }
                }
                // configflux-secb.2 / ADR-0057 §D5: BOTH sides of a
                // facet-to-facet comparison must be declared. There is no
                // value to domain-check — the comparison names none — but the
                // names themselves are subject to the same §5.2 rule, and the
                // expansion cannot run without both declared domains.
                //
                // The left-hand side is an ordinary predicate tag position, so
                // it reuses the existing phrasing. The right-hand side gets its
                // own, because the author's likely mistake there is different:
                // they meant a literal and forgot the quotes.
                conditions::ConditionOperand::ComparisonLeft(tag) => {
                    if !facets.contains_key(tag) {
                        violation = Some(ConstraintViolation::UndeclaredFacet(tag.to_string()));
                    }
                }
                conditions::ConditionOperand::ComparisonRight(tag) => {
                    if !facets.contains_key(tag) {
                        violation = Some(ConstraintViolation::UndeclaredComparand(tag.to_string()));
                    }
                }
            }
        });

        // Under `Scope::UnitLocal` an undeclared name is an unresolved
        // reference, not a violation. The VALUE rule is untouched: it fires only
        // on a facet that IS declared here.
        if scope.defers_unresolved()
            && matches!(
                violation,
                Some(ConstraintViolation::UndeclaredFacet(_))
                    | Some(ConstraintViolation::UndeclaredComparand(_))
            )
        {
            violation = None;
        }

        match violation {
            // Deliberately NOT "unknown facet": the facet a model most often
            // trips this rule with is one the author can see all over their own
            // conditions, and calling it unknown would be false. The offending
            // name and the remedy both have to be in the message, because
            // "declare it" is the only fix that keeps the policy.
            //
            // E_FACET_VALUE_UNDECLARED is shared with the closed-domain rule
            // below — the same code, one authoring step earlier (configflux-6j91)
            // — so this rule carries its own remedy: the message already states
            // the fix verbatim, and the code's default remedy talks about a
            // value's domain rather than the missing declaration.
            // //compiler:constraint_facet_diagnostic_test drives the real compile
            // path, so neither the code nor the remedy can change in silence.
            Some(ConstraintViolation::UndeclaredFacet(tag)) => coded_bail_hint!(
                E_FACET_VALUE_UNDECLARED,
                HINT_CONSTRAINT_FACET_UNDECLARED,
                "Constraint '{}' references facet '{}', which is not declared under `facets`: \
                 declare the facet with its value domain, or remove it from the constraint",
                id,
                tag
            ),
            Some(ConstraintViolation::UndeclaredValue(tag, value)) => {
                let facet = &facets[&tag];
                // The closed-domain half of E_FACET_VALUE_UNDECLARED, reported
                // with that code's own remedy (`product_api::hint_for`).
                coded_bail!(
                    E_FACET_VALUE_UNDECLARED,
                    "Constraint '{}' value '{}' is not in the closed facet '{}' domain [{}]",
                    id,
                    value,
                    tag,
                    facet.values.join(", ")
                );
            }
            // configflux-secb.2 / ADR-0057 §D5. This refusal carries NO code,
            // which is how it reports E_COMPILE_INPUT_INVALID — the code this
            // rule is specified to carry, since an unquoted right-hand side is
            // not a facet-value problem. The remedy names both readings, because
            // the author either meant another facet and misspelled it, or
            // meant a literal and forgot the quotes — and only they know
            // which.
            Some(ConstraintViolation::UndeclaredComparand(tag)) => bail!(
                "constraint '{}': right-hand side '{}' is not a declared facet or binding; \
                 quote it to compare against a literal",
                id,
                tag
            ),
            None => {}
        }
    }

    Ok(())
}

/// The first rule a constraint's operand walk broke. Carried out of the
/// `for_each_condition_operand` closure so the `bail!` happens outside it.
enum ConstraintViolation {
    UndeclaredFacet(String),
    UndeclaredValue(String, String),
    /// The right-hand side of a facet-to-facet comparison names nothing
    /// declared (configflux-secb.2 / ADR-0057 §D5).
    UndeclaredComparand(String),
}

/// Collect every authored `condition` string from the merged model in a stable,
/// id-sorted order: definition override chains first, then each component's own
/// activation condition and its params' override chains. Mirrors
/// `compiler_core::collect_ccm_clauses`'s harvest (that method walks the raw
/// chunks; this one walks the merged repository) so the closed-domain check
/// sees exactly the conditions the selection model will.
///
/// The harvest is deliberately **flat** — no kind tag (configflux-9xxq /
/// ADR-0054 §5.1) — because the producer it mirrors is flat too:
/// `collect_ccm_clauses` gathers definition-override, component-activation and
/// component-parameter-override conditions into ONE `Vec<String>`, then lowers
/// every survivor identically via `synthesize_selector_symbols`. Naming a value
/// outside a closed facet's declared domain is an authoring error either way.
fn collect_model_conditions(
    components: &HashMap<String, Component>,
    definitions: &HashMap<String, Parameter>,
    out: &mut Vec<String>,
) -> Result<()> {
    let mut definition_ids: Vec<&String> = definitions.keys().collect();
    definition_ids.sort();
    for id in definition_ids {
        collect_parameter_conditions(&definitions[id], id, 0, out)?;
    }

    let mut component_ids: Vec<&String> = components.keys().collect();
    component_ids.sort();
    for id in component_ids {
        let component = &components[id];
        if let Some(condition) = &component.condition {
            out.push(condition.clone());
        }
        let mut param_ids: Vec<&String> = component.params.keys().collect();
        param_ids.sort();
        for pid in param_ids {
            collect_parameter_conditions(&component.params[pid], pid, 0, out)?;
        }
    }

    Ok(())
}

/// Recursively collect the `condition` strings from a parameter's override
/// chain, in override order then nested-override order.
///
/// `depth` counts hops down the chain and is bounded by [`MAX_CHAIN_DEPTH`]
/// (configflux-dw9i). `owner` is the definition or parameter id the chain hangs
/// off, not a per-level path: naming the entity is what the two sibling
/// ceilings do, and building a path string per level would make an already
/// deep walk allocate quadratically on the way to its own refusal.
fn collect_parameter_conditions(
    parameter: &Parameter,
    owner: &str,
    depth: usize,
    out: &mut Vec<String>,
) -> Result<()> {
    ensure_override_depth(depth, owner)?;
    for override_block in &parameter.overrides {
        out.push(override_block.condition.clone());
        collect_parameter_conditions(override_block.payload.as_ref(), owner, depth + 1, out)?;
    }
    Ok(())
}

// DFS over the definition `inherits` string-pointer graph. Permanent per the
// ADR-0027 Decision 6 carve-out (OUTCOME B): CUE does not reject an inheritance
// cycle — see the "cyclic inherits a->b->a" fixture in
// compiler/cue/validate_fixtures.sh (configflux-0zql) and the header comment on
// `validate_definition_inheritance` above. This function is the deterministic
// rejection; `a -> b -> a` bails with "Definition inheritance cycle detected".
fn detect_definition_cycle(
    node: &str,
    definitions: &HashMap<String, Parameter>,
    states: &mut HashMap<String, VisitState>,
    stack: &mut Vec<String>,
    scope: Scope,
) -> Result<()> {
    match states.get(node).copied() {
        Some(VisitState::Visiting) => {
            let cycle_start = stack.iter().position(|n| n == node).unwrap_or(0);
            let mut cycle = stack[cycle_start..].to_vec();
            cycle.push(node.to_string());
            bail!(
                "Definition inheritance cycle detected: {}",
                cycle.join(" -> ")
            );
        }
        Some(VisitState::Done) => return Ok(()),
        None => {}
    }

    states.insert(node.to_string(), VisitState::Visiting);
    stack.push(node.to_string());
    if stack.len() > MAX_CHAIN_DEPTH {
        bail!(
            "Definition inheritance chain exceeds the maximum supported depth {} at '{}'",
            MAX_CHAIN_DEPTH,
            node
        );
    }

    if let Some(def) = definitions.get(node) {
        if let Some(parent) = &def.inherits {
            if !definitions.contains_key(parent) {
                // ADR-0058 §D3: at object time the parent may simply live in
                // another unit. Ending the walk is right either way — there is
                // no chain here to follow, and no cycle here to find.
                if !scope.defers_unresolved() {
                    bail!(
                        "Definition '{}' inherits unknown definition '{}'",
                        node,
                        parent
                    );
                }
            } else {
                detect_definition_cycle(parent, definitions, states, stack, scope)?;
            }
        }
    }

    stack.pop();
    states.insert(node.to_string(), VisitState::Done);
    Ok(())
}

fn detect_cycle(
    node: &str,
    edges: &HashMap<String, Vec<String>>,
    states: &mut HashMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> Result<()> {
    match states.get(node).copied() {
        Some(VisitState::Visiting) => {
            let cycle_start = stack.iter().position(|n| n == node).unwrap_or(0);
            let mut cycle = stack[cycle_start..].to_vec();
            cycle.push(node.to_string());
            coded_bail!(
                E_COMPONENT_DEP_CYCLE,
                "Component dependency cycle detected: {}",
                cycle.join(" -> ")
            );
        }
        Some(VisitState::Done) => return Ok(()),
        None => {}
    }

    states.insert(node.to_string(), VisitState::Visiting);
    stack.push(node.to_string());
    if stack.len() > MAX_CHAIN_DEPTH {
        bail!(
            "Component dependency chain exceeds the maximum supported depth {} at '{}'",
            MAX_CHAIN_DEPTH,
            node
        );
    }

    if let Some(deps) = edges.get(node) {
        for dep in deps {
            detect_cycle(dep, edges, states, stack)?;
        }
    }

    stack.pop();
    states.insert(node.to_string(), VisitState::Done);
    Ok(())
}

/// Validate the pack's catalogue declarations (ADR-0057 §D2).
///
/// A catalogue is a TYPED TABLE, and this is where the type earns its keep:
/// every entry must supply exactly the declared fields, each with a value of
/// the declared type. CUE checks the shape (four literal type names, `#Value`
/// entries, `#snakeId` keys) but cannot express "this struct's keys are exactly
/// that struct's keys", so the completeness and type rules are Rust's
/// (ADR-0021 "CUE authors, Rust re-validates").
///
/// This is a BODY check, not a link check: it reads a catalogue's fields and
/// entries, which an [`InterfaceSummary`] deliberately does not carry — an
/// object header would not have them either. That is why it lives here rather
/// than over the [`MergedSummary`]; see [`crate::interface_summary`] for the
/// split.
///
/// Ids are walked ascending (catalogue, then entry, then field) so the first
/// surfaced diagnostic is a deterministic function of the model.
pub(crate) fn validate_catalogues(catalogues: &HashMap<String, Catalogue>) -> Result<()> {
    let mut ids: Vec<&String> = catalogues.keys().collect();
    ids.sort();

    for id in ids {
        let catalogue = &catalogues[id];
        // ADR-0063 D1 over the catalogue id AND over every entry id, before the
        // shape rules and for the same reason the facet twin runs first: the
        // messages below interpolate both. An entry id is a binding's value
        // domain verbatim (ADR-0057 §D3), so it is held to the id rule rather
        // than the wider value token set — and this is the only place that can
        // say so, since by the time `validate_facets` sees it the entry id is
        // just one more facet value. Both messages live in
        // `check_catalogue_symbols` so the loader's consumption-side pass
        // (ADR-0063 Amendment 1) reuses them verbatim.
        check_catalogue_symbols(id, catalogue)?;
        if catalogue.fields.is_empty() {
            coded_bail!(E_CATALOGUE_INVALID, "Catalogue '{}' declares no fields", id);
        }
        if catalogue.entries.is_empty() {
            coded_bail!(E_CATALOGUE_INVALID, "Catalogue '{}' declares no entries", id);
        }

        for (entry_id, entry) in &catalogue.entries {
            for (field_id, field) in &catalogue.fields {
                let Some(value) = entry.get(field_id) else {
                    coded_bail!(
                        E_CATALOGUE_INVALID,
                        "Catalogue '{}' entry '{}' is missing field '{}'",
                        id,
                        entry_id,
                        field_id
                    );
                };
                if !value_matches(value, field.r#type) {
                    coded_bail!(
                        E_CATALOGUE_INVALID,
                        "Catalogue '{}' entry '{}' field '{}' is not of the declared type '{}'",
                        id,
                        entry_id,
                        field_id,
                        field.r#type.as_str()
                    );
                }
                // configflux-2yiq. Checked AFTER the type rule, so a float in a
                // non-float column is still reported as the type mistake it is;
                // only a value the column genuinely accepts can reach here.
                if is_non_finite(value) {
                    coded_bail!(
                        E_CATALOGUE_INVALID,
                        "Catalogue '{}' entry '{}' field '{}' is not a finite number{}",
                        id,
                        entry_id,
                        field_id,
                        NON_FINITE_REMEDY
                    );
                }
            }
            for key in entry.keys() {
                if !catalogue.fields.contains_key(key) {
                    coded_bail!(
                        E_CATALOGUE_INVALID,
                        "Catalogue '{}' entry '{}' declares undeclared field '{}'",
                        id,
                        entry_id,
                        key
                    );
                }
            }
        }
    }

    Ok(())
}

/// Does an authored value satisfy a declared catalogue field type?
///
/// `integer` accepts only an integer; `float` ALSO accepts an integer, because
/// a JSON `1000` for a millimetre dimension is the same number as `1000.0` and
/// serde's untagged `Value` resolves it to `Integer` before it ever reaches
/// here. Nothing else is promoted: a string is never a number, and a boolean is
/// never an integer, because either coercion would let a typo through as data.
fn value_matches(value: &Value, declared: CatalogueFieldType) -> bool {
    match declared {
        CatalogueFieldType::Integer => matches!(value, Value::Integer(_)),
        CatalogueFieldType::Float => matches!(value, Value::Float(_) | Value::Integer(_)),
        CatalogueFieldType::Boolean => matches!(value, Value::Boolean(_)),
        CatalogueFieldType::String => matches!(value, Value::String(_)),
    }
}

/// Why a non-finite float is refused, appended to both diagnostics so the
/// author is told what the rule protects rather than only that they broke it.
/// The remedy is the same in either namespace, so the sentence is shared.
const NON_FINITE_REMEDY: &str = ": NaN and the infinities have no JSON \
     representation, so the canonical bytes every hash preimage is built from \
     would record `null` and three different authored models would become one. \
     Author a finite number, or express the absent case some other way";

/// Is this an authored float that is not a finite number (configflux-2yiq)?
///
/// TOML 1.0 admits `nan`, `+nan`, `-nan`, `inf`, `+inf` and `-inf` as float
/// literals and [`Value::Float`] accepts every one of them. Every hash preimage
/// in this compiler is canonical JSON built with `serde_json::to_value`, and
/// `serde_json::Number::from_f64` returns `None` for exactly these values — so
/// the number is written as `null`. `nan`, `inf` and `-inf` in one column
/// therefore share one `chunk_hash`, one `model_hash` and one `resolve_hash`,
/// and the value is destroyed inside `resolved_output` on the way. That is the
/// silent truncation SECURITY.md "Input Parsing Safety" asks to be an explicit
/// error instead.
///
/// Only the authored TOML form can reach this rule: JSON has no non-finite
/// literal, so `serde_json::from_str` rejects one at parse — which covers the
/// CUE authoring front end too, since CUE reaches this compiler as exported
/// JSON.
fn is_non_finite(value: &Value) -> bool {
    matches!(value, Value::Float(float) if !float.is_finite())
}

/// Reject a non-finite float authored anywhere in the model's PARAMETERS
/// (configflux-2yiq) — the twin of the catalogue-entry rule in
/// [`validate_catalogues`]. See [`is_non_finite`] for why.
///
/// Ids are walked ascending (definitions, then components, then each
/// component's params) so the first surfaced diagnostic is a deterministic
/// function of the model, exactly as in [`validate_catalogues`].
///
/// A parameter's `value` is not its only number: `limits.min`/`limits.max` ride
/// into `resolved_output` beside it (`resolver::resolve_parameter` moves
/// `param.limits` onto the `ResolvedParameter` verbatim), and an override
/// payload replaces the value the resolver ends up with. All three reach the
/// same encoder, so all three are checked here — a rule that covered only the
/// base value would leave the identical collapse reachable through a variant.
pub(crate) fn validate_parameter_values(
    definitions: &HashMap<String, Parameter>,
    components: &HashMap<String, Component>,
) -> Result<()> {
    let mut definition_ids: Vec<&String> = definitions.keys().collect();
    definition_ids.sort();
    for id in definition_ids {
        validate_parameter_floats(&definitions[id], &format!("definitions.{id}"), 0)?;
    }

    let mut component_ids: Vec<&String> = components.keys().collect();
    component_ids.sort();
    for component_id in component_ids {
        let component = &components[component_id];
        let mut param_ids: Vec<&String> = component.params.keys().collect();
        param_ids.sort();
        for param_id in param_ids {
            validate_parameter_floats(
                &component.params[param_id],
                &format!("components.{component_id}.params.{param_id}"),
                0,
            )?;
        }
    }

    Ok(())
}

/// One parameter's floats: its own value, its limits, then its override chain
/// in override order then nested-override order — the same walk
/// [`collect_parameter_conditions`] makes over the same structure — including
/// its [`MAX_CHAIN_DEPTH`] ceiling (configflux-dw9i).
fn validate_parameter_floats(parameter: &Parameter, path: &str, depth: usize) -> Result<()> {
    ensure_override_depth(depth, path)?;
    if let Some(value) = &parameter.value {
        reject_non_finite(value, path, "value")?;
    }
    if let Some(limits) = &parameter.limits {
        if let Some(min) = &limits.min {
            reject_non_finite(min, path, "limits.min")?;
        }
        if let Some(max) = &limits.max {
            reject_non_finite(max, path, "limits.max")?;
        }
    }
    for (index, override_block) in parameter.overrides.iter().enumerate() {
        validate_parameter_floats(
            override_block.payload.as_ref(),
            &format!("{path}.overrides[{index}]"),
            depth + 1,
        )?;
    }
    Ok(())
}

/// The parameter-side diagnostic. Carries NO diagnostic code, which is how it
/// lands on the general invalid-input code the rest of the authored-value rules
/// report. Before configflux-py7w the code was recovered from this text, so the
/// authored `<path>` it interpolates could name another rule's phrase and take
/// that rule's code — four chunks with one fault, four codes.
fn reject_non_finite(value: &Value, path: &str, field: &str) -> Result<()> {
    if is_non_finite(value) {
        bail!(
            "Parameter '{}' {} is not a finite number{}",
            path,
            field,
            NON_FINITE_REMEDY
        );
    }
    Ok(())
}

/// Run every ADR-0057 LINK check over the compile set's merged interfaces.
///
/// The signature is the point (ADR-0057 §D9). Each check below reads only what
/// an [`InterfaceSummary`] carries — who exports what, who imports what, a
/// catalogue's entry roster, a binding's outward links — so when ADR-0058
/// serializes that summary as an object header, the linker calls these same
/// functions over headers instead of chunks. Nothing has to be rewritten, and
/// no check can quietly acquire a dependency on a chunk body without changing
/// this signature first.
pub(crate) fn validate_link_summary(summaries: &[InterfaceSummary]) -> Result<()> {
    let merged = crate::interface_summary::merge(summaries);
    validate_merged_summary(&merged, Scope::Complete)
}

/// The same checks over a summary the LINKER merged from object headers
/// (ADR-0058 §D4 stage 1).
///
/// The twin of [`validate_link_summary`] for a caller that already holds the
/// merge. It exists because the linker's merge is not
/// [`crate::interface_summary::merge`]: a header is per UNIT, so the linker
/// folds headers rather than chunk summaries and records the unit against each
/// id instead of a `source_id`. The RULES are identical — that is the whole
/// promise of the §D9 signature — so this delegates rather than re-deriving
/// anything, and `Scope::Complete` is what says the linked set is the whole
/// model.
pub(crate) fn validate_linked_summary(merged: &MergedSummary) -> Result<()> {
    validate_merged_summary(merged, Scope::Complete)
}

/// The same three checks over ONE unit's merged summary (ADR-0058 §D3).
///
/// `merged` is built by the object compile: its `exports`, `requirements` and
/// `clauses` are the unit's own, while `facet_domains`, `catalogue_entries` and
/// `binding_links` also carry what the interface headers declare, so a
/// requirement or a `derive` table that names a sibling unit's binding is
/// checked against the real entry roster rather than skipped.
pub(crate) fn validate_object_summary(merged: &MergedSummary) -> Result<()> {
    validate_merged_summary(merged, Scope::UnitLocal)
}

fn validate_merged_summary(merged: &MergedSummary, scope: Scope) -> Result<()> {
    validate_unique_exports(merged)?;
    // Bindings before requirements: a requirement's `accepts` list is checked
    // against its binding's CATALOGUE, so a binding that does not resolve to
    // one has to be reported as the binding fault it is, not as a requirement
    // fault with a missing entry roster.
    validate_binding_links(merged, scope)?;
    validate_requirements(merged, scope)
}

/// One entity id, one declaring chunk — for every namespace.
///
/// `Compiler::merge_partial` already rejects a duplicate at ingest with a
/// per-namespace diagnostic; this is the structural backstop that
/// `build_ir_index` used to carry inline, re-expressed over the merged summary
/// so the compile path and a future linker enforce it once.
fn validate_unique_exports(merged: &MergedSummary) -> Result<()> {
    check_unique(&merged.exports.components, "Component", Phrasing::Appears)?;
    check_unique(&merged.exports.definitions, "Definition", Phrasing::Appears)?;
    check_unique(&merged.exports.artifacts, "Artifact", Phrasing::Appears)?;
    check_unique(
        &merged.exports.catalogues,
        "Catalogue",
        Phrasing::Declared {
            code: Some(E_INGEST_DUPLICATE_CATALOGUE),
            hint: None,
        },
    )?;
    // The facet namespace carries NO code here, which is what it reported before
    // the code moved onto the refusal: the link/verify mapper never had the
    // duplicate-facet arm the ingest mapper has, and ingest's own merge (or
    // `link::check_duplicate_ids`) claims the fault before this backstop can see
    // it. Preserved rather than changed, since changing it would move a code on
    // a path nothing is known to reach; configflux-rfkn decides it.
    check_unique(
        &merged.exports.facets,
        "Facet",
        Phrasing::Declared {
            code: None,
            hint: None,
        },
    )?;
    check_unique(
        &merged.exports.bindings,
        "Binding",
        Phrasing::Declared {
            code: Some(E_INGEST_DUPLICATE_FACET),
            hint: Some(HINT_DUPLICATE_BINDING_ID),
        },
    )?;

    // ADR-0057 §D3: a binding IS a facet, so the two namespaces share ONE id
    // space. An id declared as both is the same collision as a facet declared
    // twice, and carries the same code — the message says "binding" so the
    // author knows which of the two declarations to rename.
    for (id, binding_sources) in &merged.exports.bindings {
        if let Some(facet_sources) = merged.exports.facets.get(id) {
            coded_bail_hint!(
                E_INGEST_DUPLICATE_FACET,
                HINT_DUPLICATE_BINDING_ID,
                "Binding '{}' is declared in more than one chunk: a binding shares the facet \
                 id space, and '{}' is also declared as a facet (in '{}' and '{}')",
                id,
                id,
                facet_sources.first().map(String::as_str).unwrap_or("<unknown>"),
                binding_sources.first().map(String::as_str).unwrap_or("<unknown>")
            );
        }
    }
    Ok(())
}

/// Which duplicate wording a namespace uses, and what code its duplicate
/// carries. The wordings are NOT interchangeable — they are what an author
/// reads, and both are pinned by tests — but since configflux-py7w neither one
/// selects the code: the namespace states that here instead.
#[derive(Copy, Clone)]
enum Phrasing {
    /// "<noun> '<id>' appears in multiple chunks" — the entity namespaces, none
    /// of which has a duplicate code of its own.
    Appears,
    /// "<noun> '<id>' is declared in more than one chunk" — the pack-global
    /// namespaces. `code` is the duplicate code for this namespace, or `None`
    /// where it has none; `hint` overrides that code's default remedy.
    Declared {
        code: Option<&'static str>,
        hint: Option<&'static str>,
    },
}

impl Phrasing {
    /// The refusal a duplicate in this namespace is reported as.
    fn refusal(self, message: String) -> anyhow::Error {
        match self {
            Phrasing::Appears | Phrasing::Declared { code: None, .. } => {
                anyhow::Error::msg(message)
            }
            Phrasing::Declared {
                code: Some(code),
                hint: Some(hint),
            } => coded_with_hint(code, hint, message),
            Phrasing::Declared {
                code: Some(code),
                hint: None,
            } => coded(code, message),
        }
    }
}

fn check_unique(
    index: &BTreeMap<String, Vec<String>>,
    noun: &str,
    phrasing: Phrasing,
) -> Result<()> {
    for (id, sources) in index {
        if sources.len() < 2 {
            continue;
        }
        let message = match phrasing {
            Phrasing::Appears => format!(
                "{} '{}' appears in multiple chunks: '{}' and '{}'",
                noun, id, sources[0], sources[1]
            ),
            Phrasing::Declared { .. } => format!(
                "{} '{}' is declared in more than one chunk: '{}' and '{}'",
                noun, id, sources[0], sources[1]
            ),
        };
        return Err(phrasing.refusal(message));
    }
    Ok(())
}

/// Every binding resolves to a catalogue, a legal default, and a declared
/// derive source (ADR-0057 §D3).
///
/// All of it is `E_BINDING_INVALID`, and every message names the binding first
/// — the author's fix is always on the binding, whichever half of the pair is
/// wrong. Bindings are walked id-ascending so the first surfaced diagnostic is
/// deterministic.
fn validate_binding_links(merged: &MergedSummary, scope: Scope) -> Result<()> {
    for (id, link) in &merged.binding_links {
        if link.default.is_some() && link.derive_source_count > 0 {
            coded_bail!(
                E_BINDING_INVALID,
                "Binding '{}' declares both `default` and `derive`, which are mutually \
                 exclusive: a derive table already fixes the entry",
                id
            );
        }

        let Some(entries) = merged.catalogue_entries.get(&link.catalogue) else {
            // Object time: the catalogue may be a sibling unit's, recorded as a
            // catalogue import. Nothing below can run without an entry roster,
            // so the whole binding waits for the link.
            if scope.defers_unresolved() {
                continue;
            }
            coded_bail!(
                E_BINDING_INVALID,
                "Binding '{}' names catalogue '{}', which is not declared under `catalogues`",
                id,
                link.catalogue
            );
        };

        if let Some(default) = &link.default {
            if !entries.iter().any(|entry| entry == default) {
                coded_bail!(
                    E_BINDING_INVALID,
                    "Binding '{}' default '{}' is not an entry of catalogue '{}' [{}]",
                    id,
                    default,
                    link.catalogue,
                    entries.join(", ")
                );
            }
        }

        if link.derive_source_count > 1 {
            coded_bail!(
                E_BINDING_INVALID,
                "Binding '{}' derives from {} sources; exactly one source is supported",
                id,
                link.derive_source_count
            );
        }

        let Some(source) = &link.derive_source else {
            continue;
        };
        let Some(domain) = merged.declared_domain(source) else {
            // Object time: the derive source may be a sibling unit's facet,
            // recorded as a facet import. Its keys cannot be checked without a
            // domain, so the derive table waits for the link.
            if scope.defers_unresolved() {
                continue;
            }
            coded_bail!(
                E_BINDING_INVALID,
                "Binding '{}' derives from '{}', which is not a declared facet or binding",
                id,
                source
            );
        };
        for (key, entry) in &link.derive_pairs {
            if !domain.iter().any(|value| value == key) {
                coded_bail!(
                    E_BINDING_INVALID,
                    "Binding '{}' derive key '{}' is not a declared value of '{}' [{}]",
                    id,
                    key,
                    source,
                    domain.join(", ")
                );
            }
            if !entries.iter().any(|candidate| candidate == entry) {
                coded_bail!(
                    E_BINDING_INVALID,
                    "Binding '{}' derive entry '{}' is not an entry of catalogue '{}' [{}]",
                    id,
                    entry,
                    link.catalogue,
                    entries.join(", ")
                );
            }
        }
    }
    Ok(())
}

/// Every component requirement resolves to a declared binding, accepts only
/// entries that binding's catalogue holds, and leaves at least one entry every
/// requirement on that binding can live with (ADR-0057 §D4).
///
/// Three faults, two codes. `E_REQUIRES_INVALID` is a fault in ONE requirement
/// — the author fixes the component. `E_BINDING_NO_ACCEPTABLE_ENTRY` is a fault
/// in the model as a whole: each requirement is individually legal and together
/// they leave nothing to choose, so the message names the binding and every
/// list, because the author cannot tell which one to widen without seeing them
/// all.
///
/// All of it is static set arithmetic; no solver is involved, and none is
/// needed. The intersection check deliberately ignores component `condition`s:
/// two components whose conditions are mutually exclusive could in principle
/// coexist with disjoint `accepts` lists, but establishing that is a
/// satisfiability question, and ADR-0057 §D4 asks for the set check. Being
/// stricter here fails an author's model early with an explanation they can
/// act on, rather than late with a core they have to read.
///
/// Requirements are walked component-then-slot ascending
/// ([`crate::interface_summary::merge`] sorts them), so the first surfaced
/// diagnostic is deterministic.
fn validate_requirements(merged: &MergedSummary, scope: Scope) -> Result<()> {
    for requirement in &merged.requirements {
        let Some(link) = merged.binding_links.get(&requirement.binding) else {
            // Object time: a requirement on a sibling unit's binding is a
            // binding import, and the linker refuses it if nothing declares it
            // (`E_LINK_UNRESOLVED_IMPORT`). With no binding there is no
            // catalogue, so its `accepts` list waits for the link too.
            if scope.defers_unresolved() {
                continue;
            }
            coded_bail!(
                E_REQUIRES_INVALID,
                "component '{}' requires slot '{}' bound to unknown binding '{}'",
                requirement.component,
                requirement.slot,
                requirement.binding
            );
        };

        // `validate_binding_links` ran first and proved the catalogue exists,
        // so an absent roster here would mean the two walked different data.
        // An empty slice then rejects every entry name rather than accepting
        // them all — fail closed, not open.
        let entries = merged
            .catalogue_entries
            .get(&link.catalogue)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        let Some(accepts) = &requirement.accepts else {
            continue;
        };

        if accepts.is_empty() {
            coded_bail!(
                E_REQUIRES_INVALID,
                "component '{}' requires slot '{}' with an empty `accepts` list; a component \
                 that accepts no entry of binding '{}' can never be satisfied — drop `accepts` \
                 to accept every entry",
                requirement.component,
                requirement.slot,
                requirement.binding
            );
        }

        let mut seen: HashSet<&str> = HashSet::new();
        for entry in accepts {
            if !seen.insert(entry.as_str()) {
                coded_bail!(
                    E_REQUIRES_INVALID,
                    "component '{}' requires slot '{}' accepting entry '{}' more than once",
                    requirement.component,
                    requirement.slot,
                    entry
                );
            }
            if !entries.iter().any(|candidate| candidate == entry) {
                coded_bail!(
                    E_REQUIRES_INVALID,
                    "component '{}' requires slot '{}' accepting entry '{}', which is not an \
                     entry of catalogue '{}' [{}]",
                    requirement.component,
                    requirement.slot,
                    entry,
                    link.catalogue,
                    entries.join(", ")
                );
            }
        }

        // The `accepts` lowering wraps the conjunct in the declaring
        // component's inclusion `condition` (ADR-0054 §3). A condition that
        // does not parse would make that conjunct unparseable, and the failure
        // would surface deep in the emitter naming a synthesized id. Refuse it
        // here, where the message can name the component. An unparseable
        // condition on a component with NO `accepts` stays legal — it widens no
        // facet and forms no clause, exactly as before.
        if let Some(condition) = requirement.condition.as_deref() {
            let trimmed = condition.trim();
            if !trimmed.is_empty() && conditions::parse_condition_expr(trimmed).is_err() {
                coded_bail!(
                    E_REQUIRES_INVALID,
                    "component '{}' requires slot '{}' with `accepts`, but the component's \
                     condition '{}' does not parse; an `accepts` list is only meaningful when \
                     the compiler can tell whether the component is included",
                    requirement.component,
                    requirement.slot,
                    condition
                );
            }
        }
    }

    validate_accepts_intersection(merged)
}

/// For every binding, at least one catalogue entry survives every `accepts`
/// list that names it (ADR-0057 §D4, `E_BINDING_NO_ACCEPTABLE_ENTRY`).
///
/// A requirement WITHOUT `accepts` accepts everything and so narrows nothing —
/// it is the identity of this intersection, which is why it is skipped rather
/// than folded in as the full entry roster.
fn validate_accepts_intersection(merged: &MergedSummary) -> Result<()> {
    // binding -> the entries still standing, seeded from the catalogue on first
    // sight and narrowed by each list. `BTreeMap` so the first reported binding
    // is deterministic.
    let mut surviving: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut narrowed: BTreeMap<&str, Vec<String>> = BTreeMap::new();

    for requirement in &merged.requirements {
        let Some(accepts) = &requirement.accepts else {
            continue;
        };
        let Some(link) = merged.binding_links.get(&requirement.binding) else {
            continue;
        };
        let entry = surviving
            .entry(requirement.binding.as_str())
            .or_insert_with(|| {
                merged
                    .catalogue_entries
                    .get(&link.catalogue)
                    .cloned()
                    .unwrap_or_default()
            });
        entry.retain(|candidate| accepts.iter().any(|accepted| accepted == candidate));
        narrowed
            .entry(requirement.binding.as_str())
            .or_default()
            .push(format!(
                "{}.{} accepts [{}]",
                requirement.component,
                requirement.slot,
                accepts.join(", ")
            ));
    }

    for (binding, entries) in surviving {
        if !entries.is_empty() {
            continue;
        }
        coded_bail!(
            E_BINDING_NO_ACCEPTABLE_ENTRY,
            "binding '{}' has no entry every requirement accepts: {}",
            binding,
            narrowed
                .get(binding)
                .map(|lists| lists.join("; "))
                .unwrap_or_default()
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "link_verify_catalogue_tests.rs"]
mod catalogue_tests;

#[cfg(test)]
#[path = "link_verify_requires_tests.rs"]
mod requires_tests;

#[cfg(test)]
#[path = "link_verify_symbol_charset_tests.rs"]
mod symbol_charset_tests;

#[cfg(test)]
mod facet_tests {
    use super::*;
    use crate::schema::{Component, Facet, Parameter};

    fn facet(values: &[&str], default: Option<&str>, open: bool) -> Facet {
        Facet {
            values: values.iter().map(|s| s.to_string()).collect(),
            default: default.map(|s| s.to_string()),
            open,
            doc: None,
        }
    }

    fn facets(pairs: Vec<(&str, Facet)>) -> HashMap<String, Facet> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn component_with_condition(cond: &str) -> HashMap<String, Component> {
        let mut map = HashMap::new();
        map.insert(
            "c".to_string(),
            Component {
                r#type: None,
                condition: Some(cond.to_string()),
                depends_on: Vec::new(),
                requires: Default::default(),
                params: HashMap::new(),
            },
        );
        map
    }

    fn param_with_override_condition(cond: &str) -> HashMap<String, Parameter> {
        let mut inner = empty_param();
        let base = empty_param();
        inner.overrides = vec![crate::schema::ConditionalBlock {
            condition: cond.to_string(),
            payload: Box::new(base),
        }];
        let mut map = HashMap::new();
        map.insert("d".to_string(), inner);
        map
    }

    fn empty_param() -> Parameter {
        Parameter {
            inherits: None,
            r#type: None,
            unit: None,
            doc: None,
            value: None,
            lifecycle: None,
            safety: None,
            access: None,
            limits: None,
            req_id: None,
            facet: None,
            overrides: Vec::new(),
        }
    }

    #[test]
    fn empty_values_is_rejected() {
        let f = facets(vec![("region", facet(&[], None, false))]);
        let err = validate_facets(&f, &HashMap::new(), &HashMap::new()).unwrap_err();
        assert!(format!("{err}").contains("empty value domain"), "err: {err}");
    }

    #[test]
    fn duplicate_values_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "eu"], None, false))]);
        let err = validate_facets(&f, &HashMap::new(), &HashMap::new()).unwrap_err();
        assert!(format!("{err}").contains("duplicate value 'eu'"), "err: {err}");
    }

    #[test]
    fn default_not_in_values_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("mars"), false))]);
        let err = validate_facets(&f, &HashMap::new(), &HashMap::new()).unwrap_err();
        assert!(
            format!("{err}").contains("default 'mars' is not one of"),
            "err: {err}"
        );
    }

    #[test]
    fn valid_facet_with_default_is_accepted() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        assert!(validate_facets(&f, &HashMap::new(), &HashMap::new()).is_ok());
    }

    #[test]
    fn closed_facet_undeclared_condition_value_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let comps = component_with_condition("region == 'mars'");
        let err = validate_facets(&f, &comps, &HashMap::new()).unwrap_err();
        // The message names the facet, the value, and the declared domain, and
        // carries the phrase product_api maps to E_FACET_VALUE_UNDECLARED.
        let msg = format!("{err}");
        assert!(msg.contains("closed facet 'region'"), "err: {msg}");
        assert!(msg.contains("'mars'"), "err: {msg}");
        assert!(msg.contains("eu, us"), "err: {msg}");
    }

    #[test]
    fn closed_facet_declared_condition_value_is_accepted() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let comps = component_with_condition("region == 'us'");
        assert!(validate_facets(&f, &comps, &HashMap::new()).is_ok());
    }

    #[test]
    fn open_facet_extends_with_undeclared_condition_value() {
        // `open: true` means the declared domain is extensible, so a condition
        // value outside it is NOT an error (ADR-0047 §3).
        let f = facets(vec![("region", facet(&["eu"], Some("eu"), true))]);
        let comps = component_with_condition("region == 'mars'");
        assert!(validate_facets(&f, &comps, &HashMap::new()).is_ok());
    }

    #[test]
    fn undeclared_facet_keeps_legacy_inferred_behavior() {
        // A condition referencing a facet with no declaration at all is never
        // gated — its domain stays the inferred set, exactly as before.
        let comps = component_with_condition("variant == 'heavy'");
        assert!(validate_facets(&HashMap::new(), &comps, &HashMap::new()).is_ok());
    }

    #[test]
    fn closed_facet_check_also_scans_param_override_conditions() {
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let defs = param_with_override_condition("region == 'mars'");
        let err = validate_facets(&f, &HashMap::new(), &defs).unwrap_err();
        assert!(
            format!("{err}").contains("closed facet 'region'"),
            "err: {err}"
        );
    }

    #[test]
    fn unparseable_condition_cannot_violate_a_closed_domain() {
        // A condition the BDD grammar cannot represent widens no domain today,
        // so it must not trip the closed-domain check either.
        let f = facets(vec![("region", facet(&["eu", "us"], Some("eu"), false))]);
        let comps = component_with_condition("this is not <> a condition");
        assert!(validate_facets(&f, &comps, &HashMap::new()).is_ok());
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::*;
    use crate::schema::{Constraint, Facet};

    fn facet(values: &[&str], open: bool) -> Facet {
        Facet {
            values: values.iter().map(|s| s.to_string()).collect(),
            default: None,
            open,
            doc: None,
        }
    }

    fn facets(pairs: Vec<(&str, Facet)>) -> HashMap<String, Facet> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn constraints(pairs: Vec<(&str, &str)>) -> HashMap<String, Constraint> {
        pairs
            .into_iter()
            .map(|(id, condition)| {
                (
                    id.to_string(),
                    Constraint {
                        condition: condition.to_string(),
                        doc: None,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_model_with_no_constraints_is_accepted() {
        assert!(validate_constraints(&HashMap::new(), &HashMap::new()).is_ok());
    }

    #[test]
    fn unparseable_constraint_is_an_ingest_error() {
        // THE asymmetry with a selector condition (ADR-0054 §1): an
        // unparseable selector is skipped, an unparseable CONSTRAINT is fatal.
        // A policy nobody can evaluate must never be silently dropped.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("bogus", "this is not <> a condition")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Constraint 'bogus'"), "err: {msg}");
        assert!(msg.contains("does not parse"), "err: {msg}");
    }

    #[test]
    fn constraint_over_a_declared_facet_is_accepted() {
        let f = facets(vec![
            ("environment", facet(&["dev", "staging", "prod"], false)),
            ("log_level", facet(&["info", "debug"], false)),
        ]);
        let c = constraints(vec![(
            "prod_forbids_debug",
            "environment != 'prod' || log_level != 'debug'",
        )]);
        assert!(validate_constraints(&c, &f).is_ok());
    }

    #[test]
    fn constraint_over_an_undeclared_facet_is_rejected() {
        // configflux-6j91. Undeclared is undeclared: this function cannot see
        // the model's conditions, so a facet some selector mentions and a facet
        // nothing mentions are the same input here and take the same diagnostic
        // — which is the point of dropping the union. The two shapes are told
        // apart where whole models exist:
        // `compiler_core_tests::a_constraint_over_a_condition_inferred_facet_
        // fails_before_anything_is_emitted` pins the condition-inferred one.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("ghost", "tls_mode == 'strict'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        let msg = format!("{err}");
        // Constraint id, offending facet, and the remedy all have to be here:
        // "declare it" is the only fix that keeps the policy.
        assert!(msg.contains("Constraint 'ghost'"), "err: {msg}");
        assert!(msg.contains("facet 'tls_mode'"), "err: {msg}");
        assert!(msg.contains("not declared"), "err: {msg}");
        assert!(msg.contains("declare the facet"), "err: {msg}");
        // Both rules carry E_FACET_VALUE_UNDECLARED, but this one is not a
        // closed-domain violation — `tls_mode` has no domain at all — so
        // borrowing the sibling's phrasing would make the message false.
        assert!(!msg.contains("closed facet"), "err: {msg}");
    }

    #[test]
    fn the_offending_facet_is_named_even_when_a_later_predicate_is_declared() {
        // The walk must report the first illegal facet in AST order, not fall
        // through because some other predicate in the same expression is fine.
        let f = facets(vec![("environment", facet(&["dev", "prod"], false))]);
        let c = constraints(vec![("mixed", "arch == 'x86' || environment != 'prod'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        assert!(format!("{err}").contains("facet 'arch'"), "err: {err}");
    }

    #[test]
    fn closed_facet_undeclared_eq_value_is_rejected() {
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("bad", "region == 'mars'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        // Carries the phrase product_api maps to E_FACET_VALUE_UNDECLARED, and
        // names the constraint so the author knows which policy is wrong.
        let msg = format!("{err}");
        assert!(msg.contains("Constraint 'bad'"), "err: {msg}");
        assert!(msg.contains("closed facet 'region'"), "err: {msg}");
        assert!(msg.contains("eu, us"), "err: {msg}");
    }

    #[test]
    fn closed_facet_undeclared_ne_value_is_also_rejected() {
        // Deliberately STRICTER than `validate_facets`, which checks `==` only.
        // Against a closed domain `region != 'mars'` is a tautology, so a typo
        // here does not merely fail to filter — it silently voids the policy.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![("typo", "region != 'marz'")]);
        let err = validate_constraints(&c, &f).unwrap_err();
        assert!(
            format!("{err}").contains("closed facet 'region'"),
            "err: {err}"
        );
    }

    #[test]
    fn open_facet_accepts_a_value_outside_its_declared_domain() {
        // An open domain is extensible (ADR-0047 §3), so the closed-domain rule
        // does not apply — same carve-out `validate_facets` makes.
        let f = facets(vec![("region", facet(&["eu"], true))]);
        let c = constraints(vec![("ok", "region != 'mars'")]);
        assert!(validate_constraints(&c, &f).is_ok());
    }

    #[test]
    fn disjunctive_and_negated_constraints_are_supported_verbatim() {
        // The whole point of the construct: a policy is an arbitrary
        // `ConditionExpr`, not a pure conjunction. Nothing here may filter on
        // shape (ADR-0054 §2).
        let f = facets(vec![
            ("environment", facet(&["dev", "prod"], false)),
            ("log_level", facet(&["info", "debug"], false)),
        ]);
        let c = constraints(vec![
            ("disjunction", "environment != 'prod' || log_level != 'debug'"),
            ("negation", "!(environment == 'prod')"),
            ("cardinality", "exactly_one_of(log_level == 'info', log_level == 'debug')"),
        ]);
        assert!(validate_constraints(&c, &f).is_ok());
    }

    #[test]
    fn the_first_reported_violation_is_id_ascending() {
        // Two broken constraints; the diagnostic must be a deterministic
        // function of the model, not of HashMap iteration order.
        let f = facets(vec![("region", facet(&["eu", "us"], false))]);
        let c = constraints(vec![
            ("zeta", "region == 'pluto'"),
            ("alpha", "region == 'mars'"),
        ]);
        let err = validate_constraints(&c, &f).unwrap_err();
        assert!(format!("{err}").contains("Constraint 'alpha'"), "err: {err}");
    }

}

