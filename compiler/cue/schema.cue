// ConfigFlux 150% model — CUE authoring schema (L1).
//
// Hand-written from compiler/src/schema.rs (the single source of truth, per
// ADR 0021 decision 3). This schema validates authored configuration *chunks*
// at the structural level; the Rust compiler + BDD solver still own conditions,
// overrides, cardinality, and late-binding (ADR 0021 decision 1).
//
// Each authored chunk is a `#Config` (package + version + any of
// definitions/components/artifacts). `condition` strings and `overrides` blocks
// are opaque pass-through data here — CUE does not interpret them.
//
// Recursion note (ADR 0021 decision 7): `#ConditionalBlock` is a `#Parameter`
// plus a required `condition`. A *closed* `#Parameter` rejects the extra field
// (`close(P) & {condition}` fails), so the shared fields live in an OPEN hidden
// template `_paramFields`, and each definition closes it — `#ConditionalBlock`
// closes it together with `condition`.
package configflux

#Role:        "developer" | "integrator" | "technician" | "supervisor" | "super_user"
// Note: "q_m" is serde's snake_case rendering of the `QM` variant (not "qm").
#SafetyLevel: "q_m" | "sil1" | "sil2" | "sil3" | "sil4"
#Lifecycle:   "construction" | "startup" | "runtime"

// serde(untagged) Value = Integer(i64) | Float(f64) | Boolean(bool) | String.
#Value: int | float | bool | string

// Authored-ID snake_case constraint (ADR-0027 decision 4). This MOVES the
// `validate_snake_case_ids` check (ingest_merge.rs:62-79) from the Rust ingest
// path to the CUE authoring front-end; the check is preserved, not dropped.
//
// It must reproduce `validate_snake_case` (ingest_merge.rs:81-104) EXACTLY:
//   - first byte must be ascii-lowercase            (rejects `Foo`, `1foo`, ``)
//   - every byte is `[a-z0-9_]`
//   - NO double underscore `__` (the `prev_underscore` guard, line 96-99)
//   - a SINGLE trailing underscore is permitted (the guard never fires at EOL,
//     so `foo_` is accepted by Rust today — the pattern must accept it too)
//
// The naive `^[a-z][a-z0-9_]*$` admits `__` and would silently weaken the
// contract (ADR-0027 decision 4, "RESOLVED for B-1"). RE2 has no lookahead, so
// `__` is excluded structurally: after the leading char, each unit is either a
// non-underscore alnum or a single `_` glued to a following alnum, with one
// optional trailing `_`. `ensure_snake_case_ident` (resolver.rs:66-87) governs
// *runtime* scope selectors, not authored config, and stays Rust-owned.
#snakeId: =~"^[a-z]([a-z0-9]|_[a-z0-9])*_?$"

#Limits: close({
	min?:     #Value
	max?:     #Value
	min_len?: int & >=0
	max_len?: int & >=0
})

// Open template of the shared Parameter fields (see recursion note above).
_paramFields: {
	inherits?:  string
	type?:      string
	unit?:      string
	doc?:       string
	value?:     #Value
	lifecycle?: #Lifecycle
	safety?:    #SafetyLevel
	access?:    #Role
	limits?:    #Limits
	req_id?:    string
	overrides?: [...#ConditionalBlock]
}

#Parameter: close(_paramFields)

#ConditionalBlock: close(_paramFields & {
	condition!: string
})

// A definition is a valueless #Parameter (ADR-0027 decision 3). Definitions
// declare the inheritable shape (type/unit/safety/lifecycle/access/limits/doc);
// `value` is authored on the *using* component param, never on the definition
// — exactly mirroring resolver.rs:358, which deliberately never inherits
// `value`. Forbidding `value` here (via `_|_`) keeps the gap-fill engine below
// from ever having a parent `value` to push down. Single source of truth: the
// fields stay in `_paramFields`; only `value` is closed off.
#Definition: close(_paramFields & {
	value?: _|_
})

#Component: close({
	type?:      string
	condition?: string
	// depends_on entries are component references; validate_snake_case_ids
	// (ingest_merge.rs:71-73) checks each dep, so the element type is #snakeId.
	depends_on?: [...#snakeId]
	// component param KEYS are snake_case-checked (ingest_merge.rs:74-76); their
	// VALUES are full #Parameter (value-bearing — the using param authors value).
	params?: close({[#snakeId]: #Parameter})
})

#Artifact: close({
	name!:    string
	version?: string
	hash?:    string
	source?:  string
	target?:  string
	doc?:     string
})

// A first-class facet/domain declaration (ADR-0047). A facet is a named,
// ordered value domain with an optional default — the authored construct that
// lets a default arm no condition mentions become visible and resolvable.
// `values` is non-empty here (`[string, ...string]`); its uniqueness and the
// `default ∈ values` membership are re-validated in Rust at link time (CUE
// can't express list-element uniqueness without the `list` package, so those
// two invariants live in `link_verify::validate_facets`, per ADR-0021's
// "CUE authors, Rust re-validates"). `open` defaults to false (closed domain).
#Facet: close({
	values!: [string, ...string]
	default?: string
	open?:   bool | *false
	doc?:    string
})

// A named policy assertion (ADR-0054 §1). `condition` is an expression in the
// existing condition grammar over facet values — the SAME grammar a component or
// override `condition` uses, deliberately: ADR-0054 adds no new expression
// language, operator, or evaluator.
//
// The difference from a selector `condition` is semantic, not syntactic. A
// selector decides what a configuration CONTAINS; a constraint decides what a
// user is ALLOWED TO PICK, under one rule — every declared constraint must hold
// in every resolved configuration (§2). CUE cannot express that difference, so
// it only checks the shape here; unlike a selector condition, the Rust ingest
// REJECTS an unparseable constraint rather than dropping it
// (`link_verify::validate_constraints`, ADR-0021 "CUE authors, Rust
// re-validates").
#Constraint: close({
	condition!: string
	doc?:       string
})

// A single authored chunk. definitions/components/artifacts are all optional
// (serde(default)); package + version are required and shared across a pack's
// chunks. Authored top-level IDs (definition/component/artifact keys) carry the
// snake_case constraint that moved off the Rust ingest path (ADR-0027 decision
// 4 / ingest_merge.rs:63-69); `close({[#snakeId]: …})` makes a non-matching key
// a "field not allowed" error. Definitions are valueless (#Definition).
#Config: close({
	package!: string
	version!: string
	definitions?: close({[#snakeId]: #Definition})
	components?: close({[#snakeId]:  #Component})
	artifacts?: close({[#snakeId]:   #Artifact})
	// Fourth top-level namespace (ADR-0047). Facet keys carry the #snakeId
	// constraint, like the other three namespaces. Facets pass through the
	// #ResolvePack / export layer verbatim — no inheritance or gap-fill.
	facets?: close({[#snakeId]: #Facet})
	// Fifth top-level namespace (ADR-0054 §1). A map keyed by id, not a list:
	// the id is the primary key, so diagnostics and unsat cores can NAME the
	// violated policy, duplicate keys are rejected for free, and the walk order
	// is deterministic for byte-stable emission. Pack-global; passes through the
	// resolve layer verbatim, like facets.
	constraints?: close({[#snakeId]: #Constraint})
})

// A resolution profile (scenario selection domains + default context). Mirrors
// the compiler's `ScenarioProfile` (compiler/src/scenario_baseline_tests.rs):
// `selection_domains` and `default_context` are required; `profile_id` and
// `scenario_id` are optional metadata carried by most fixtures. Profiles are
// resolution inputs, NOT part of the 150% model (#Config) — they are validated
// here so the CUE front-end covers every authored scenario file.
#Profile: close({
	selection_domains!: {[string]: [...string]}
	default_context!: {[string]:  string}
	profile_id?:  string
	scenario_id?: string
})

// ----------------------------------------------------------------------------
// Inheritance / unification layer (ADR-0027 decisions 1-3).
//
// CUE becomes the semantic owner of `inherits` resolution. This layer is the
// gap-fill engine that REPLACES `apply_inheritance` (resolver.rs:335-361). It
// runs at WHOLE-PACK evaluation: a component param's `inherits` pointer is
// resolved against the sibling 00_definitions chunk, while emission stays
// per-file (ADR-0027 decision 2 — "resolution whole-pack, attribution
// per-file"). B-3's export wires the two chunks into scope and feeds them here;
// B-4's equivalence corpus is the proof that CUE-resolved ≡ Rust-resolved.
//
// Two invariants this layer enforces, taken straight from the Rust engine:
//   1. "Fill Gaps, do not overwrite" — a field already authored on the child
//      wins; the parent only fills an absent field (resolver.rs:336-357).
//   2. value-not-inherited — `value` is taken ONLY from the child, NEVER from
//      the parent (resolver.rs:358-360). Definitions are valueless by
//      construction (#Definition forbids `value`), so there is no parent value
//      to leak; this rule keeps the contract explicit even so.
//
// Note the inheritable set is exactly resolver.rs's: type, unit, safety,
// lifecycle, access, limits, doc. `inherits` (the pointer), `req_id`,
// `condition`, and `overrides` are NOT gap-filled — they stay author-owned and,
// for overrides/condition, opaque late-bound data (ADR-0003).

// #InheritFields: the gap-fillable subset of a definition (resolver.rs:337-356).
#InheritFields: {
	type?:      string
	unit?:      string
	safety?:    #SafetyLevel
	lifecycle?: #Lifecycle
	access?:    #Role
	limits?:    #Limits
	doc?:       string
}

// #ResolveParam: resolve one authored child param against a definitions table.
//   _child: the authored #Parameter (may carry inherits + its own value).
//   _defs:  the pack's definitions map ({[#snakeId]: #Definition}).
// `out` is the resolved param: child fields take precedence, gaps filled from
// the inherited definition, `value` preserved from the child only.
#ResolveParam: {
	_child: #Parameter
	_defs: {[#snakeId]: #Definition}

	// The parent definition selected by the child's `inherits` pointer, or an
	// empty struct when the child does not inherit (nothing to fill).
	let _parent = {
		if _child.inherits != _|_ {_defs[_child.inherits]}
		if _child.inherits == _|_ {}
	}

	out: {
		// Gap-fill: parent fills a field ONLY when the child left it absent.
		if _parent.type != _|_ if _child.type == _|_ {type: _parent.type}
		if _parent.unit != _|_ if _child.unit == _|_ {unit: _parent.unit}
		if _parent.safety != _|_ if _child.safety == _|_ {safety: _parent.safety}
		if _parent.lifecycle != _|_ if _child.lifecycle == _|_ {lifecycle: _parent.lifecycle}
		if _parent.access != _|_ if _child.access == _|_ {access: _parent.access}
		if _parent.limits != _|_ if _child.limits == _|_ {limits: _parent.limits}
		if _parent.doc != _|_ if _child.doc == _|_ {doc: _parent.doc}

		// Author-owned fields pass through verbatim (no inheritance).
		if _child.type != _|_ {type: _child.type}
		if _child.unit != _|_ {unit: _child.unit}
		if _child.safety != _|_ {safety: _child.safety}
		if _child.lifecycle != _|_ {lifecycle: _child.lifecycle}
		if _child.access != _|_ {access: _child.access}
		if _child.limits != _|_ {limits: _child.limits}
		if _child.doc != _|_ {doc: _child.doc}
		if _child.req_id != _|_ {req_id: _child.req_id}
		if _child.inherits != _|_ {inherits: _child.inherits}
		// overrides pass through verbatim when authored (opaque late-bound data,
		// ADR-0003). `overrides` is optional, so guard on presence rather than
		// len() (CUE forbids `len()` on an optional field reference).
		if _child.overrides != _|_ {overrides: _child.overrides}

		// value-not-inherited: child only, never the parent.
		if _child.value != _|_ {value: _child.value}
	}
}

// #ResolvePack: resolve a whole pack — every param of every component in the
// components chunk is gap-filled against the pack's definitions chunk. Emission
// (B-3) then selects only the originating chunk, so attribution stays per-file
// while resolution saw the full pack. `definitions` pass through unchanged
// (they are already the inheritance roots and remain valueless).
#ResolvePack: {
	_definitions: {[#snakeId]: #Definition}
	_components: {[#snakeId]:  #Component}
	// Facets are pack-global and pass through the resolution layer verbatim —
	// no inheritance, gap-fill, or merge (ADR-0047 §1). Optional: a pack that
	// declares no facet omits this input entirely.
	_facets?: {[#snakeId]: #Facet}
	// Constraints are pack-global policy assertions and likewise pass through
	// the resolution layer verbatim (ADR-0054 §1) — there is nothing to gap-fill
	// in a proposition. Optional: a pack that declares no policy omits it.
	_constraints?: {[#snakeId]: #Constraint}

	definitions: _definitions
	if _facets != _|_ {facets: _facets}
	if _constraints != _|_ {constraints: _constraints}
	components: {
		for _cid, _c in _components {
			(_cid): _c & {
				if _c.params != _|_ {
					params: {
						for _pk, _pv in _c.params {
							(_pk): (#ResolveParam & {_child: _pv, _defs: _definitions}).out
						}
					}
				}
			}
		}
	}
}
