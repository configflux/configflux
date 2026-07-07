#!/usr/bin/env bash

configflux_clang_toolchain_dir() {
  printf '%s\n' "${CONFIGFLUX_CLANG_TOOLCHAIN_DIR:-/tmp/configflux-clang-toolchain}"
}

configflux_clang_toolchain_root() {
  local toolchain_dir
  toolchain_dir="$(configflux_clang_toolchain_dir)"
  printf '%s\n' "${toolchain_dir}/root"
}

configflux_clang_runtime_lib_path() {
  local root_dir="$1"
  local multiarch=""
  local joined=""
  local candidate
  local -a candidates=()
  local -a existing=()
  local -A seen=()

  if command -v dpkg-architecture >/dev/null 2>&1; then
    multiarch="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || true)"
  fi

  if [[ -n "${multiarch}" ]]; then
    candidates+=("${root_dir}/usr/lib/${multiarch}")
    candidates+=("${root_dir}/lib/${multiarch}")
  fi

  candidates+=("${root_dir}/usr/lib")
  candidates+=("${root_dir}/lib")

  for candidate in "${root_dir}/usr/lib/"*-linux-gnu "${root_dir}/lib/"*-linux-gnu; do
    if [[ -d "${candidate}" ]]; then
      candidates+=("${candidate}")
    fi
  done

  for candidate in "${candidates[@]}"; do
    if [[ ! -d "${candidate}" ]]; then
      continue
    fi
    if [[ -n "${seen["${candidate}"]+x}" ]]; then
      continue
    fi
    seen["${candidate}"]=1
    existing+=("${candidate}")
  done

  for candidate in "${existing[@]}"; do
    if [[ -z "${joined}" ]]; then
      joined="${candidate}"
    else
      joined="${joined}:${candidate}"
    fi
  done

  printf '%s\n' "${joined}"
}
