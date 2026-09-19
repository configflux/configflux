#!/usr/bin/env bash
#
# Validate the hand-written CUE authoring schema (schema.cue) against every
# authored scenario file and a battery of malformed inputs. Part of the CUE
# authoring front-end (ADR 0021; configflux-vfrz + configflux-iabu).
#
# Positive coverage:
#   - #Config:  every scenario chunk, every examples/ config, and a synthetic
#     full-surface fixture (testdata/full_surface.json) that exercises the
#     fields/enums/value-types/limits/nested-overrides the real corpus does not.
#   - #Profile: every scenario profile.toml.
# Division of labor:
#   - mutation fixtures are structurally valid (pass #Config); their semantic
#     defects (unknown dep, unreachable branch) are the compiler's job.
# Negative coverage:
#   - malformed #Config and #Profile inputs must be rejected.
#
# Requires: python3 >= 3.11 (tomllib) and the pinned `cue` evaluator. By default
# this resolves the hermetic, Bazel-pinned cue (ADR-0021 Decision 5) via
# //tools:cue_binary — there is no silent $PATH fallback (an unpinned evaluator
# can change model_hash, ADR-0027). Set $CUE to opt into a specific binary,
# e.g. in CI:
#   CUE=/path/to/cue compiler/cue/validate_fixtures.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
# shellcheck source=tools/lib/resolve_cue.sh
source "$REPO/tools/lib/resolve_cue.sh"
CUE="$(configflux_resolve_cue)" || exit $?
SCHEMA="$HERE/schema.cue"
SCENARIOS="$REPO/compiler/scenarios"
EXAMPLES="$REPO/examples"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

pos=0
pos_fail=0
neg_ok=0
neg_fail=0

to_json() {
  case "$1" in
    *.json) cp "$1" "$tmp/d.json" ;;
    *) python3 -c "import tomllib,json,sys; json.dump(tomllib.load(open(sys.argv[1],'rb')), open(sys.argv[2],'w'))" "$1" "$tmp/d.json" ;;
  esac
}

vet_pos() {
  to_json "$1"
  if "$CUE" vet -d "$2" "$tmp/d.json" "$SCHEMA" 2>"$tmp/e"; then
    pos=$((pos + 1))
  else
    pos_fail=$((pos_fail + 1))
    echo "  FAIL ($2) ${1#"$REPO"/}"
    sed 's/^/    /' "$tmp/e"
  fi
}

check_reject() {
  printf '%s' "$2" >"$tmp/n.json"
  if "$CUE" vet -d "$3" "$tmp/n.json" "$SCHEMA" 2>/dev/null; then
    echo "  NOT REJECTED: $1"
    neg_fail=$((neg_fail + 1))
  else
    neg_ok=$((neg_ok + 1))
  fi
}

# Positive assertion for an inline JSON literal (counts into the positive tally).
accept_inline() {
  printf '%s' "$2" >"$tmp/p.json"
  if "$CUE" vet -d "$3" "$tmp/p.json" "$SCHEMA" 2>"$tmp/e"; then
    pos=$((pos + 1))
  else
    pos_fail=$((pos_fail + 1))
    echo "  FAIL ($3) $1"
    sed 's/^/    /' "$tmp/e"
  fi
}

echo "== schema compiles =="
"$CUE" vet "$SCHEMA"

echo "== positive #Config: scenario chunks + examples + full-surface fixture =="
while IFS= read -r f; do vet_pos "$f" '#Config'; done < <(find "$SCENARIOS" -path '*/chunks/*.toml' | sort)
while IFS= read -r f; do vet_pos "$f" '#Config'; done < <(find "$EXAMPLES" -name '*.toml' | sort)
# The flat JSON packs (s_catalogue_binding, s_facet_equality, s_labeled_mus,
# s_requires_accepts) are authored as .json directly rather than exported from
# .cue, so they are not reached by the `*/chunks/*.toml` sweep above. They are
# still #Config documents and must satisfy the schema — a pack that only the
# Rust ingest ever validated would let the two front-ends drift.
while IFS= read -r f; do vet_pos "$f" '#Config'; done < <(find "$SCENARIOS" -mindepth 2 -maxdepth 2 -name '*.json' | sort)
vet_pos "$HERE/testdata/full_surface.json" '#Config'

echo "== positive #Profile: every scenario profile =="
while IFS= read -r f; do vet_pos "$f" '#Profile'; done < <(find "$SCENARIOS" -name 'profile.toml' | sort)

echo "== division of labor: mutation fixtures are structurally valid (#Config) =="
while IFS= read -r f; do vet_pos "$f" '#Config'; done < <(find "$SCENARIOS" -path '*/mutations/*.toml' | sort)

echo "== positive: value is allowed on a COMPONENT param + trailing-_ ids (ADR-0027 D3/D4) =="
# value-not-inherited only forbids `value` on definitions; a using component
# param legitimately authors a value (resolver.rs:358 — value is authored on the
# using parameter). This must stay accepted.
accept_inline "value on a component param"   '{"package":"p","version":"1","components":{"c":{"params":{"p":{"inherits":"d","value":1}}}}}' '#Config'
# validate_snake_case (ingest_merge.rs) accepts a single trailing underscore
# (its prev_underscore guard only fires on "__"); the CUE pattern must match.
accept_inline "trailing single underscore id" '{"package":"p","version":"1","definitions":{"foo_":{"type":"string"}}}' '#Config'

# ADR-0027 Decision 6 carve-out proof (configflux-0zql) — OUTCOME B, measured.
# Decision 6 kept detect_definition_cycle (link_verify.rs) under the default-keep
# rule because B-2 never proved whether CUE rejects a cyclic `inherits` chain.
# This fixture supplies that missing evidence and resolves it to OUTCOME B: CUE
# does NOT reject an inheritance cycle. Measured with the pinned cue (//tools:cue):
#   - `cue vet -d '#Config'` on a -> b -> a            => exit 0 (ACCEPTED)
#   - the self-cycle a -> a                            => exit 0 (ACCEPTED)
#   - the real #ResolvePack whole-pack resolution path => exit 0 (resolves clean)
# Reason: ConfigFlux carries `inherits` as a STRING POINTER resolved by a single
# hop (schema.cue #ResolveParam `_defs[_child.inherits]`, no recursion into the
# parent's own `inherits`), so `a -> b -> a` is never a CUE evaluation cycle. CUE
# *would* reject it only if inheritance were dereferenced STRUCTURALLY (a control
# `parent: defs[x.inh]` deref errors "structural cycle"), which #ResolveParam
# deliberately does not do. Therefore the deterministic rejection is the Rust
# DFS guard's job: detect_definition_cycle bails "Definition inheritance cycle
# detected: a -> b -> a" (proven by compiler/src/lib_tests.rs
# ::test_link_and_verify_definition_inheritance_cycle). Asserting ACCEPT here (not
# reject) is the load-bearing point: a check_reject would FAIL, which is exactly
# why the guard stays permanent and is NOT deleted under this task (the Decision 6
# "delete only if OUTCOME A" branch is not triggered).
accept_inline "cyclic inherits a->b->a is structurally valid (cue accepts; cycle caught in Rust)" \
  '{"package":"p","version":"1","definitions":{"a":{"inherits":"b"},"b":{"inherits":"a"}}}' '#Config'

# ADR-0047: first-class facet declarations. CUE checks the shape (non-empty
# `values`, `default`/`open`/`doc` types, closed struct, #snakeId keys). The
# `default ∈ values` membership and value-uniqueness invariants are NOT
# CUE-expressible without the `list` package and are re-validated in Rust
# (link_verify::validate_facets) — so a facet whose default is not in `values`
# still PASSES cue here (accepted below) and is rejected by the compiler.
accept_inline "closed facet with default"  '{"package":"p","version":"1","facets":{"region":{"values":["eu","us","apac"],"default":"eu","doc":"deployment region"}}}' '#Config'
accept_inline "open facet, no default"      '{"package":"p","version":"1","facets":{"tls_mode":{"values":["strict"],"open":true}}}' '#Config'
accept_inline "facet default∉values passes cue (Rust rejects)" '{"package":"p","version":"1","facets":{"region":{"values":["eu","us"],"default":"mars"}}}' '#Config'

# ADR-0054: first-class `constraints` declarations. CUE checks the shape only
# (required `condition`, optional `doc`, closed struct, #snakeId keys). The
# expression itself is opaque here, exactly as a component/override `condition`
# is — CUE never interprets the condition grammar. Every semantic rule
# (the expression parses, the facets it names exist, its values are in a closed
# facet's domain) is Rust's, in link_verify::validate_constraints, so a
# constraint carrying gibberish still PASSES cue and is rejected by the compiler.
accept_inline "constraint with doc"          '{"package":"p","version":"1","constraints":{"prod_forbids_debug":{"condition":"environment != '"'"'prod'"'"' || log_level != '"'"'debug'"'"'","doc":"Debug logging is not permitted in production."}}}' '#Config'
accept_inline "constraint without doc"       '{"package":"p","version":"1","constraints":{"eu_needs_tls":{"condition":"region != '"'"'eu'"'"' || tls_mode == '"'"'strict'"'"'"}}}' '#Config'
accept_inline "unparseable constraint passes cue (Rust rejects)" '{"package":"p","version":"1","constraints":{"bogus":{"condition":"this is not <> a condition"}}}' '#Config'

# ADR-0057 §D2/§D3: first-class `catalogues` and `bindings`. CUE owns the SHAPE
# (four literal field types, #snakeId keys, closed structs); everything that
# needs to look ACROSS namespaces or compare a value against its own declared
# type is Rust's, in link_verify (`E_CATALOGUE_INVALID`, `E_BINDING_INVALID`).
# CUE validates one chunk at a time and cannot express "these keys are exactly
# those keys", so each cross-namespace fixture below is asserted to PASS here
# and is rejected by the Rust twin in compiler/src/link_verify_catalogue_tests.rs.
accept_inline "catalogue with all field metadata" '{"package":"p","version":"1","catalogues":{"containers":{"fields":{"width_mm":{"type":"integer","unit":"mm","doc":"width"}},"entries":{"c1":{"width_mm":800}}}}}' '#Config'
accept_inline "binding with default"              '{"package":"p","version":"1","bindings":{"line_container":{"catalogue":"containers","default":"c1","doc":"the line container"}}}' '#Config'
accept_inline "binding with derive table"         '{"package":"p","version":"1","bindings":{"line_container":{"catalogue":"containers","derive":{"site":{"factory_a":"c1","factory_b":"c2"}}}}}' '#Config'
accept_inline "entry missing a declared field passes cue (Rust rejects)"  '{"package":"p","version":"1","catalogues":{"containers":{"fields":{"width_mm":{"type":"integer"},"height_mm":{"type":"integer"}},"entries":{"c1":{"width_mm":800}}}}}' '#Config'
accept_inline "entry with an undeclared field passes cue (Rust rejects)"  '{"package":"p","version":"1","catalogues":{"containers":{"fields":{"width_mm":{"type":"integer"}},"entries":{"c1":{"width_mm":800,"depth_mm":400}}}}}' '#Config'
accept_inline "entry value of the wrong type passes cue (Rust rejects)"   '{"package":"p","version":"1","catalogues":{"containers":{"fields":{"width_mm":{"type":"integer"}},"entries":{"c1":{"width_mm":"wide"}}}}}' '#Config'
accept_inline "binding default not an entry passes cue (Rust rejects)"    '{"package":"p","version":"1","catalogues":{"containers":{"fields":{"w":{"type":"integer"}},"entries":{"c1":{"w":1}}}},"bindings":{"line_container":{"catalogue":"containers","default":"c9"}}}' '#Config'
accept_inline "binding default + derive passes cue (Rust rejects)"        '{"package":"p","version":"1","bindings":{"line_container":{"catalogue":"containers","default":"c1","derive":{"site":{"factory_a":"c1"}}}}}' '#Config'
accept_inline "derive key outside the source domain passes cue (Rust rejects)" '{"package":"p","version":"1","facets":{"site":{"values":["factory_a"]}},"catalogues":{"containers":{"fields":{"w":{"type":"integer"}},"entries":{"c1":{"w":1}}}},"bindings":{"line_container":{"catalogue":"containers","derive":{"site":{"factory_z":"c1"}}}}}' '#Config'
accept_inline "binding naming an unknown catalogue passes cue (Rust rejects)"  '{"package":"p","version":"1","bindings":{"line_container":{"catalogue":"nowhere"}}}' '#Config'

# ADR-0057 §D4: component `requires`. CUE owns the SHAPE — the bare form is a
# #snakeId, the explicit form is a closed struct with a required `binding` and a
# non-empty `accepts` list of #snakeId. Everything cross-namespace is Rust's, in
# link_verify::validate_requirements (`E_REQUIRES_INVALID`,
# `E_BINDING_NO_ACCEPTABLE_ENTRY`): whether the binding is declared, whether an
# accepted entry is in its catalogue, whether the list repeats an entry, and
# whether the lists of two components intersect to nothing. Each of those
# fixtures is asserted to PASS here and is rejected by the Rust twin in
# compiler/src/link_verify_requires_tests.rs.
accept_inline "bare requirement"                  '{"package":"p","version":"1","components":{"c":{"requires":{"container":"line_container"}}}}' '#Config'
accept_inline "requirement with accepts"          '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"binding":"line_container","accepts":["c1","c2"]}}}}}' '#Config'
accept_inline "two slots on one component"        '{"package":"p","version":"1","components":{"c":{"requires":{"primary":"line_container","secondary":{"binding":"sorter_container","accepts":["c3"]}}}}}' '#Config'
accept_inline "requirement naming an undeclared binding passes cue (Rust rejects)" '{"package":"p","version":"1","components":{"c":{"requires":{"container":"nowhere"}}}}' '#Config'
accept_inline "accepts entry outside the catalogue passes cue (Rust rejects)"      '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"binding":"line_container","accepts":["c9"]}}}}}' '#Config'
accept_inline "duplicate accepts entry passes cue (Rust rejects)"                  '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"binding":"line_container","accepts":["c1","c1"]}}}}}' '#Config'
accept_inline "disjoint accepts lists pass cue (Rust rejects)"                     '{"package":"p","version":"1","components":{"a":{"requires":{"container":{"binding":"line_container","accepts":["c1"]}}},"b":{"requires":{"container":{"binding":"line_container","accepts":["c2"]}}}}}' '#Config'

# ADR-0064 D1: a parameter may declare `facet: <name>`, making it that facet's
# runtime handle. CUE owns the SHAPE — the value is a #snakeId, and
# `#ConditionalBlock` closes the field off so a variant cannot rebind (the
# rejects below). Everything that needs to look ACROSS namespaces or at the
# whole model is Rust's, in link_verify::validate_facet_bindings_scoped: whether
# the facet is declared, whether the effective type is `string`, whether the
# parameter also authors a `value`, and whether a second parameter binds the
# same facet. Each of those is asserted to PASS here and to be rejected by the
# Rust twin in compiler/tests/facet_binding.rs.
accept_inline "facet binding on a component param" '{"package":"p","version":"1","facets":{"tier":{"values":["basic"]}},"components":{"c":{"params":{"h":{"type":"string","facet":"tier"}}}}}' '#Config'
accept_inline "facet binding on a definition"      '{"package":"p","version":"1","definitions":{"d":{"type":"string","facet":"tier"}}}' '#Config'
accept_inline "binding an undeclared facet passes cue (Rust rejects)"   '{"package":"p","version":"1","components":{"c":{"params":{"h":{"type":"string","facet":"nowhere"}}}}}' '#Config'
accept_inline "non-string bound param passes cue (Rust rejects)"        '{"package":"p","version":"1","components":{"c":{"params":{"h":{"type":"integer","facet":"tier"}}}}}' '#Config'
accept_inline "bound param with its own value passes cue (Rust rejects)" '{"package":"p","version":"1","components":{"c":{"params":{"h":{"type":"string","facet":"tier","value":"basic"}}}}}' '#Config'
accept_inline "two params binding one facet passes cue (Rust rejects)"  '{"package":"p","version":"1","components":{"a":{"params":{"h":{"type":"string","facet":"tier"}}},"b":{"params":{"h":{"type":"string","facet":"tier"}}}}}' '#Config'

echo "  positives: $pos ok, $pos_fail failed"

echo "== negative: malformed input must be rejected =="
check_reject "unknown field (typo)"      '{"package":"p","version":"1","components":{"c":{"params":{"x":{"valeu":1}}}}}' '#Config'
check_reject "bad safety enum"           '{"package":"p","version":"1","definitions":{"d":{"safety":"sil9"}}}' '#Config'
check_reject "bad access enum"           '{"package":"p","version":"1","definitions":{"d":{"access":"admin"}}}' '#Config'
check_reject "bad lifecycle enum"        '{"package":"p","version":"1","definitions":{"d":{"lifecycle":"boot"}}}' '#Config'
check_reject "value as object"           '{"package":"p","version":"1","definitions":{"d":{"value":{"k":1}}}}' '#Config'
check_reject "value as array"            '{"package":"p","version":"1","definitions":{"d":{"value":[1,2]}}}' '#Config'
check_reject "limits.min_len wrong type" '{"package":"p","version":"1","definitions":{"d":{"limits":{"min_len":"x"}}}}' '#Config'
check_reject "limits unknown sub-field"  '{"package":"p","version":"1","definitions":{"d":{"limits":{"step":1}}}}' '#Config'
check_reject "artifact missing name"     '{"package":"p","version":"1","artifacts":{"a":{"version":"1"}}}' '#Config'
check_reject "missing package"           '{"version":"1","definitions":{}}' '#Config'
check_reject "missing version"           '{"package":"p","definitions":{}}' '#Config'
check_reject "condition on base param"   '{"package":"p","version":"1","definitions":{"d":{"condition":"x==1"}}}' '#Config'
check_reject "depends_on non-string"     '{"package":"p","version":"1","components":{"c":{"depends_on":[1]}}}' '#Config'
check_reject "profile: domain not list"  '{"selection_domains":{"f":"x"},"default_context":{}}' '#Profile'
check_reject "profile: missing domains"  '{"default_context":{}}' '#Profile'
check_reject "profile: unknown field"    '{"selection_domains":{},"default_context":{},"bogus":1}' '#Profile'

# ADR-0027 Decision 3: definitions are valueless by construction (matches
# resolver.rs:358 — value is never inherited; it is authored on the using
# component param, never on a definition). A definition that carries `value`
# must be rejected.
check_reject "value on a definition"      '{"package":"p","version":"1","definitions":{"d":{"value":1}}}' '#Config'
check_reject "value(string) on a definition" '{"package":"p","version":"1","definitions":{"grid_profile":{"inherits":"grid_profile","value":"ieee_1547"}}}' '#Config'

# ADR-0027 Decision 4: authored-ID snake_case constraint moves to CUE and must
# reproduce validate_snake_case (ingest_merge.rs:81-104) EXACTLY, including the
# `prev_underscore` rejection of double underscores ("__"). A bare
# `^[a-z][a-z0-9_]*$` admits "__" and would silently weaken the contract.
check_reject "definition id with __"      '{"package":"p","version":"1","definitions":{"foo__bar":{"type":"string"}}}' '#Config'
check_reject "component id with __"        '{"package":"p","version":"1","components":{"foo__bar":{}}}' '#Config'
check_reject "artifact id with __"         '{"package":"p","version":"1","artifacts":{"foo__bar":{"name":"x"}}}' '#Config'
check_reject "component param key with __"  '{"package":"p","version":"1","components":{"c":{"params":{"foo__bar":{"value":1}}}}}' '#Config'
check_reject "component dep with __"        '{"package":"p","version":"1","components":{"c":{"depends_on":["foo__bar"]}}}' '#Config'
check_reject "definition id leading upper"  '{"package":"p","version":"1","definitions":{"Foo":{"type":"string"}}}' '#Config'
check_reject "definition id leading digit"  '{"package":"p","version":"1","definitions":{"1foo":{"type":"string"}}}' '#Config'

# ADR-0047: facet-shape violations CUE catches structurally. (Semantic
# invariants — default∉values, duplicate values — are Rust's job, see the
# positive block above.)
check_reject "facet empty values"          '{"package":"p","version":"1","facets":{"region":{"values":[]}}}' '#Config'
check_reject "facet missing values"        '{"package":"p","version":"1","facets":{"region":{"default":"eu"}}}' '#Config'
check_reject "facet values non-string"     '{"package":"p","version":"1","facets":{"region":{"values":[1]}}}' '#Config'
check_reject "facet open non-bool"         '{"package":"p","version":"1","facets":{"region":{"values":["eu"],"open":"yes"}}}' '#Config'
check_reject "facet unknown field"         '{"package":"p","version":"1","facets":{"region":{"values":["eu"],"bogus":1}}}' '#Config'
check_reject "facet id with __"            '{"package":"p","version":"1","facets":{"foo__bar":{"values":["x"]}}}' '#Config'

# ADR-0054: constraint-shape violations CUE catches structurally. (Semantic
# invariants — the expression parses, its facets/values exist — are Rust's, see
# the positive block above.)
check_reject "constraint missing condition" '{"package":"p","version":"1","constraints":{"c":{"doc":"no condition"}}}' '#Config'

# ADR-0057 §D4: requirement-shape violations CUE catches structurally. (The
# semantic invariants — the binding is declared, the entries exist, the lists
# intersect — are Rust's, see the positive block above.)
check_reject "requirement empty accepts"        '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"binding":"line_container","accepts":[]}}}}}' '#Config'
check_reject "requirement missing binding"      '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"accepts":["c1"]}}}}}' '#Config'
check_reject "requirement unknown field"        '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"binding":"line_container","bogus":1}}}}}' '#Config'
check_reject "requirement accepts non-string"   '{"package":"p","version":"1","components":{"c":{"requires":{"container":{"binding":"line_container","accepts":[1]}}}}}' '#Config'
check_reject "requirement binding not snake"    '{"package":"p","version":"1","components":{"c":{"requires":{"container":"Line__Container"}}}}' '#Config'
check_reject "requirement slot key with __"     '{"package":"p","version":"1","components":{"c":{"requires":{"foo__bar":"line_container"}}}}' '#Config'

# ADR-0064 D1: CUE's OWN share of the binding rules. A binding is a property of
# the parameter, not of a variant of it, so `#ConditionalBlock` closes the field
# off; and the value names a facet, so it carries the #snakeId charset CUE
# already owns. The JSON-direct path never evaluates CUE, so both are re-checked
# in Rust (compiler/tests/facet_binding.rs).
check_reject "facet inside an override"     '{"package":"p","version":"1","components":{"c":{"params":{"h":{"type":"string","facet":"tier","overrides":[{"condition":"x","facet":"site"}]}}}}}' '#Config'
check_reject "facet binding with __"        '{"package":"p","version":"1","components":{"c":{"params":{"h":{"type":"string","facet":"foo__bar"}}}}}' '#Config'
check_reject "facet binding non-string"     '{"package":"p","version":"1","components":{"c":{"params":{"h":{"type":"string","facet":1}}}}}' '#Config'
check_reject "constraint condition non-string" '{"package":"p","version":"1","constraints":{"c":{"condition":1}}}' '#Config'
check_reject "constraint doc non-string"    '{"package":"p","version":"1","constraints":{"c":{"condition":"a == '"'"'b'"'"'","doc":1}}}' '#Config'
check_reject "constraint unknown field"     '{"package":"p","version":"1","constraints":{"c":{"condition":"a == '"'"'b'"'"'","bogus":1}}}' '#Config'
check_reject "constraint id with __"        '{"package":"p","version":"1","constraints":{"foo__bar":{"condition":"a == '"'"'b'"'"'"}}}' '#Config'

# ADR-0057 §D2/§D3: catalogue/binding SHAPE violations CUE catches
# structurally. (The semantic invariants — entry completeness, type
# agreement, catalogue existence, default/derive membership — are Rust's;
# see the accept_inline block above.)
check_reject "catalogue missing fields" '{"package":"p","version":"1","catalogues":{"c":{"entries":{"e":{"w":1}}}}}' '#Config'
check_reject "catalogue missing entries" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"type":"integer"}}}}}' '#Config'
check_reject "catalogue field unknown type" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"type":"decimal"}},"entries":{"e":{"w":1}}}}}' '#Config'
check_reject "catalogue field missing type" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"unit":"mm"}},"entries":{"e":{"w":1}}}}}' '#Config'
check_reject "catalogue field unknown key" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"type":"integer","bogus":1}},"entries":{"e":{"w":1}}}}}' '#Config'
check_reject "catalogue unknown top-level key" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"type":"integer"}},"entries":{"e":{"w":1}},"bogus":1}}}' '#Config'
check_reject "catalogue id with __" '{"package":"p","version":"1","catalogues":{"foo__bar":{"fields":{"w":{"type":"integer"}},"entries":{"e":{"w":1}}}}}' '#Config'
check_reject "catalogue entry id not snake_case" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"type":"integer"}},"entries":{"C1":{"w":1}}}}}' '#Config'
check_reject "catalogue entry value is a list" '{"package":"p","version":"1","catalogues":{"c":{"fields":{"w":{"type":"integer"}},"entries":{"e":{"w":[1]}}}}}' '#Config'
check_reject "binding missing catalogue" '{"package":"p","version":"1","bindings":{"b":{"default":"c1"}}}' '#Config'
check_reject "binding catalogue not snake_case" '{"package":"p","version":"1","bindings":{"b":{"catalogue":"Containers"}}}' '#Config'
check_reject "binding unknown field" '{"package":"p","version":"1","bindings":{"b":{"catalogue":"c","bogus":1}}}' '#Config'
check_reject "binding derive entry not snake_case" '{"package":"p","version":"1","bindings":{"b":{"catalogue":"c","derive":{"site":{"factory_a":"C1"}}}}}' '#Config'
check_reject "binding derive source not snake_case" '{"package":"p","version":"1","bindings":{"b":{"catalogue":"c","derive":{"Site":{"factory_a":"c1"}}}}}' '#Config'
check_reject "binding id with __" '{"package":"p","version":"1","bindings":{"foo__bar":{"catalogue":"c"}}}' '#Config'

echo "== summary =="
if [ "$pos_fail" -eq 0 ] && [ "$neg_fail" -eq 0 ]; then
  echo "OK: $pos positive fixtures valid, $neg_ok negatives rejected"
else
  echo "FAILED: $pos_fail positive failures, $neg_fail negatives not rejected"
  exit 1
fi
