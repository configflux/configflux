// SPDX-License-Identifier: BUSL-1.1
//
// Crate root for the `runtime_c_abi_static` staticlib (configflux-u32v).
//
// This static library exports the runtime C ABI symbols
// (`configflux_runtime_session_*`) that the C++/ROS2 SDKs link against. It lives
// in the runtime crate — not the compiler crate — so the open entrypoint can
// enforce the ADR-0030 D2 `.ccm` solver-model precondition, which requires the
// `solver` crate that the compiler may not import (ADR-0003 §2).
//
// It deliberately includes ONLY the modules the C ABI needs:
//   - `solver_validation`: the shared `runtime_open_with_solver_validation`
//     fail-closed wrapper and `ccm_usable_for_open` predicate (also used by the
//     runtime CLI handler — single enforcement point).
//   - `runtime_c_abi`: the `#[no_mangle] extern "C"` exports themselves.
//
// It does NOT include `cli_adapter` (the clap-based CLI shell), so the staticlib
// stays free of CLI machinery. `solver_validation` also carries the
// `set_parameter` constraint-validation surface, which this staticlib does not
// call; `allow(dead_code)` covers that intentionally-unused subset rather than
// fragmenting the module.
#![allow(dead_code)]

mod runtime_c_abi;
mod solver_validation;
