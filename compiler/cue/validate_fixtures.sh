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

echo "== summary =="
if [ "$pos_fail" -eq 0 ] && [ "$neg_fail" -eq 0 ]; then
  echo "OK: $pos positive fixtures valid, $neg_ok negatives rejected"
else
  echo "FAILED: $pos_fail positive failures, $neg_fail negatives not rejected"
  exit 1
fi
