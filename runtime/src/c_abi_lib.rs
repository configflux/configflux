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
//     fail-closed wrapper, the `ccm_usable_for_open` predicate, and the
//     snapshot→facet-assignment projection (all also used by the runtime CLI —
//     single enforcement point).
//   - `write_enforcement`: the three `*_with_solver_validation` write wrappers
//     the ABI dispatches to (configflux-jraj, ADR-0017 amendment D7). Before
//     that change the ABI called the raw `compiler::runtime_api` write functions
//     and ran no constraint check at all, so every C++/ROS2 SDK write was
//     unchecked. The wrappers live in their own module rather than in
//     `cli_adapter` precisely so this crate root can reach them.
//   - `runtime_c_abi`: the `#[no_mangle] extern "C"` exports themselves.
//
// It does NOT include `cli_adapter` (the clap-based CLI shell), so the staticlib
// stays free of CLI machinery. Some of what these modules carry is unused from
// here; `allow(dead_code)` covers that intentionally-unused subset rather than
// fragmenting the modules.
#![allow(dead_code)]

mod runtime_c_abi;
mod solver_validation;
mod write_enforcement;
