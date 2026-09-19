#!/usr/bin/env bash
#
# Export authored CUE scenario chunks to their sibling .json fixtures using the
# pinned cue binary, or (--check) verify the committed .json still matches a
# fresh export. Part of the CUE authoring front-end (ADR 0021 phase 6,
# configflux-et7t; inheritance-resolved emission added under ADR 0027 Track B,
# configflux-fsql).
#
# Coverage (configflux-a3cb): in addition to the scenario corpus this script
# also drift-checks the worked examples under examples/* (ADR 0027 E-5,
# configflux-bmgy), which were migrated from TOML+inherits to CUE authoring.
# See the "examples/* coverage" block near the bottom for the layout contract
# and why the example comparison is canonicalized rather than byte-exact.
#
# Layout contract: each authored chunk lives at
#   compiler/scenarios/<scenario>/<size>/cue/<NN_name>.cue
# is `package configflux`, and defines a top-level `chunk: #Config & {...}`.
# A pack is the pair of chunks in one cue/ directory:
#   00_definitions.cue  (the inheritance roots)
#   10_components.cue   (components + artifacts; component params inherit)
#
# Resolution model (ADR 0027 decision 2 — "resolution whole-pack, attribution
# per-file"). CUE now OWNS `inherits` resolution (decisions 1-3): a component
# param's `inherits` pointer is gap-filled against the sibling definitions chunk
# during WHOLE-PACK evaluation, exactly reproducing resolver.rs::apply_inheritance
# (fill type/unit/safety/lifecycle/access/limits/doc; never push a parent value
# down). Emission then stays per-file: each chunk's .json contains ONLY the
# entities authored in that file, so the one-entity-one-chunk CMP provenance
# contract (ingest_merge.rs::build_ir_index) is preserved and no parent field
# bleeds into the wrong chunk. The proof is the B-4 equivalence corpus
# (scenario_cue_equivalence_tests.rs): any cross-file leak changes chunk bytes.
#
# Mechanism: both chunks are exported raw (`cue export -e chunk`), wrapped into
# distinct fields of a packaged driver, run through `#ResolvePack` from
# schema.cue, then the per-file slice is emitted:
#   00_definitions.json <- {package, version, definitions}   (roots, pass-through)
#   10_components.json  <- {package, version, [artifacts,] components}  (resolved)
# Wrapping each raw chunk into its own packaged .cue file (rather than using
# `cue export -l`, whose path label is cumulative across files and would nest the
# second data file under the first) keeps the two chunks under independent paths
# in a single evaluation.
#
# The committed .json is what the Rust differential gate
# (compiler/src/scenario_cue_equivalence_tests.rs) ingests; --check is wired
# into CI (.github/workflows/cue-schema.yml) so the fixture cannot silently
# diverge from its CUE source.
#
# Requires the pinned `cue` evaluator. By default this resolves the hermetic,
# Bazel-pinned cue (ADR-0021 Decision 5) via //tools:cue_binary — there is no
# silent $PATH fallback (an unpinned evaluator can change model_hash, ADR-0027).
# Set $CUE to opt into a specific binary, e.g. in CI:
#   CUE=/path/to/cue compiler/cue/export_fixtures.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
# shellcheck source=tools/lib/resolve_cue.sh
source "$REPO/tools/lib/resolve_cue.sh"
CUE="$(configflux_resolve_cue)" || exit $?
SCHEMA="$HERE/schema.cue"
SCENARIOS="$REPO/compiler/scenarios"
EXAMPLES="$REPO/examples"

mode="write"
[ "${1:-}" = "--check" ] && mode="check"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Driver that resolves a whole pack and exposes per-file slices. defsIn/compsIn
# are injected by the wrapped chunk files below. Emitted via `-e definitionsOut`
# / `-e componentsOut` so only the originating file's entities land in its .json.
cat >"$tmp/driver.cue" <<'DRIVER'
package configflux

_resolved: #ResolvePack & {
	_definitions: defsIn.definitions
	_components:  compsIn.components
}

// Definitions-file slice: the inheritance roots, passed through unchanged
// (#ResolvePack.definitions is _definitions verbatim). Carry artifacts/components
// only if the definitions chunk authored them (the corpus never does, but keep
// the slice faithful to whatever the chunk declared).
definitionsOut: {
	package: defsIn.package
	version: defsIn.version
	if defsIn.definitions != _|_ {definitions: defsIn.definitions}
	if defsIn.components != _|_ {components: defsIn.components}
	if defsIn.artifacts != _|_ {artifacts: defsIn.artifacts}
	// Facets pass through verbatim (ADR-0047). Convention places them in the
	// 00_definitions chunk, but carry them from whichever chunk authored them.
	if defsIn.facets != _|_ {facets: defsIn.facets}
	// Constraints likewise pass through verbatim (ADR-0054 §1) — a proposition
	// has nothing to gap-fill. Same convention (00_definitions) and same
	// whichever-chunk-authored-it rule as facets.
	if defsIn.constraints != _|_ {constraints: defsIn.constraints}
	// Catalogues and bindings, likewise (ADR-0057 §D2/§D3): a typed table has
	// no inheritable shape and a binding is a declared facet, so both ride the
	// same verbatim path facets and constraints take.
	if defsIn.catalogues != _|_ {catalogues: defsIn.catalogues}
	if defsIn.bindings != _|_ {bindings: defsIn.bindings}
}

// Components-file slice: resolved components (inheritance gap-filled) plus this
// chunk's own artifacts. No definitions leak in — attribution stays per-file.
componentsOut: {
	package: compsIn.package
	version: compsIn.version
	if compsIn.artifacts != _|_ {artifacts: compsIn.artifacts}
	if compsIn.components != _|_ {components: _resolved.components}
	if compsIn.facets != _|_ {facets: compsIn.facets}
	if compsIn.constraints != _|_ {constraints: compsIn.constraints}
	if compsIn.catalogues != _|_ {catalogues: compsIn.catalogues}
	if compsIn.bindings != _|_ {bindings: compsIn.bindings}
}
DRIVER

# Single-file driver for the examples/* layout where one cue/ chunk authors
# BOTH definitions and components (e.g. examples/01-hello-led/cue/config.cue ->
# config.json). The same #ResolvePack whole-pack resolution runs, but the slice
# is the entire config (no per-file attribution split). srcIn is injected by the
# wrapped chunk file below.
cat >"$tmp/driver_single.cue" <<'DRIVER'
package configflux

_resolved: #ResolvePack & {
	_definitions: srcIn.definitions
	_components:  srcIn.components
}

configOut: {
	package: srcIn.package
	version: srcIn.version
	if srcIn.definitions != _|_ {definitions: srcIn.definitions}
	if srcIn.artifacts != _|_ {artifacts: srcIn.artifacts}
	if srcIn.components != _|_ {components: _resolved.components}
	if srcIn.facets != _|_ {facets: srcIn.facets}
	if srcIn.constraints != _|_ {constraints: srcIn.constraints}
	if srcIn.catalogues != _|_ {catalogues: srcIn.catalogues}
	if srcIn.bindings != _|_ {bindings: srcIn.bindings}
}
DRIVER

# Emit one resolved slice. $1=raw defs json, $2=raw comps json, $3=expr to emit
# (definitionsOut|componentsOut). Writes canonical JSON to stdout.
emit_slice() {
	local defs_json="$1" comps_json="$2" expr="$3"
	{ echo 'package configflux'; printf 'defsIn: '; cat "$defs_json"; } >"$tmp/defs_wrap.cue"
	{ echo 'package configflux'; printf 'compsIn: '; cat "$comps_json"; } >"$tmp/comps_wrap.cue"
	"$CUE" export "$tmp/driver.cue" "$tmp/defs_wrap.cue" "$tmp/comps_wrap.cue" "$SCHEMA" \
		-e "$expr" --out json
}

# Emit the whole resolved config for a single-file example chunk. $1=raw chunk
# json (defs+comps in one). Writes canonical JSON to stdout.
emit_single() {
	local src_json="$1"
	{ echo 'package configflux'; printf 'srcIn: '; cat "$src_json"; } >"$tmp/src_wrap.cue"
	"$CUE" export "$tmp/driver_single.cue" "$tmp/src_wrap.cue" "$SCHEMA" \
		-e configOut --out json
}

# Canonicalize a JSON file for the examples/* comparison (see compare_or_write's
# "canon" mode): parse it and re-emit with recursively-sorted object keys via
# `jq -S`. Key-sorting is the whole point — the committed example JSON uses
# serde struct field order (e.g. ...,lifecycle,access,doc) while a fresh
# `cue export` uses cue's evaluation order (...,doc,lifecycle,access). Neither
# `cue export` nor a raw byte diff normalizes key order, so jq is required here.
# jq parses JSON natively, so example condition strings containing single quotes
# (e.g. "motor_class == 'brushed_dc'") are handled correctly.
canon_json() {
	jq -S . "$1"
}

# Compare a freshly-emitted slice against the committed copy (check mode) or
# write it (write mode). $1=committed json path, $2=fresh emitted json path,
# $3=comparison mode (raw|canon).
#   raw   — byte-for-byte `diff` (scenario corpus: committed JSON IS the raw
#           `cue export` output, so it must match exactly).
#   canon — compare both sides after re-canonicalizing through the pinned cue
#           binary (examples/*: their committed JSON was emitted by a different
#           pipeline — 2-space, serde struct key order — so a byte diff would
#           flag pure formatting. Canonicalizing both sides still fails loudly
#           on any real content drift: an added/removed/changed key or value).
# Mutates the outer `drift` and `count` counters.
compare_or_write() {
	local json_file="$1" out_json="$2" cmp="${3:-raw}"
	local rel="${json_file#"$REPO"/}"
	if [ "$mode" = "check" ]; then
		if [ ! -f "$json_file" ]; then
			echo "  MISSING committed fixture: $rel"
			drift=$((drift + 1))
		elif [ "$cmp" = "canon" ]; then
			if ! diff -u <(canon_json "$json_file") <(canon_json "$out_json") >/dev/null 2>&1; then
				echo "  DRIFT: $rel differs from a fresh cue export (canonicalized)"
				diff -u <(canon_json "$json_file") <(canon_json "$out_json") | sed 's/^/    /' || true
				drift=$((drift + 1))
			fi
		elif ! diff -u "$json_file" "$out_json" >/dev/null 2>&1; then
			echo "  DRIFT: $rel differs from a fresh cue export"
			diff -u "$json_file" "$out_json" | sed 's/^/    /' || true
			drift=$((drift + 1))
		fi
	else
		cp "$out_json" "$json_file"
		echo "  wrote $rel"
	fi
	count=$((count + 1))
}

# Process one scenario .json fixture: re-derive it from the whole-pack
# resolution and either write it or byte-diff it against the committed copy.
process_fixture() {
	local json_file="$1" defs_raw="$2" comps_raw="$3" expr="$4"
	emit_slice "$defs_raw" "$comps_raw" "$expr" >"$tmp/out.json"
	compare_or_write "$json_file" "$tmp/out.json" raw
}

count=0
drift=0
# Iterate per pack (each cue/ directory holding 00_definitions.cue). The packs
# are discovered, not hard-coded, so adding a scenario size needs no edit here.
while IFS= read -r defs_cue; do
	pack_dir="$(dirname "$defs_cue")"
	comps_cue="$pack_dir/10_components.cue"
	if [ ! -f "$comps_cue" ]; then
		echo "ERROR: $pack_dir has 00_definitions.cue but no 10_components.cue" >&2
		exit 2
	fi
	# Raw (unresolved) chunk data for both files in this pack.
	"$CUE" export -e chunk "$defs_cue" "$SCHEMA" --out json >"$tmp/defs_raw.json"
	"$CUE" export -e chunk "$comps_cue" "$SCHEMA" --out json >"$tmp/comps_raw.json"

	process_fixture "${defs_cue%.cue}.json"  "$tmp/defs_raw.json" "$tmp/comps_raw.json" definitionsOut
	process_fixture "${comps_cue%.cue}.json" "$tmp/defs_raw.json" "$tmp/comps_raw.json" componentsOut
done < <(find "$SCENARIOS" -path '*/cue/00_definitions.cue' | sort)

# ---------------------------------------------------------------------------
# examples/* coverage (configflux-a3cb)
# ---------------------------------------------------------------------------
# The worked examples under examples/* were migrated to CUE authoring in E-5
# (ADR 0027, configflux-bmgy). Each example dir holds hand-authored CUE under
# cue/ and the committed resolved JSON that the compiler ingests, but the JSON
# lands in the EXAMPLE ROOT (one level up from cue/), not beside the .cue:
#
#   examples/01-hello-led/cue/config.cue           -> examples/01-hello-led/config.json
#   examples/02-sensor-gateway/cue/00_definitions.cue -> .../00_definitions.json
#   examples/02-sensor-gateway/cue/10_components.cue  -> .../10_components.json
#
# Two layouts occur and both are discovered, not hard-coded:
#   - PACK   (00_definitions.cue + 10_components.cue): identical resolution to
#            the scenario corpus; emit the two per-file slices.
#   - SINGLE (one chunk authoring defs+components together, e.g. config.cue):
#            resolve the whole chunk and emit the entire config.
#
# The sweep is depth-limited to `examples/*/cue` because that pair of layouts is
# the whole contract here: a cue/ directory one level below an example root,
# with its committed JSON in that root. An example whose chunks are NESTED
# deeper — examples/06-catalogue-polyrepo/repos/<repo>/cue, the multi-repository
# pack, whose N chunks resolve against one shared definitions chunk and are
# emitted by examples/export_pack.sh — fits neither layout and must not be
# force-fitted into one. Without the depth limit it would be read as SINGLE and
# demand a repos/<repo>/config.json that does not and should not exist. Its
# drift guard is //examples:export_pack_test, which compares BYTE-for-byte
# against a fresh export_pack.sh run (configflux-dkmm.6).
#
# Comparison is "canon" (not byte-exact): the committed example JSON was emitted
# by a different pipeline than raw `cue export` (2-space indent, serde struct
# key order), so canonicalizing both sides with `jq -S` is what lets the check
# fail loudly on real CUE<->JSON drift without tripping on pure formatting.
#
# Examples are CHECK-ONLY: this script never WRITES example JSON. The example
# JSON format is owned by the E-5 export pipeline (compiler-canonical 2-space),
# not by this script's raw-cue-export writer, so regenerating it here would
# reformat the user-facing fixtures. In write mode the examples pass is skipped;
# to refresh an example, re-run its authoring export and commit the result.
if [ -d "$EXAMPLES" ] && [ "$mode" = "check" ]; then
	command -v jq >/dev/null 2>&1 || { echo "ERROR: jq not found (required to canonicalize examples/* JSON)"; exit 2; }
	while IFS= read -r ex_cue_dir; do
		ex_root="$(dirname "$ex_cue_dir")"
		defs_cue="$ex_cue_dir/00_definitions.cue"
		comps_cue="$ex_cue_dir/10_components.cue"
		if [ -f "$defs_cue" ]; then
			# PACK layout — mirror the scenario emit, but JSON lives in ex_root.
			if [ ! -f "$comps_cue" ]; then
				echo "ERROR: $ex_cue_dir has 00_definitions.cue but no 10_components.cue" >&2
				exit 2
			fi
			"$CUE" export -e chunk "$defs_cue" "$SCHEMA" --out json >"$tmp/defs_raw.json"
			"$CUE" export -e chunk "$comps_cue" "$SCHEMA" --out json >"$tmp/comps_raw.json"
			emit_slice "$tmp/defs_raw.json" "$tmp/comps_raw.json" definitionsOut >"$tmp/out.json"
			compare_or_write "$ex_root/00_definitions.json" "$tmp/out.json" canon
			emit_slice "$tmp/defs_raw.json" "$tmp/comps_raw.json" componentsOut >"$tmp/out.json"
			compare_or_write "$ex_root/10_components.json" "$tmp/out.json" canon
		else
			# SINGLE layout — exactly one *.cue chunk authoring defs+components.
			single_cue="$(find "$ex_cue_dir" -maxdepth 1 -name '*.cue' | sort | head -1)"
			if [ -z "$single_cue" ]; then
				continue
			fi
			"$CUE" export -e chunk "$single_cue" "$SCHEMA" --out json >"$tmp/src_raw.json"
			emit_single "$tmp/src_raw.json" >"$tmp/out.json"
			compare_or_write "$ex_root/config.json" "$tmp/out.json" canon
		fi
	done < <(find "$EXAMPLES" -mindepth 2 -maxdepth 2 -type d -name cue | sort)
fi

echo "== $count chunk(s) processed =="
if [ "$mode" = "check" ]; then
	if [ "$drift" -ne 0 ]; then
		echo "FAILED: $drift fixture(s) out of sync — run compiler/cue/export_fixtures.sh"
		exit 1
	fi
	echo "OK: all committed JSON fixtures match a fresh cue export"
fi
