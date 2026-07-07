#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/clang_toolchain_env.sh
source "${SCRIPT_DIR}/clang_toolchain_env.sh"

usage() {
  cat <<'EOF'
Usage:
  tools/install_clang_toolchain.sh [--toolchain-dir <path>]

Description:
  Installs a non-root clang 18 bundle by downloading and unpacking Ubuntu
  packages into a local directory.

Defaults:
  --toolchain-dir defaults to:
    $CONFIGFLUX_CLANG_TOOLCHAIN_DIR or /tmp/configflux-clang-toolchain

After installation, ConfigFlux clang wrappers will automatically discover and
use the bundle.
EOF
}

package_is_available() {
  local package_name="$1"
  apt-cache show "${package_name}" >/dev/null 2>&1
}

resolve_latest_available_package() {
  local prefix="$1"
  local regex="$2"
  local description="$3"
  local package_name
  local -a matches=()

  while IFS= read -r package_name; do
    if [[ -z "${package_name}" ]]; then
      continue
    fi
    if [[ ! "${package_name}" =~ ${regex} ]]; then
      continue
    fi
    if ! package_is_available "${package_name}"; then
      continue
    fi
    matches+=("${package_name}")
  done < <(apt-cache pkgnames "${prefix}")

  if [[ ${#matches[@]} -eq 0 ]]; then
    echo "error: unable to resolve ${description}; no available package matched ${regex}" >&2
    return 1
  fi

  printf '%s\n' "$(printf '%s\n' "${matches[@]}" | sort -Vu | tail -n1)"
}

build_download_packages() {
  local libicu_package
  local libobjc_dev_package

  libicu_package="$(resolve_latest_available_package "libicu" '^libicu[0-9]+$' "libicu runtime package")"
  libobjc_dev_package="$(resolve_latest_available_package "libobjc-" '^libobjc-[0-9]+-dev$' "libobjc development package")"

  printf '%s\n' \
    "clang-18" \
    "libclang-common-18-dev" \
    "libclang-cpp18" \
    "libclang1-18" \
    "libgc1" \
    "${libicu_package}" \
    "libllvm18" \
    "${libobjc_dev_package}" \
    "libobjc4" \
    "libxml2" \
    "llvm-18-linker-tools"
}

toolchain_dir=""
root_dir=""
packages_dir=""
bin_dir=""
clang_path=""
clangxx_path=""

verify_clang_binary() {
  local compiler_path="$1"
  local local_lib_path=""

  local_lib_path="$(configflux_clang_runtime_lib_path "${root_dir}")"
  if [[ -n "${local_lib_path}" ]]; then
    LD_LIBRARY_PATH="${local_lib_path}" "${compiler_path}" --version >/dev/null 2>&1
  else
    "${compiler_path}" --version >/dev/null 2>&1
  fi
}

install_shims() {
  mkdir -p "${bin_dir}"

  cat > "${bin_dir}/clang" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

compute_local_lib_path() {
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

toolchain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
local_lib_path="$(compute_local_lib_path "${toolchain_root}/root")"
if [[ -n "${local_lib_path}" ]]; then
  if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
    export LD_LIBRARY_PATH="${local_lib_path}:${LD_LIBRARY_PATH}"
  else
    export LD_LIBRARY_PATH="${local_lib_path}"
  fi
fi
exec "${toolchain_root}/root/usr/bin/clang-18" "$@"
EOF

  cat > "${bin_dir}/clang++" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

compute_local_lib_path() {
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

toolchain_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
local_lib_path="$(compute_local_lib_path "${toolchain_root}/root")"
if [[ -n "${local_lib_path}" ]]; then
  if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
    export LD_LIBRARY_PATH="${local_lib_path}:${LD_LIBRARY_PATH}"
  else
    export LD_LIBRARY_PATH="${local_lib_path}"
  fi
fi
exec "${toolchain_root}/root/usr/bin/clang++-18" "$@"
EOF

  chmod +x "${bin_dir}/clang" "${bin_dir}/clang++"
}

main() {
  local -a packages=()

  toolchain_dir="$(configflux_clang_toolchain_dir)"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --toolchain-dir)
        if [[ $# -lt 2 ]]; then
          echo "error: --toolchain-dir requires a value" >&2
          return 2
        fi
        toolchain_dir="$2"
        shift 2
        ;;
      -h|--help)
        usage
        return 0
        ;;
      *)
        echo "error: unknown argument '$1'" >&2
        usage >&2
        return 2
        ;;
    esac
  done

  if ! command -v apt >/dev/null 2>&1; then
    echo "error: apt is required to bootstrap clang in this environment" >&2
    return 1
  fi

  if ! command -v apt-cache >/dev/null 2>&1; then
    echo "error: apt-cache is required to resolve clang dependencies in this environment" >&2
    return 1
  fi

  if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "error: dpkg-deb is required to unpack clang packages" >&2
    return 1
  fi

  root_dir="${toolchain_dir}/root"
  packages_dir="${toolchain_dir}/pkgs"
  bin_dir="${toolchain_dir}/bin"
  clang_path="${root_dir}/usr/bin/clang-18"
  clangxx_path="${root_dir}/usr/bin/clang++-18"

  if [[ -x "${clang_path}" && -x "${clangxx_path}" ]]; then
    if verify_clang_binary "${clang_path}"; then
      install_shims
      echo "clang bundle already installed at ${toolchain_dir}"
      echo "  shims:   ${bin_dir}/clang, ${bin_dir}/clang++"
      return 0
    fi
  fi

  mapfile -t packages < <(build_download_packages)

  mkdir -p "${root_dir}" "${packages_dir}" "${bin_dir}"
  rm -f "${packages_dir}"/*.deb

  pushd "${packages_dir}" >/dev/null
  echo "Downloading clang toolchain packages..."
  apt download "${packages[@]}"

  echo "Extracting clang toolchain packages..."
  for deb in ./*.deb; do
    dpkg-deb -x "${deb}" "${root_dir}"
  done
  popd >/dev/null

  if ! verify_clang_binary "${clang_path}"; then
    echo "error: clang verification failed after install" >&2
    return 1
  fi

  if ! verify_clang_binary "${clangxx_path}"; then
    echo "error: clang++ verification failed after install" >&2
    return 1
  fi

  install_shims

  echo "clang bundle installed at ${toolchain_dir}"
  echo "  clang:   ${clang_path}"
  echo "  clang++: ${clangxx_path}"
  echo "  shims:   ${bin_dir}/clang, ${bin_dir}/clang++"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
