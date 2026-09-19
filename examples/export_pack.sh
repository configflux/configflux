#!/usr/bin/env bash
# export_pack.sh — reference exporter for a ConfigFlux CUE pack.
#
# The compiler ingests JSON, not CUE, and it ingests the INHERITANCE-RESOLVED
# JSON: every `inherits` pointer already gap-filled with its definition's
# type/unit/safety/lifecycle/access/limits/doc. Resolution is a whole-pack
# operation (`#ResolvePack` in compiler/cue/schema.cue) while emission stays
# per-file, so each chunk's .json contains only the entities that chunk
# authored. This script performs both halves for a pack of ANY size.
#
# It generalizes the two-file recipe: a pack has exactly ONE definitions chunk
# (the inheritance roots) and ANY NUMBER of components chunks, which may live
# in different directories — or in different repositories. Every components
# chunk is resolved against the SAME definitions map, so a component parameter
# in one repository can inherit a definition declared in another.
#
# Usage:
#   export_pack.sh --schema <schema.cue> --out <dir> \
#       --definitions <00_defs.cue> [--components <a.cue> [--components <b.cue> ...]]
#   export_pack.sh --schema <schema.cue> --out <dir> --single <config.cue>
#
# Options:
#   --schema <path>       compiler/cue/schema.cue (supplies #Config and
#                         #ResolvePack). Required.
#   --definitions <path>  The pack's definitions chunk. Required unless
#                         --single is used.
#   --components <path>   A components chunk. Repeatable; may be omitted when
#                         the definitions chunk authors the whole model.
#   --single <path>       One chunk authoring definitions AND components
#                         together (the single-file pack). Mutually exclusive
#                         with --definitions/--components.
#   --out <dir>           Directory to write the resolved JSON into. Required;
#                         created if absent.
#
# Output: one resolved JSON per input chunk, named after the input file --
#   00_defs.cue -> <out>/00_defs.json,  10_vision.cue -> <out>/10_vision.json.
# The definitions chunk emits its definitions, facets and constraints
# unchanged (they are already the inheritance roots) plus its own components
# resolved, if it authored any. Each components chunk emits its resolved
# components plus its own artifacts, facets and constraints. Nothing leaks
# across chunks, so provenance stays per-file.
#
# Determinism: the same inputs produce byte-identical outputs. `cue export`
# emits canonical JSON and this script adds no timestamp, path or ordering of
# its own, so the committed JSON can be drift-checked with `cmp`.
#
# Locating cue: inside this repository the pinned evaluator is resolved through
# tools/lib/resolve_cue.sh (ADR-0021 Decision 5), which honours $CUE with a
# version assert and otherwise materializes the hermetic Bazel-pinned binary.
# Outside this repository, set $CUE to the cue binary you trust. There is NO
# silent $PATH fallback -- an unpinned evaluator can change model_hash
# (ADR-0027), so an unresolvable evaluator is an error, not a warning.
#
# Exit status:
#   0  every chunk was exported and resolved
#   1  cue rejected a chunk (schema violation, unreadable file, bad reference);
#      cue's own stderr is forwarded unchanged
#   2  usage error, or the pinned cue evaluator could not be resolved
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

EXIT_OK=0
EXIT_EXPORT=1
EXIT_USAGE=2

usage() {
  sed -n '2,58p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

die_usage() {
  echo "ERROR: $1" >&2
  echo "" >&2
  usage >&2
  exit "${EXIT_USAGE}"
}

# ---------------------------------------------------------------------------
# Locate the cue evaluator (see the header note above).
# ---------------------------------------------------------------------------
resolve_cue_binary() {
  # (1) $CUE is the documented, explicit opt-in and wins. Outside this
  #     repository it is the only way to name an evaluator.
  if [[ -n "${CUE:-}" ]] && { [[ -x "${CUE}" ]] || command -v "${CUE}" >/dev/null 2>&1; }; then
    printf '%s\n' "${CUE}"
    return 0
  fi
  # (2) Inside this repository, fall back to the hermetic Bazel-pinned binary.
  local repo_lib="${SCRIPT_DIR}/../tools/lib/resolve_cue.sh"
  if [[ -f "${repo_lib}" ]]; then
    # shellcheck source=tools/lib/resolve_cue.sh
    source "${repo_lib}"
    configflux_resolve_cue
    return $?
  fi
  # (3) Neither: fail closed rather than reaching for a $PATH cue.
  cat >&2 <<'EOF'
ERROR: could not resolve a cue evaluator.
  Set $CUE to the pinned cue binary you author this pack with, e.g.
    CUE=/path/to/cue export_pack.sh --schema ... --out ...
  There is deliberately no $PATH fallback: an unpinned evaluator can change
  the compiled model_hash.
EOF
  return "${EXIT_USAGE}"
}

# ---------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------
SCHEMA=""
DEFS_CUE=""
SINGLE_CUE=""
OUT_DIR=""
COMPONENT_CUES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --schema)      [[ $# -ge 2 ]] || die_usage "--schema needs a value";      SCHEMA="$2"; shift 2 ;;
    --definitions) [[ $# -ge 2 ]] || die_usage "--definitions needs a value"; DEFS_CUE="$2"; shift 2 ;;
    --components)  [[ $# -ge 2 ]] || die_usage "--components needs a value";  COMPONENT_CUES+=("$2"); shift 2 ;;
    --single)      [[ $# -ge 2 ]] || die_usage "--single needs a value";      SINGLE_CUE="$2"; shift 2 ;;
    --out)         [[ $# -ge 2 ]] || die_usage "--out needs a value";         OUT_DIR="$2"; shift 2 ;;
    -h|--help)     usage; exit "${EXIT_USAGE}" ;;
    *)             die_usage "unknown argument: $1" ;;
  esac
done

[[ -n "${SCHEMA}" ]]  || die_usage "--schema is required"
[[ -n "${OUT_DIR}" ]] || die_usage "--out is required"
[[ -f "${SCHEMA}" ]]  || die_usage "--schema '${SCHEMA}' is not a readable file"

if [[ -n "${SINGLE_CUE}" ]]; then
  if [[ -n "${DEFS_CUE}" || ${#COMPONENT_CUES[@]} -gt 0 ]]; then
    die_usage "--single is mutually exclusive with --definitions/--components"
  fi
else
  [[ -n "${DEFS_CUE}" ]] || die_usage "--definitions is required (or use --single)"
fi

# Every chunk this run will emit, in output order. The definitions chunk (or
# the single chunk) is index 0; components chunks follow in the order given.
CHUNKS=()
if [[ -n "${SINGLE_CUE}" ]]; then
  CHUNKS+=("${SINGLE_CUE}")
else
  CHUNKS+=("${DEFS_CUE}")
  [[ ${#COMPONENT_CUES[@]} -gt 0 ]] && CHUNKS+=("${COMPONENT_CUES[@]}")
fi

for chunk in "${CHUNKS[@]}"; do
  [[ -f "${chunk}" ]] || die_usage "chunk '${chunk}' is not a readable file"
done

# Outputs are named after the INPUT BASENAME, so two chunks sharing a basename
# would silently overwrite each other. Refuse instead: a pack that emits fewer
# files than it has chunks is a pack the compiler will ingest incompletely.
declare -A seen_basenames=()
for chunk in "${CHUNKS[@]}"; do
  base="$(basename "${chunk}")"
  base="${base%.cue}"
  if [[ -n "${seen_basenames[${base}]:-}" ]]; then
    die_usage "two chunks share the basename '${base}.cue' ('${seen_basenames[${base}]}' and '${chunk}'); outputs would collide"
  fi
  seen_basenames["${base}"]="${chunk}"
done

CUE_BIN="$(resolve_cue_binary)" || exit $?

mkdir -p "${OUT_DIR}"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# ---------------------------------------------------------------------------
# Step 1: export every chunk RAW (un-gap-filled).
#
# Each chunk file binds a top-level `chunk`, so they cannot be evaluated
# together as-is. Exporting each one raw first lets the generated driver place
# them under distinct fields (defsIn, chunk1In, ...) in a single resolving
# evaluation.
# ---------------------------------------------------------------------------
raw_field() { [[ "$1" -eq 0 ]] && printf 'defsIn' || printf 'chunk%sIn' "$1"; }

for i in "${!CHUNKS[@]}"; do
  if ! "${CUE_BIN}" export -e chunk "${CHUNKS[$i]}" "${SCHEMA}" --out json \
      >"${tmp}/raw_${i}.json"; then
    echo "ERROR: cue could not export '${CHUNKS[$i]}'" >&2
    exit "${EXIT_EXPORT}"
  fi
  {
    echo 'package configflux'
    printf '%s: ' "$(raw_field "$i")"
    cat "${tmp}/raw_${i}.json"
  } >"${tmp}/wrap_${i}.cue"
done

# ---------------------------------------------------------------------------
# Step 2: generate the driver.
#
# ONE definitions map (`_defs`) feeds a separate #ResolvePack per chunk, so a
# component parameter in any chunk -- in any directory, in any repository --
# gap-fills against the pack's single set of inheritance roots. Each chunk then
# gets its own emission slice (`out<i>`) carrying only what that chunk
# authored: definitions, facets, constraints, catalogues and bindings pass
# through verbatim, components come from that chunk's own resolution.
# ---------------------------------------------------------------------------
{
  echo 'package configflux'
  echo ''
  echo '_defs: {'
  printf '\tif defsIn.definitions != _|_ {defsIn.definitions}\n'
  printf '\tif defsIn.definitions == _|_ {}\n'
  echo '}'
  for i in "${!CHUNKS[@]}"; do
    field="$(raw_field "$i")"
    printf '\n_resolved%s: #ResolvePack & {\n' "$i"
    printf '\t_definitions: _defs\n'
    printf '\tif %s.components != _|_ {_components: %s.components}\n' "${field}" "${field}"
    printf '\tif %s.components == _|_ {_components: {}}\n' "${field}"
    printf '}\n'
    printf '\nout%s: {\n' "$i"
    printf '\tpackage: %s.package\n' "${field}"
    printf '\tversion: %s.version\n' "${field}"
    printf '\tif %s.definitions != _|_ {definitions: %s.definitions}\n' "${field}" "${field}"
    printf '\tif %s.artifacts != _|_ {artifacts: %s.artifacts}\n' "${field}" "${field}"
    printf '\tif %s.components != _|_ {components: _resolved%s.components}\n' "${field}" "$i"
    printf '\tif %s.facets != _|_ {facets: %s.facets}\n' "${field}" "${field}"
    printf '\tif %s.constraints != _|_ {constraints: %s.constraints}\n' "${field}" "${field}"
    printf '\tif %s.catalogues != _|_ {catalogues: %s.catalogues}\n' "${field}" "${field}"
    printf '\tif %s.bindings != _|_ {bindings: %s.bindings}\n' "${field}" "${field}"
    printf '}\n'
  done
} >"${tmp}/driver.cue"

# ---------------------------------------------------------------------------
# Step 3: emit one resolved slice per chunk.
# ---------------------------------------------------------------------------
wraps=()
for i in "${!CHUNKS[@]}"; do
  wraps+=("${tmp}/wrap_${i}.cue")
done

for i in "${!CHUNKS[@]}"; do
  base="$(basename "${CHUNKS[$i]}")"
  target="${OUT_DIR}/${base%.cue}.json"
  if ! "${CUE_BIN}" export "${tmp}/driver.cue" "${wraps[@]}" "${SCHEMA}" \
      -e "out${i}" --out json >"${tmp}/out_${i}.json"; then
    echo "ERROR: cue could not resolve '${CHUNKS[$i]}' against the pack" >&2
    exit "${EXIT_EXPORT}"
  fi
  mv "${tmp}/out_${i}.json" "${target}"
  echo "  wrote ${target}"
done

echo "== ${#CHUNKS[@]} chunk(s) exported into ${OUT_DIR} =="
exit "${EXIT_OK}"
