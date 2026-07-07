// SPDX-License-Identifier: BUSL-1.1

mod cli_adapter;
// Human-readable unsat-core renderer (ADR-0031 D5, configflux-9d28) now lives in
// the shared `//interpreter:explain_renderer` library (configflux-2awb.3 / CFX-2)
// so the interpreter binary and `cfx` consume one renderer. The binary links it
// via COMMON_DEPS; the envelope path does not call it (the machine envelope
// never depends on the rendered text — ADR-0031 D2/D5).

#[cfg(test)]
use cli_adapter::*;
#[cfg(test)]
mod tests;

fn main() -> std::process::ExitCode {
    cli_adapter::main_entry()
}
