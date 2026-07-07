# C++ Engineering Standards

Status: normative
Scope: all first-party C++ code in ConfigFlux (`sdk/cpp`, `sdk/ros2`, and related tests/tools)

## 1. Non-Negotiable Rules

1. Language is C++ only for first-party wrappers/SDK:
   - Do not add first-party `*.c` files.
   - A C-compatible ABI boundary is allowed where required, but implementation remains
     Rust/C++.
2. Modern C++ baseline:
   - Minimum standard: C++20.
   - Prefer standard-library facilities over ad-hoc utilities.
3. Compiler:
   - First-party C++ must compile with `clang`/`clang++` (clang-only policy).
   - Bazel C++ builds must use `--config=clang_cpp`.
4. Style:
   - Follow Google C++ style.
   - Formatting uses repository `.clang-format` (Google-based).

## 2. Repository Layout and Ownership

1. Runtime core logic belongs in `runtime/` (Rust).
2. C++ SDK code belongs in `sdk/cpp/`.
3. ROS2 wrapper code belongs in `sdk/ros2/`.
4. Do not place SDK implementation under `runtime/`.

## 2.1 Toolchain Wrapper Contract

1. Use the ConfigFlux clang wrapper (`tools/clang_wrapper.sh --compiler clang|clang++`) when
   wiring first-party C++ toolchains through scripts.
2. Optional local clang bundle location is configured via
   `CONFIGFLUX_CLANG_TOOLCHAIN_DIR` (default: `/tmp/configflux-clang-toolchain`).
3. Wrapper/runtime library path resolution must stay architecture-aware; do not hardcode
   architecture-specific library directories in multiple scripts.

## 3. Design and API Expectations

1. Use RAII and explicit ownership.
2. Avoid owning raw pointers in public APIs.
3. Prefer value semantics, `std::unique_ptr`, and `std::shared_ptr` when ownership sharing
   is required.
4. Keep exceptions from crossing FFI/boundary layers.
5. Expose deterministic error/status mapping for runtime-facing operations.

## 4. Embedded-System Constraints

1. Keep allocations bounded and predictable in hot paths.
2. Avoid hidden global state and nondeterministic behavior.
3. Define thread-safety contracts per API surface.
4. Preserve deterministic event ordering where callbacks are exposed.

## 5. Verification Expectations

1. All new C++ behavior needs Bazel-mapped tests.
2. ROS2 wrapper behavior also requires colcon build validation via Bazel orchestration.
3. Requirement IDs and test mappings must be updated in `docs/requirement-test-matrix.tsv`.
