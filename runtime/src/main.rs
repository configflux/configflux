// SPDX-License-Identifier: BUSL-1.1

mod cli_adapter;
// Runtime `explain-rejection` solver wrapper (configflux-3b5y, ADR-0031). Like
// `solver_validation`, it depends on the `solver` crate and converts the
// solver-owned labeled MUS into the compiler-side `UnsatCore`. CLI-only — not
// part of the C ABI staticlib root (`c_abi_lib.rs`).
mod explain_rejection;
// Human-readable unsat-core renderer (ADR-0031 D5, configflux-9d28). A pure,
// layered presentation helper over the compiler-side `UnsatCore`; the machine
// envelope never depends on it. CLI-only, like `explain_rejection` — not part of
// the C ABI staticlib root (`c_abi_lib.rs`).
mod explain_renderer;
// The runtime C ABI lives in the runtime crate (not the compiler crate) so its
// open entrypoint can enforce the ADR-0030 D2 `.ccm` solver-model precondition,
// which needs the `solver` crate the compiler may not import (ADR-0003 §2;
// configflux-u32v). The binary does not call these `#[no_mangle]` exports; they
// are linked into the `runtime_c_abi_static` staticlib (see runtime/BUILD.bazel)
// and exercised by the runtime test crate.
mod runtime_c_abi;
mod solver_validation;

#[cfg(test)]
use cli_adapter::*;
#[cfg(test)]
mod tests;

fn main() -> std::process::ExitCode {
    cli_adapter::main_entry()
}
