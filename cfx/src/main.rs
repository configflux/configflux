// SPDX-License-Identifier: BUSL-1.1
//
// `cfx` — a unified one-shot presentation CLI over the ConfigFlux resolution
// pipeline (ADR-0042). It ships the `resolve` verb (configflux-2awb.2 / CFX-1):
// open a compiled model, apply a selection, resolve, and export the resolved
// snapshot to a directory in one command, printing the hash lineage; and the
// `options` verb (configflux-2awb.4 / CFX-3): list every facet and its
// currently-valid options after a (possibly empty) partial selection.
//
// `cfx` is presentation only: it parses arguments, builds request envelopes,
// and renders results, never reimplementing resolution/selection/export. It
// composes the `compiler::loader_api` entry points for the loader-only ops and
// routes the solver-authoritative `options` valid set through the shared
// `session_compose` crate (ADR-0042 §2 amended, ADR-0003 §2 amended). It reaches
// the solver ONLY through `session_compose` — no direct `//solver` edge — so
// consuming it makes `cfx` transitively link CUDD, like the interpreter. The
// envelope-speaking `interpreter` binary remains the machine/agent seam; `cfx`
// is the human seam. See docs/adrs/0042-cfx-unified-cli.md.

mod explain;
mod options;
mod pipeline;
mod render;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{error::ErrorKind, Args, Parser, Subcommand, ValueEnum};

use compiler::product_api::OperationStatus;
use explain::ExplainOutcome;
use pipeline::{parse_select_pair, PipelineError, SelectPair};

/// Exit code contract (ADR-0042 §3): `0` success; `2` usage/IO error; `3`
/// valid-input-but-unsatisfiable (the "run cfx explain" case). `cfx` never
/// uses exit `1` — the interpreter's transport-error code has no analogue here.
pub const EXIT_OK: u8 = 0;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_UNSAT: u8 = 3;

#[derive(Parser)]
#[command(name = "cfx")]
#[command(version)]
#[command(about = "ConfigFlux one-shot resolver \u{2014} resolve a compiled model in one command")]
#[command(after_help = "Exit codes: 0=success, 2=usage/IO error, 3=unsatisfiable selection")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a compiled model, apply a selection, resolve, and export the
    /// resolved snapshot to a directory; print the hash lineage.
    Resolve(ResolveArgs),
    /// Open a compiled model, apply a (possibly empty) partial selection, and
    /// list every facet with its currently-valid options — the guided-walk
    /// primitive. No output directory: this is a read-only query.
    Options(OptionsArgs),
    /// Open a compiled model, apply a selection, and explain WHY it is
    /// unsatisfiable — the minimal conflicting-constraint set as human text
    /// (default) or the explain JSON envelope (`--format json`). Exits 3 when
    /// the selection is satisfiable (nothing to explain).
    Explain(ExplainArgs),
}

#[derive(Args)]
struct ResolveArgs {
    /// Path to the compiled model package (CMP) manifest.
    #[arg(long, value_name = "PATH")]
    model: PathBuf,
    /// A selection choice `FACET=OPTION`. Repeatable; applied on top of the
    /// selection file (flags override the file).
    #[arg(long = "select", value_name = "FACET=OPTION")]
    select: Vec<String>,
    /// A selection-state JSON file supplying the scope, context tags, and any
    /// base choices (the existing selection JSON shape).
    #[arg(long = "selection-file", value_name = "PATH")]
    selection_file: Option<PathBuf>,
    /// Directory to write the exported resolved snapshot into.
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// Output format for stdout (files are written to --out regardless).
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Args)]
struct OptionsArgs {
    /// Path to the compiled model package (CMP) manifest.
    #[arg(long, value_name = "PATH")]
    model: PathBuf,
    /// A selection choice `FACET=OPTION`. Repeatable; applied on top of the
    /// selection file (flags override the file). The listing shows the options
    /// still valid after these choices.
    #[arg(long = "select", value_name = "FACET=OPTION")]
    select: Vec<String>,
    /// A selection-state JSON file supplying the scope, context tags, and any
    /// base choices (the existing selection JSON shape).
    #[arg(long = "selection-file", value_name = "PATH")]
    selection_file: Option<PathBuf>,
    /// Output format for stdout.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Args)]
struct ExplainArgs {
    /// Path to the compiled model package (CMP) manifest.
    #[arg(long, value_name = "PATH")]
    model: PathBuf,
    /// A selection choice `FACET=OPTION`. Repeatable; applied on top of the
    /// selection file (flags override the file). The first choice the solver
    /// rejects is the one explained.
    #[arg(long = "select", value_name = "FACET=OPTION")]
    select: Vec<String>,
    /// A selection-state JSON file supplying the scope, context tags, and any
    /// base choices (the existing selection JSON shape).
    #[arg(long = "selection-file", value_name = "PATH")]
    selection_file: Option<PathBuf>,
    /// Output format for stdout.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

fn main() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    ExitCode::from(run(std::env::args_os(), &mut stdout, &mut stderr))
}

/// Parse and dispatch. Returns the process exit code. Split from `main` so
/// integration/unit tests can drive it with explicit args and captured
/// streams.
fn run<I, S, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> u8
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
    W: Write,
    E: Write,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => return render_parse_error(err, stdout, stderr),
    };

    match cli.command {
        Commands::Resolve(args) => run_resolve(args, stdout, stderr),
        Commands::Options(args) => run_options(args, stdout, stderr),
        Commands::Explain(args) => run_explain(args, stdout, stderr),
    }
}

/// Parse every `--select facet=option` pair up front so a malformed pair fails
/// fast (exit 2) before any model I/O.
fn parse_selects(raw: &[String]) -> Result<Vec<SelectPair>, PipelineError> {
    raw.iter().map(|arg| parse_select_pair(arg)).collect()
}

/// Finish a render shared by every verb: on a stdout write failure report the
/// canonical `cfx: failed to write output` (exit 2), otherwise return `ok_code`.
fn finish<E: Write>(render_result: std::io::Result<()>, ok_code: u8, stderr: &mut E) -> u8 {
    match render_result {
        Ok(()) => ok_code,
        Err(_) => {
            let _ = writeln!(stderr, "cfx: failed to write output");
            EXIT_USAGE
        }
    }
}

/// clap parse-error handling mapped onto the `cfx` exit contract: `--help` /
/// `--version` are success (`0`) on stdout; every other parse failure is a
/// usage error (`2`) on stderr.
fn render_parse_error<W: Write, E: Write>(err: clap::Error, stdout: &mut W, stderr: &mut E) -> u8 {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let _ = write!(stdout, "{err}");
            EXIT_OK
        }
        _ => {
            let _ = write!(stderr, "{err}");
            EXIT_USAGE
        }
    }
}

fn run_resolve<W: Write, E: Write>(args: ResolveArgs, stdout: &mut W, stderr: &mut E) -> u8 {
    let file = args.selection_file.as_deref();
    let selects = match parse_selects(&args.select) {
        Ok(pairs) => pairs,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    let outcome = match pipeline::run(&args.model, file, &selects, &args.out) {
        Ok(outcome) => outcome,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    let render_result = match args.format {
        Format::Text => render::render_text(&outcome, stdout),
        Format::Json => render::render_json(&outcome, stdout),
    };
    finish(render_result, EXIT_OK, stderr)
}

fn run_options<W: Write, E: Write>(args: OptionsArgs, stdout: &mut W, stderr: &mut E) -> u8 {
    let file = args.selection_file.as_deref();
    let selects = match parse_selects(&args.select) {
        Ok(pairs) => pairs,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    let outcome = match options::run(&args.model, file, &selects) {
        Ok(outcome) => outcome,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    let render_result = match args.format {
        Format::Text => render::render_options_text(&outcome, stdout),
        Format::Json => render::render_options_json(&outcome, stdout),
    };
    finish(render_result, EXIT_OK, stderr)
}

fn run_explain<W: Write, E: Write>(args: ExplainArgs, stdout: &mut W, stderr: &mut E) -> u8 {
    let file = args.selection_file.as_deref();
    let selects = match parse_selects(&args.select) {
        Ok(pairs) => pairs,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    let result = match explain::run(&args.model, file, &selects) {
        // Satisfiable — nothing to explain (exit 3, ADR-0042 §3). The distinct
        // code lets a caller tell "explained" (0) from "no conflict" (3).
        Ok(ExplainOutcome::Satisfiable) => {
            let _ = writeln!(stdout, "selection is satisfiable; nothing to explain");
            return EXIT_UNSAT;
        }
        // Same "unsatisfiable" verdict `cfx resolve` reaches, but explained
        // (exit 0, ADR-0042 §3): text names the unbound tag(s), JSON emits the
        // existing ResolveResult failure envelope unmodified (configflux-sc69).
        Ok(ExplainOutcome::ResolveContextUnsatisfied(conflict)) => {
            let rendered =
                render::render_resolve_context(&conflict, args.format == Format::Json, stdout);
            return finish(rendered, EXIT_OK, stderr);
        }
        Ok(ExplainOutcome::Explained(result)) => result,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    // A produced explanation is exit 0 (ADR-0031 D2: a rejection is the success
    // path). A status-Error result means the explain could not run at all (no
    // usable .ccm, solver fault) — a usage/IO-class failure, exit 2. JSON emits
    // the envelope UNMODIFIED regardless of status (byte-identical to the
    // interpreter path); text renders the core on success, else surfaces the
    // fail-closed diagnostic on stderr.
    let ok = result.status == OperationStatus::Ok;
    let render_result = match (args.format, ok) {
        (Format::Json, _) => render::render_explain_json(&result, stdout),
        (Format::Text, true) => render::render_explain_text(&result, stdout),
        (Format::Text, false) => {
            let _ = writeln!(stderr, "cfx: {}", result.rejection.message);
            Ok(())
        }
    };
    finish(render_result, if ok { EXIT_OK } else { EXIT_USAGE }, stderr)
}

/// Rebuild the `cfx explain` command that reproduces the selection the user just
/// had rejected.
///
/// EVERY input that shaped the selection must appear, or the suggestion points
/// at a DIFFERENT one (configflux-0qk2). `--selection-file` is not optional
/// detail: it carries the scope and the immutable context tags, neither of which
/// any `--select` pair can express, and it may carry choices the flags never
/// mention. Reconstructing from the flags alone yields a command that — run
/// verbatim — either reports the refused selection satisfiable or confidently
/// explains an unrelated failure. File first, then flags: the same precedence
/// the verbs apply (ADR-0042 §2).
fn explain_hint(
    model: &std::path::Path,
    selection_file: Option<&std::path::Path>,
    selects: &[String],
) -> String {
    let mut hint = format!("cfx explain --model {}", model.display());
    if let Some(path) = selection_file {
        hint.push_str(&format!(" --selection-file {}", path.display()));
    }
    for pair in selects {
        hint.push_str(&format!(" --select {pair}"));
    }
    hint
}

/// Emit a pipeline error to stderr and return its exit code: the REASON first,
/// then — on the unsatisfiable path (exit `3`) — the canonical guidance line
/// pointing at `cfx explain` with the user's own flags (ADR-0042 §2). The reason
/// names the violated constraint and quotes it (ADR-0054 §6): exit 3 owes a why.
fn emit_error<E: Write>(
    err: PipelineError,
    model: &std::path::Path,
    selection_file: Option<&std::path::Path>,
    selects: &[String],
    stderr: &mut E,
) -> u8 {
    let _ = writeln!(stderr, "cfx: {}", err.message);
    if err.unsatisfiable {
        let _ = writeln!(
            stderr,
            "selection is unsatisfiable; run: {}",
            explain_hint(model, selection_file, selects)
        );
    }
    err.exit_code
}
