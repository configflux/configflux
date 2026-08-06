#!/usr/bin/env bash
# resolve_environment.sh — reference resolver for a ConfigFlux environment
# manifest.
#
# An environment manifest is a small, versioned JSON file the user owns. It
# names deployment targets, each bound to the three free fields of a
# resolution: a scope, a set of context tags, and a set of selection choices
# (see docs/service-integration-guide.md). The manifest is NOT a product input
# format — the ConfigFlux binaries keep accepting only their existing
# per-invocation stdin envelopes. This script "desugars" one named environment
# into the existing interpreter verb chain and writes one resolved snapshot:
#
#   open  ->  init-selection-state  ->  select x N  ->  resolve
#
# The manifest shape (the convention fixes the field names; the user owns the
# file):
#
#   {
#     "schema_version": 3,
#     "environments": {
#       "<name>": {
#         "scope": "component:<name>" | "all",
#         "context_tags": { "<tag>": "<value>" },
#         "choices":      { "<facet>": "<option>" }
#       }
#     }
#   }
#
# context_tags feed the init-selection-state request; each choices entry
# {facet: option} becomes one `select` delta; the scope flows through every
# request. This is exactly how a resolution is assembled by hand — this script
# generalizes it from the manifest.
#
# Matrix mode resolves N environments x M scopes: it iterates the manifest's
# environments and, for each, the scopes to resolve (the environment's own
# scope by default, or every scope passed with --scopes), invoking the chain
# once per cell and collecting one snapshot per cell. The matrix is N*M
# independent, deterministic resolves with no cross-cell state — a documented
# loop, not a new product verb.
#
# Snapshots follow the resolve_result.<root>.<selection>.json naming
# convention, where <root> is the scope root and <selection> is a deterministic
# label derived from the (sorted) choices, falling back to the environment name
# when an environment has no choices.
#
# Usage:
#   resolve_environment.sh --cmp <cmp.manifest.json> --manifest <env.json> \
#       --environment <name> [--out <dir>]
#   resolve_environment.sh --cmp <cmp.manifest.json> --manifest <env.json> \
#       --matrix [--scopes <scope[,scope...]>] [--out <dir>]
#
# Options:
#   --cmp <path>          Path to a compiled-model manifest (cmp.manifest.json)
#                         produced by `configflux-compiler compile`.
#   --manifest <path>     Path to the environment manifest JSON.
#   --environment <name>  Resolve exactly one named environment.
#   --matrix              Resolve every environment in the manifest.
#   --scopes <list>       Comma-separated scopes to resolve per environment in
#                         matrix mode (default: each environment's own scope).
#   --out <dir>           Output directory (default: ./out). In matrix mode,
#                         each cell is written under <dir>/<environment>/.
#
# Exit status:
#   0  the requested resolution(s) completed and a snapshot was written per cell
#   1  a resolution was rejected, or the manifest/environment was malformed
#   2  usage error (wrong arguments, or a required tool is unavailable)
#
# Dependencies: bash, jq, and the ConfigFlux interpreter binary. The interpreter
# is located via CONFIGFLUX_INTERPRETER, falling back to the bazel-built binary
# at bazel-bin/interpreter/interpreter (same convention as the example run.sh
# scripts). No new CLI verb or flag is used — only open, init-selection-state,
# select, and resolve.
set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
usage() {
  cat >&2 <<'USAGE'
Usage:
  resolve_environment.sh --cmp <cmp.manifest.json> --manifest <env.json> \
      --environment <name> [--out <dir>]
  resolve_environment.sh --cmp <cmp.manifest.json> --manifest <env.json> \
      --matrix [--scopes <scope[,scope...]>] [--out <dir>]

Desugars a named environment from an environment manifest into the existing
interpreter verb chain (open -> init-selection-state -> select x N -> resolve)
and writes one resolved snapshot per cell.
USAGE
}

reject() { echo "FAIL: $*" >&2; exit 1; }

need_tool() {
  local tool="$1"
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "ERROR: required tool '${tool}' not found on PATH" >&2
    exit 2
  fi
}

# Locate the interpreter the same way the example run.sh scripts do: an
# explicit override via CONFIGFLUX_INTERPRETER, else the bazel-built binary
# relative to the repository root.
find_interpreter() {
  if [[ -n "${CONFIGFLUX_INTERPRETER:-}" ]]; then
    echo "${CONFIGFLUX_INTERPRETER}"
    return
  fi
  local script_dir repo_root candidate
  script_dir="$(cd "$(dirname "$0")" && pwd)"
  repo_root="$(cd "${script_dir}/.." && pwd)"
  candidate="${repo_root}/bazel-bin/interpreter/interpreter"
  if [[ -x "${candidate}" ]]; then
    echo "${candidate}"
    return
  fi
  echo >&2 "ERROR: interpreter binary not found."
  echo >&2 "  Either build it:  bazel build //interpreter"
  echo >&2 "  Or set:           export CONFIGFLUX_INTERPRETER=/path/to/interpreter"
  exit 2
}

# ---------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------
CMP=""
MANIFEST=""
ENVIRONMENT=""
MATRIX=0
SCOPES_OVERRIDE=""
OUT_DIR="./out"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cmp)         CMP="${2:-}"; shift 2 ;;
    --manifest)    MANIFEST="${2:-}"; shift 2 ;;
    --environment) ENVIRONMENT="${2:-}"; shift 2 ;;
    --matrix)      MATRIX=1; shift ;;
    --scopes)      SCOPES_OVERRIDE="${2:-}"; shift 2 ;;
    --out)         OUT_DIR="${2:-}"; shift 2 ;;
    -h|--help)     usage; exit 2 ;;
    *)             echo "ERROR: unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "${CMP}" || -z "${MANIFEST}" ]]; then
  usage
  exit 2
fi
if [[ ${MATRIX} -eq 0 && -z "${ENVIRONMENT}" ]]; then
  echo "ERROR: provide either --environment <name> or --matrix" >&2
  usage
  exit 2
fi
if [[ ${MATRIX} -eq 1 && -n "${ENVIRONMENT}" ]]; then
  echo "ERROR: --environment and --matrix are mutually exclusive" >&2
  usage
  exit 2
fi
if [[ -n "${SCOPES_OVERRIDE}" && ${MATRIX} -eq 0 ]]; then
  echo "ERROR: --scopes is only valid with --matrix" >&2
  usage
  exit 2
fi

need_tool jq

if [[ ! -f "${CMP}" ]]; then
  echo "ERROR: compiled-model manifest not found: ${CMP}" >&2
  exit 2
fi
if [[ ! -f "${MANIFEST}" ]]; then
  echo "ERROR: environment manifest not found: ${MANIFEST}" >&2
  exit 2
fi

INTERPRETER="$(find_interpreter)"
if [[ ! -x "${INTERPRETER}" ]]; then
  echo "ERROR: interpreter binary is not executable: ${INTERPRETER}" >&2
  exit 2
fi

# Validate the manifest envelope up front.
if ! jq -e '.schema_version == 1 and (.environments | type == "object")' \
  "${MANIFEST}" >/dev/null 2>&1; then
  reject "manifest is not a valid environment manifest (need schema_version == 1 and an environments object): ${MANIFEST}"
fi

# ---------------------------------------------------------------------------
# Derive a deterministic <selection> label for the snapshot file name
# ---------------------------------------------------------------------------
# The choices map {facet: option} is read in sorted-key order, and its option
# values are joined with '-' to form a stable, human-readable label. When an
# environment has no choices, the environment name is used instead. The label
# is sanitized to a file-safe token.
selection_label() {
  local env_name="$1"
  local label
  label="$(jq -r --arg e "${env_name}" '
    (.environments[$e].choices // {})
    | to_entries | sort_by(.key) | map(.value | tostring) | join("-")
  ' "${MANIFEST}")"
  if [[ -z "${label}" ]]; then
    label="${env_name}"
  fi
  # Restrict to a safe token set for a file name component.
  echo "${label}" | tr -c 'A-Za-z0-9._-' '_'
}

# The <root> component of the snapshot name is the scope root: the part after
# "component:" for a component scope, or "all".
scope_root() {
  local scope="$1"
  case "${scope}" in
    component:*) echo "${scope#component:}" ;;
    all)         echo "all" ;;
    *)           echo "${scope}" | tr -c 'A-Za-z0-9._-' '_' ;;
  esac
}

# ---------------------------------------------------------------------------
# Resolve a single (environment, scope) cell into <cell_out>
# ---------------------------------------------------------------------------
# Drives open -> init-selection-state -> select x N -> resolve, threading the
# selection_state and applying each choice as one select delta. Writes the
# resolve result as resolve_result.<root>.<selection>.json in <cell_out>.
#
# The init-selection-state, select, and resolve interpreter calls read from
# /dev/null: this script drives them from inside loops whose input arrives on
# stdin via here-strings (the `select x N` loop below, and the matrix loop in
# the caller). With the native binary that is harmless — the --request-file
# verbs never read stdin — but a CONFIGFLUX_INTERPRETER override that ATTACHES
# stdin (e.g. a `docker run -i` wrapper around the toolchain image, ADR-0033,
# or an ssh wrapper) would otherwise drain the loop's input and silently apply
# only the first choice / resolve only the first environment (configflux-f3uf).
# `open` legitimately pipes its request on stdin and is therefore left as-is.
resolve_cell() {
  local env_name="$1" scope="$2" cell_out="$3"

  # The environment must exist in the manifest.
  if ! jq -e --arg e "${env_name}" '.environments | has($e)' \
    "${MANIFEST}" >/dev/null 2>&1; then
    reject "environment '${env_name}' is not defined in the manifest"
  fi

  mkdir -p "${cell_out}"
  local work="${cell_out}/.work"
  rm -rf "${work}"
  mkdir -p "${work}"

  # --- open ---------------------------------------------------------------
  printf '{"schema_version":4,"cmp_manifest_ref":"%s"}\n' "${CMP}" \
    | "${INTERPRETER}" open > "${work}/open.result.json" \
    || reject "open failed for environment '${env_name}'"

  # --- init-selection-state ----------------------------------------------
  # context_tags and scope come from the manifest entry; an empty/absent
  # context_tags defaults to {}.
  jq -n \
    --slurpfile o "${work}/open.result.json" \
    --arg scope "${scope}" \
    --argjson tags "$(jq -c --arg e "${env_name}" \
      '(.environments[$e].context_tags // {})' "${MANIFEST}")" \
    '{schema_version: 4, model_handle: $o[0].model_handle,
      scope: $scope, context_tags: $tags}' \
    > "${work}/init.request.json"
  "${INTERPRETER}" init-selection-state \
    --request-file "${work}/init.request.json" \
    --response-file "${work}/init.result.json" </dev/null \
    || reject "init-selection-state failed for environment '${env_name}'"

  # --- select x N ---------------------------------------------------------
  # Each choices entry becomes one select delta, applied in sorted-key order so
  # the chain is deterministic regardless of manifest authoring order.
  local prev_state="${work}/init.result.json"
  local facets
  facets="$(jq -r --arg e "${env_name}" \
    '(.environments[$e].choices // {}) | keys[]' "${MANIFEST}")"

  local idx=0
  local facet option req res
  while IFS= read -r facet; do
    [[ -z "${facet}" ]] && continue
    option="$(jq -r --arg e "${env_name}" --arg f "${facet}" \
      '.environments[$e].choices[$f]' "${MANIFEST}")"
    idx=$((idx + 1))
    req="${work}/select_${idx}.request.json"
    res="${work}/select_${idx}.result.json"
    jq -n \
      --slurpfile o "${work}/open.result.json" \
      --slurpfile s "${prev_state}" \
      --arg scope "${scope}" \
      --arg facet "${facet}" \
      --arg option "${option}" \
      '{schema_version: 4, model_handle: $o[0].model_handle, scope: $scope,
        selection_state: $s[0].selection_state,
        selection_delta: {facet: $facet, option: $option}}' \
      > "${req}"
    "${INTERPRETER}" select --request-file "${req}" --response-file "${res}" \
      </dev/null \
      || reject "select ${facet}=${option} failed for environment '${env_name}'"
    prev_state="${res}"
  done <<< "${facets}"

  # --- resolve ------------------------------------------------------------
  jq -n \
    --slurpfile o "${work}/open.result.json" \
    --slurpfile s "${prev_state}" \
    --arg scope "${scope}" \
    '{schema_version: 4, model_handle: $o[0].model_handle, scope: $scope,
      selection_state: $s[0].selection_state}' \
    > "${work}/resolve.request.json"

  local root sel snapshot
  root="$(scope_root "${scope}")"
  sel="$(selection_label "${env_name}")"
  snapshot="${cell_out}/resolve_result.${root}.${sel}.json"

  "${INTERPRETER}" resolve \
    --request-file "${work}/resolve.request.json" \
    --response-file "${snapshot}" </dev/null \
    || reject "resolve failed for environment '${env_name}' (scope ${scope})"

  # A clean resolve must report status ok.
  if [[ "$(jq -r '.status' "${snapshot}" 2>/dev/null)" != "ok" ]]; then
    echo "--- resolve diagnostics ---" >&2
    jq '.diagnostics // .' "${snapshot}" >&2 2>/dev/null || cat "${snapshot}" >&2
    reject "resolve for environment '${env_name}' (scope ${scope}) did not return status ok"
  fi

  rm -rf "${work}"
  echo "  -> ${snapshot} (scope ${scope})"
}

# ---------------------------------------------------------------------------
# Drive: single environment or matrix
# ---------------------------------------------------------------------------
mkdir -p "${OUT_DIR}"

if [[ ${MATRIX} -eq 0 ]]; then
  # Single environment: resolve its own scope.
  scope="$(jq -er --arg e "${ENVIRONMENT}" '.environments[$e].scope' \
    "${MANIFEST}" 2>/dev/null)" \
    || reject "environment '${ENVIRONMENT}' has no scope (or is undefined)"
  echo "=== resolve environment '${ENVIRONMENT}' ==="
  resolve_cell "${ENVIRONMENT}" "${scope}" "${OUT_DIR}"
  echo "Done. One snapshot written to ${OUT_DIR}/"
  exit 0
fi

# Matrix: every environment x (override scopes, or each environment's own
# scope). Each cell is independent and deterministic; one snapshot per cell.
ENV_NAMES="$(jq -r '.environments | keys[]' "${MANIFEST}")"

# Build the scope list for matrix mode.
declare -a SCOPE_LIST=()
if [[ -n "${SCOPES_OVERRIDE}" ]]; then
  IFS=',' read -r -a SCOPE_LIST <<< "${SCOPES_OVERRIDE}"
fi

CELL_COUNT=0
echo "=== matrix resolve (environments x scopes) ==="
while IFS= read -r env_name; do
  [[ -z "${env_name}" ]] && continue
  # Determine the scopes to resolve for this environment.
  declare -a env_scopes=()
  if [[ ${#SCOPE_LIST[@]} -gt 0 ]]; then
    env_scopes=("${SCOPE_LIST[@]}")
  else
    own_scope="$(jq -er --arg e "${env_name}" '.environments[$e].scope' \
      "${MANIFEST}" 2>/dev/null)" \
      || reject "environment '${env_name}' has no scope"
    env_scopes=("${own_scope}")
  fi

  for scope in "${env_scopes[@]}"; do
    # One cell directory per (environment, scope). The scope root keeps cells
    # for the same environment but different scopes from colliding.
    root="$(scope_root "${scope}")"
    cell_out="${OUT_DIR}/${env_name}/${root}"
    echo "--- ${env_name} x ${scope} ---"
    resolve_cell "${env_name}" "${scope}" "${cell_out}"
    CELL_COUNT=$((CELL_COUNT + 1))
  done
done <<< "${ENV_NAMES}"

echo "Done. ${CELL_COUNT} snapshot(s) written under ${OUT_DIR}/"
exit 0
