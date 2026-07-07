#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
  WORKSPACE_ROOT="${BUILD_WORKSPACE_DIRECTORY}"
else
  WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
fi

usage() {
  cat <<'EOF'
Usage:
  bazel run //tools:run_ros2_colcon -- [--workspace <path>] [-- <extra colcon args>]

Examples:
  bazel run //tools:run_ros2_colcon -- --workspace sdk/ros2
  bazel run //tools:run_ros2_colcon -- --workspace sdk/ros2 -- --packages-select configflux_ros2_sdk

Diagnostics (env overrides; set to 0 to disable):
  RUN_ROS2_COLCON_HEARTBEAT_SECS=60
  RUN_ROS2_COLCON_INACTIVITY_TIMEOUT_SECS=600
  RUN_ROS2_COLCON_MONITOR_POLL_SECS=5
  RUN_ROS2_COLCON_TIMEOUT_GRACE_SECS=10
EOF
}

workspace="sdk/ros2"
colcon_args=()
monitor_heartbeat_secs="${RUN_ROS2_COLCON_HEARTBEAT_SECS:-60}"
monitor_inactivity_timeout_secs="${RUN_ROS2_COLCON_INACTIVITY_TIMEOUT_SECS:-600}"
monitor_poll_secs="${RUN_ROS2_COLCON_MONITOR_POLL_SECS:-5}"
monitor_timeout_grace_secs="${RUN_ROS2_COLCON_TIMEOUT_GRACE_SECS:-10}"

ensure_nonnegative_integer() {
  local label="$1"
  local value="$2"
  if [[ ! "${value}" =~ ^[0-9]+$ ]]; then
    echo "error: ${label} must be a non-negative integer (got '${value}')" >&2
    exit 2
  fi
}

stat_mtime_epoch() {
  local path="$1"
  if [[ ! -e "${path}" ]]; then
    return 1
  fi
  stat -c '%Y' "${path}"
}

latest_colcon_logger_path() {
  local workspace_dir="$1"
  local latest_symlink_logger="${workspace_dir}/log/latest_build/logger_all.log"
  local latest_found_logger=""

  if [[ -f "${latest_symlink_logger}" ]]; then
    printf '%s\n' "${latest_symlink_logger}"
    return 0
  fi

  if [[ ! -d "${workspace_dir}/log" ]]; then
    return 1
  fi

  latest_found_logger="$(
    find "${workspace_dir}/log" -maxdepth 2 -type f -name 'logger_all.log' -printf '%T@ %p\n' 2>/dev/null \
    | sort -nr \
    | head -n 1 \
    | awk '{ $1=""; sub(/^ /, ""); print }'
  )"
  if [[ -z "${latest_found_logger}" ]]; then
    return 1
  fi
  printf '%s\n' "${latest_found_logger}"
}

latest_colcon_activity_epoch() {
  local workspace_dir="$1"
  local fallback_epoch="$2"
  local logger_path=""
  local mtime=""

  if logger_path="$(latest_colcon_logger_path "${workspace_dir}")"; then
    if mtime="$(stat_mtime_epoch "${logger_path}")"; then
      printf '%s\n' "${mtime}"
      return 0
    fi
  fi

  if mtime="$(stat_mtime_epoch "${workspace_dir}/log")"; then
    printf '%s\n' "${mtime}"
    return 0
  fi

  printf '%s\n' "${fallback_epoch}"
}

print_colcon_process_snapshot() {
  local colcon_pid="$1"

  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  echo "warning: process snapshot for colcon pid ${colcon_pid}" >&2
  ps -o pid=,ppid=,stat=,etime=,pcpu=,pmem=,command= -p "${colcon_pid}" 2>/dev/null >&2 || true
  ps -o pid=,ppid=,stat=,etime=,pcpu=,pmem=,command= --ppid "${colcon_pid}" 2>/dev/null >&2 || true
}

emit_inactivity_timeout_diagnostics() {
  local workspace_dir="$1"
  local colcon_pid="$2"
  local elapsed_secs="$3"
  local silence_secs="$4"
  local timeout_secs="$5"
  local logger_path=""

  echo "error: run_ros2_colcon inactivity timeout after ${silence_secs}s without colcon log activity (limit ${timeout_secs}s, elapsed ${elapsed_secs}s)" >&2
  echo "warning: workspace=${workspace_dir}" >&2
  if logger_path="$(latest_colcon_logger_path "${workspace_dir}")"; then
    echo "warning: latest colcon logger=${logger_path}" >&2
    echo "warning: tail -n 20 ${logger_path}" >&2
    tail -n 20 "${logger_path}" >&2 || true
  else
    echo "warning: colcon logger_all.log not found yet under ${workspace_dir}/log" >&2
  fi
  print_colcon_process_snapshot "${colcon_pid}"
}

terminate_colcon_process() {
  local colcon_pid="$1"
  local grace_secs="$2"

  if ! kill -0 "${colcon_pid}" 2>/dev/null; then
    return 0
  fi

  kill -TERM "${colcon_pid}" 2>/dev/null || true

  local deadline=$((SECONDS + grace_secs))
  while kill -0 "${colcon_pid}" 2>/dev/null; do
    if (( SECONDS >= deadline )); then
      break
    fi
    sleep 1
  done

  if kill -0 "${colcon_pid}" 2>/dev/null; then
    echo "warning: colcon pid ${colcon_pid} did not exit after ${grace_secs}s; sending SIGKILL" >&2
    kill -KILL "${colcon_pid}" 2>/dev/null || true
  fi
}

monitor_colcon_liveness() {
  local workspace_dir="$1"
  local colcon_pid="$2"
  local heartbeat_secs="$3"
  local inactivity_timeout_secs="$4"
  local poll_secs="$5"
  local timeout_grace_secs="$6"
  local start_epoch="$7"
  local timeout_flag_file="$8"

  local now_epoch="${start_epoch}"
  local last_activity_epoch="${start_epoch}"
  local last_heartbeat_epoch="${start_epoch}"
  local current_activity_epoch=""
  local elapsed_secs=0
  local silence_secs=0
  local logger_path=""

  while kill -0 "${colcon_pid}" 2>/dev/null; do
    now_epoch="$(date +%s)"
    current_activity_epoch="$(latest_colcon_activity_epoch "${workspace_dir}" "${start_epoch}")"
    if (( current_activity_epoch > last_activity_epoch )); then
      last_activity_epoch="${current_activity_epoch}"
    fi

    elapsed_secs=$((now_epoch - start_epoch))
    silence_secs=$((now_epoch - last_activity_epoch))

    if (( heartbeat_secs > 0 && now_epoch - last_heartbeat_epoch >= heartbeat_secs )); then
      if logger_path="$(latest_colcon_logger_path "${workspace_dir}")"; then
        echo "info: run_ros2_colcon heartbeat elapsed=${elapsed_secs}s silence=${silence_secs}s pid=${colcon_pid} logger=${logger_path}" >&2
      else
        echo "info: run_ros2_colcon heartbeat elapsed=${elapsed_secs}s silence=${silence_secs}s pid=${colcon_pid} logger=<pending>" >&2
      fi
      last_heartbeat_epoch="${now_epoch}"
    fi

    if (( inactivity_timeout_secs > 0 && silence_secs >= inactivity_timeout_secs )); then
      emit_inactivity_timeout_diagnostics "${workspace_dir}" "${colcon_pid}" "${elapsed_secs}" "${silence_secs}" "${inactivity_timeout_secs}"
      printf 'inactivity-timeout\n' >"${timeout_flag_file}"
      terminate_colcon_process "${colcon_pid}" "${timeout_grace_secs}"
      return 0
    fi

    sleep "${poll_secs}"
  done
}

resolve_workspace_path() {
  local requested_path="$1"
  if [[ "${requested_path}" = /* ]]; then
    printf '%s\n' "${requested_path}"
    return 0
  fi

  local workspace_root_candidate="${WORKSPACE_ROOT}/${requested_path}"
  if [[ -d "${workspace_root_candidate}" ]]; then
    printf '%s\n' "${workspace_root_candidate}"
    return 0
  fi

  printf '%s\n' "${requested_path}"
}

source_with_relaxed_nounset() {
  local setup_script="$1"
  local had_nounset=0
  local status=0

  if [[ $- == *u* ]]; then
    had_nounset=1
    set +u
  fi

  # shellcheck disable=SC1090
  source "${setup_script}" || status=$?

  if [[ ${had_nounset} -eq 1 ]]; then
    set -u
  fi

  return "${status}"
}

initialize_ros_env_defaults() {
  export AMENT_TRACE_SETUP_FILES="${AMENT_TRACE_SETUP_FILES:-}"
  export AMENT_PREFIX_PATH="${AMENT_PREFIX_PATH:-}"
  export COLCON_PREFIX_PATH="${COLCON_PREFIX_PATH:-}"
  export CMAKE_PREFIX_PATH="${CMAKE_PREFIX_PATH:-}"
  export PYTHONPATH="${PYTHONPATH:-}"
  export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}"

  if [[ -z "${AMENT_PYTHON_EXECUTABLE:-}" ]]; then
    local python3_path=""
    if python3_path="$(command -v python3 2>/dev/null)"; then
      export AMENT_PYTHON_EXECUTABLE="${python3_path}"
    fi
  fi
}

resolve_clang_wrapper() {
  local compiler="$1"
  # Policy shims moved to tools/sdk-policy-shims/ so they are not
  # PATH-discoverable and cannot be re-entered by a child process running
  # `command -v clang`. Keep the sibling tools/ path as a legacy fallback
  # (harmless if the file is absent).
  local workspace_candidate="${WORKSPACE_ROOT}/tools/sdk-policy-shims/${compiler}"
  local legacy_candidate="${WORKSPACE_ROOT}/tools/${compiler}"
  local script_candidate="${SCRIPT_DIR}/sdk-policy-shims/${compiler}"
  local legacy_script_candidate="${SCRIPT_DIR}/${compiler}"

  if [[ -x "${workspace_candidate}" ]]; then
    printf '%s\n' "${workspace_candidate}"
    return 0
  fi
  if [[ -x "${legacy_candidate}" ]]; then
    printf '%s\n' "${legacy_candidate}"
    return 0
  fi
  if [[ -x "${script_candidate}" ]]; then
    printf '%s\n' "${script_candidate}"
    return 0
  fi
  if [[ -x "${legacy_script_candidate}" ]]; then
    printf '%s\n' "${legacy_script_candidate}"
    return 0
  fi
  if command -v "${compiler}" >/dev/null 2>&1; then
    command -v "${compiler}"
    return 0
  fi
  return 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace)
      if [[ $# -lt 2 ]]; then
        echo "error: --workspace requires a value" >&2
        exit 2
      fi
      workspace="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      colcon_args+=("$@")
      break
      ;;
    *)
      colcon_args+=("$1")
      shift
      ;;
  esac
done

ensure_nonnegative_integer "RUN_ROS2_COLCON_HEARTBEAT_SECS" "${monitor_heartbeat_secs}"
ensure_nonnegative_integer "RUN_ROS2_COLCON_INACTIVITY_TIMEOUT_SECS" "${monitor_inactivity_timeout_secs}"
ensure_nonnegative_integer "RUN_ROS2_COLCON_MONITOR_POLL_SECS" "${monitor_poll_secs}"
ensure_nonnegative_integer "RUN_ROS2_COLCON_TIMEOUT_GRACE_SECS" "${monitor_timeout_grace_secs}"
if [[ "${monitor_poll_secs}" == "0" ]]; then
  echo "error: RUN_ROS2_COLCON_MONITOR_POLL_SECS must be greater than 0" >&2
  exit 2
fi

workspace_arg="${workspace}"
workspace="$(resolve_workspace_path "${workspace_arg}")"

if ! command -v colcon >/dev/null 2>&1; then
  echo "error: colcon was not found in PATH" >&2
  exit 1
fi

if [[ ! -d "$workspace" ]]; then
  if [[ "${workspace_arg}" == "${workspace}" ]]; then
    echo "error: workspace '${workspace_arg}' does not exist" >&2
  else
    echo "error: workspace '${workspace_arg}' does not exist (resolved to '${workspace}')" >&2
  fi
  exit 1
fi

if [[ ! -d "$workspace/src" ]]; then
  echo "error: workspace '$workspace' must include a src/ directory" >&2
  exit 1
fi

if [[ -z "${ROS_DISTRO:-}" && -d /opt/ros ]]; then
  guessed_distro="$(ls -1 /opt/ros | sort | tail -n 1 || true)"
  if [[ -n "${guessed_distro}" ]]; then
    export ROS_DISTRO="${guessed_distro}"
  fi
fi

ros_setup_script=""
if [[ -n "${ROS_DISTRO:-}" ]]; then
  ros_setup_script="${ROS_SETUP_SCRIPT:-/opt/ros/${ROS_DISTRO}/setup.bash}"
fi

initialize_ros_env_defaults

if [[ -n "${ros_setup_script}" && -f "${ros_setup_script}" ]]; then
  if ! source_with_relaxed_nounset "${ros_setup_script}"; then
    echo "error: failed to source ROS setup script '${ros_setup_script}'" >&2
    exit 1
  fi
else
  echo "warning: ROS setup script not found; building with current environment" >&2
fi

# Reject a bare-name CC/CXX inherited from the environment (e.g. the
# devcontainer ENV CC=clang from docker/dev.Dockerfile). A bare name
# would cause colcon to PATH-resolve the compiler at build time, which
# bypasses the SDK clang policy wrapper entirely and may resolve to raw
# /usr/bin/clang. Force re-resolution via resolve_clang_wrapper so CC
# ends up pointing at tools/sdk-policy-shims/clang (the policy shim).
if [[ -n "${CC:-}" && "${CC}" != /* ]]; then
  unset CC
fi
if [[ -n "${CXX:-}" && "${CXX}" != /* ]]; then
  unset CXX
fi

if [[ -z "${CC:-}" ]]; then
  if ! CC="$(resolve_clang_wrapper clang)"; then
    echo "error: unable to locate clang wrapper or clang executable" >&2
    exit 1
  fi
  export CC
fi
if [[ -z "${CXX:-}" ]]; then
  if ! CXX="$(resolve_clang_wrapper clang++)"; then
    echo "error: unable to locate clang++ wrapper or clang++ executable" >&2
    exit 1
  fi
  export CXX
fi

pushd "$workspace" >/dev/null
cmd=(colcon build --merge-install --event-handlers console_direct+)
if [[ ${#colcon_args[@]} -gt 0 ]]; then
  cmd+=("${colcon_args[@]}")
fi

echo "Running: ${cmd[*]}"
echo "Diagnostics: heartbeat=${monitor_heartbeat_secs}s inactivity_timeout=${monitor_inactivity_timeout_secs}s poll=${monitor_poll_secs}s grace=${monitor_timeout_grace_secs}s" >&2

monitor_tmpdir="$(mktemp -d)"
monitor_timeout_flag_file="${monitor_tmpdir}/timeout.flag"
monitor_pid=""
colcon_pid=""
colcon_status=0
monitor_timed_out=0

cleanup_monitor_artifacts() {
  if [[ -n "${monitor_pid:-}" ]] && kill -0 "${monitor_pid}" 2>/dev/null; then
    kill "${monitor_pid}" 2>/dev/null || true
    wait "${monitor_pid}" 2>/dev/null || true
  fi
  rm -rf "${monitor_tmpdir}"
}

forward_interrupt_to_colcon() {
  local signal_name="$1"
  if [[ -n "${colcon_pid:-}" ]] && kill -0 "${colcon_pid}" 2>/dev/null; then
    echo "warning: received ${signal_name}; terminating colcon pid ${colcon_pid}" >&2
    terminate_colcon_process "${colcon_pid}" "${monitor_timeout_grace_secs}"
  fi
}

trap 'forward_interrupt_to_colcon SIGINT; cleanup_monitor_artifacts; exit 130' INT
trap 'forward_interrupt_to_colcon SIGTERM; cleanup_monitor_artifacts; exit 143' TERM

start_epoch="$(date +%s)"
"${cmd[@]}" &
colcon_pid=$!

monitor_colcon_liveness \
  "${workspace}" \
  "${colcon_pid}" \
  "${monitor_heartbeat_secs}" \
  "${monitor_inactivity_timeout_secs}" \
  "${monitor_poll_secs}" \
  "${monitor_timeout_grace_secs}" \
  "${start_epoch}" \
  "${monitor_timeout_flag_file}" &
monitor_pid=$!

set +e
wait "${colcon_pid}"
colcon_status=$?
set -e

if [[ -s "${monitor_timeout_flag_file}" ]]; then
  monitor_timed_out=1
fi

cleanup_monitor_artifacts
trap - INT TERM

if [[ "${monitor_timed_out}" -eq 1 ]]; then
  colcon_status=124
fi

popd >/dev/null
exit "${colcon_status}"
