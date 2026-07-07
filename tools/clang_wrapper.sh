#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/clang_toolchain_env.sh
source "${SCRIPT_DIR}/clang_toolchain_env.sh"

usage() {
  cat <<'EOF'
Usage:
  tools/clang_wrapper.sh --compiler <clang|clang++> [compiler args...]

Description:
  Resolves a clang/clang++ executable for ConfigFlux clang-only builds.
  Resolution order:
    1. CONFIGFLUX_CLANG_BIN_DIR/<compiler>[(-18)]
    2. CONFIGFLUX_CLANG_TOOLCHAIN_DIR bootstrap bundle (default: /tmp/configflux-clang-toolchain)
    3. <compiler> on PATH
    4. common distro install paths

  If no compiler is found, run:
    tools/install_clang_toolchain.sh
EOF
}

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 2
fi

compiler=""
case "$1" in
  --compiler=clang)
    compiler="clang"
    shift
    ;;
  --compiler=clang++)
    compiler="clang++"
    shift
    ;;
  --compiler)
    if [[ $# -lt 2 ]]; then
      usage >&2
      exit 2
    fi
    compiler="$2"
    shift 2
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

if [[ "${compiler}" != "clang" && "${compiler}" != "clang++" ]]; then
  echo "error: unsupported compiler '${compiler}' (expected clang or clang++)" >&2
  exit 2
fi

local_root="$(configflux_clang_toolchain_root)"
local_lib_path="$(configflux_clang_runtime_lib_path "${local_root}")"

declare -a candidates=()

if [[ -n "${CONFIGFLUX_CLANG_BIN_DIR:-}" ]]; then
  candidates+=("${CONFIGFLUX_CLANG_BIN_DIR}/${compiler}")
  candidates+=("${CONFIGFLUX_CLANG_BIN_DIR}/${compiler}-18")
fi

candidates+=("${local_root}/usr/bin/${compiler}-18")
candidates+=("${local_root}/usr/bin/${compiler}")

if path_candidate="$(command -v "${compiler}" 2>/dev/null)"; then
  # Avoid infinite recursion: skip if PATH resolves back to our own wrapper
  # or to a shim that would re-invoke us (e.g. tools/clang → clang_wrapper.sh).
  # Use readlink -f to canonicalise paths — the Bazel sandbox and git worktrees
  # may expose the script through symlink chains or different mount points that
  # make naive path comparison unreliable.
  path_candidate_real="$(readlink -f "${path_candidate}" 2>/dev/null || echo "${path_candidate}")"
  script_real="$(readlink -f "${BASH_SOURCE[0]}" 2>/dev/null || echo "${BASH_SOURCE[0]}")"
  _clang_wrapper_is_self() {
    # (a) candidate IS the wrapper itself (symlink or same file)
    [[ "${path_candidate_real}" == "${script_real}" ]] && return 0
    # (b) candidate sits next to a clang_wrapper.sh — it is a shim that
    #     would re-invoke us, causing infinite recursion
    local cand_dir
    cand_dir="$(dirname "${path_candidate_real}")"
    [[ -f "${cand_dir}/clang_wrapper.sh" ]] && return 0
    return 1
  }
  if ! _clang_wrapper_is_self; then
    candidates+=("${path_candidate}")
  fi
fi

candidates+=("/usr/bin/${compiler}")
candidates+=("/usr/local/bin/${compiler}")
candidates+=("/usr/lib/llvm-18/bin/${compiler}")
candidates+=("/usr/lib/llvm-17/bin/${compiler}")
candidates+=("/opt/homebrew/opt/llvm/bin/${compiler}")

resolved=""
for candidate in "${candidates[@]}"; do
  if [[ -x "${candidate}" ]]; then
    resolved="${candidate}"
    break
  fi
done

if [[ -z "${resolved}" ]]; then
  echo "error: unable to locate ${compiler}" >&2
  echo "hint: run tools/install_clang_toolchain.sh to install a local non-root clang bundle" >&2
  echo "hint: or set CONFIGFLUX_CLANG_BIN_DIR to a directory containing ${compiler}" >&2
  exit 1
fi

if [[ "${resolved}" == "${local_root}/"* && -n "${local_lib_path}" ]]; then
  if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
    export LD_LIBRARY_PATH="${local_lib_path}:${LD_LIBRARY_PATH}"
  else
    export LD_LIBRARY_PATH="${local_lib_path}"
  fi
fi

exec "${resolved}" "$@"
