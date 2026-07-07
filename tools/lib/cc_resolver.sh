# shellcheck shell=bash
#
# Shared native C/C++ compiler resolver for gate entry points.
#
# Gate wrappers must never hand Bazel (or any subprocess) a bare compiler
# name like "clang" when tools/ or tools/sdk-policy-shims/ may be on PATH —
# that can resolve back to the policy shim and re-enter clang_wrapper.sh,
# which, under any sandbox/symlink edge case, becomes a fork bomb.
#
# This library provides configflux_resolve_native_cc, which:
#   1. Rejects any CC/CXX that is not an absolute path (forces re-resolution)
#   2. Rejects any CC/CXX whose resolved dir is co-located with
#      clang_wrapper.sh (i.e. is a policy shim)
#   3. Walks gcc/cc/clang on PATH, skipping any candidate co-located with
#      clang_wrapper.sh, and returns the first absolute, non-shim match
#
# Both tools/run_local_task_gate.sh and tools/run_release_gate.sh source
# this library. Keeping the resolver in one place ensures future hardening
# lands once and both gates benefit.

_configflux_cc_is_shim_path() {
  # Returns 0 (true) if the candidate path is a ConfigFlux policy shim that
  # would re-exec tools/clang_wrapper.sh, creating a recursion surface.
  #
  # Two layouts are recognised as shim locations:
  #   (a) candidate sits directly beside clang_wrapper.sh  (legacy tools/)
  #   (b) candidate sits in tools/sdk-policy-shims/ beside a parent
  #       directory containing clang_wrapper.sh              (current layout)
  local candidate="$1"
  local abs abs_dir parent_dir
  abs="$(readlink -f "${candidate}" 2>/dev/null || echo "${candidate}")"
  abs_dir="$(dirname "${abs}")"
  parent_dir="$(dirname "${abs_dir}")"

  # (a) sibling layout
  if [[ -f "${abs_dir}/clang_wrapper.sh" ]]; then
    return 0
  fi
  # (b) sdk-policy-shims layout (or any subdir next to clang_wrapper.sh)
  if [[ "$(basename "${abs_dir}")" == "sdk-policy-shims" ]] \
      && [[ -f "${parent_dir}/clang_wrapper.sh" ]]; then
    return 0
  fi
  return 1
}

_configflux_cc_resolve_candidate() {
  # Resolves a bare command name to an absolute, non-shim path.
  # Echoes the resolved path on success. Returns non-zero if no safe
  # candidate can be found.
  local name="$1"
  local abs
  abs="$(command -v "${name}" 2>/dev/null)" || return 1
  abs="$(readlink -f "${abs}" 2>/dev/null || echo "${abs}")"
  [[ "${abs}" == /* ]] || return 1
  if _configflux_cc_is_shim_path "${abs}"; then
    return 1
  fi
  printf '%s\n' "${abs}"
}

configflux_resolve_native_cc() {
  # Hardened CC/CXX pre-flight.
  #
  # - Trusts a pre-set CC/CXX ONLY if it is an absolute path AND not a
  #   policy-shim co-located path.
  # - Otherwise clears and re-resolves from gcc/cc/clang (for CC) or
  #   clang++/g++ (for CXX).
  # - Leaves CC/CXX unchanged if no safe candidate exists; downstream
  #   degradation logic handles the no-compiler case.
  local cc_candidate="" cxx_candidate=""

  # ---- CC -------------------------------------------------------------
  if [[ -n "${CC:-}" ]]; then
    if [[ "${CC}" == /* ]] && ! _configflux_cc_is_shim_path "${CC}"; then
      : # trusted absolute, non-shim path — keep as-is
    else
      unset CC
    fi
  fi

  if [[ -z "${CC:-}" ]]; then
    local _cc_name
    for _cc_name in gcc cc clang; do
      if cc_candidate="$(_configflux_cc_resolve_candidate "${_cc_name}" 2>/dev/null)"; then
        export CC="${cc_candidate}"
        break
      fi
    done
  fi

  # ---- CXX ------------------------------------------------------------
  if [[ -n "${CXX:-}" ]]; then
    if [[ "${CXX}" == /* ]] && ! _configflux_cc_is_shim_path "${CXX}"; then
      : # trusted absolute, non-shim path — keep as-is
    else
      unset CXX
    fi
  fi

  if [[ -z "${CXX:-}" ]]; then
    local _cxx_name
    if [[ "${CC:-}" == *"clang"* ]]; then
      for _cxx_name in clang++ g++; do
        if cxx_candidate="$(_configflux_cc_resolve_candidate "${_cxx_name}" 2>/dev/null)"; then
          export CXX="${cxx_candidate}"
          break
        fi
      done
    else
      for _cxx_name in g++ clang++; do
        if cxx_candidate="$(_configflux_cc_resolve_candidate "${_cxx_name}" 2>/dev/null)"; then
          export CXX="${cxx_candidate}"
          break
        fi
      done
    fi
  fi

  return 0
}
