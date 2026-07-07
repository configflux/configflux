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
    let selects = match parse_selects(&args.select) {
        Ok(pairs) => pairs,
        Err(err) => return emit_error(err, &args.model, &args.select, stderr),
    };

    let outcome = match pipeline::run(
        &args.model,
        args.selection_file.as_deref(),
        &selects,
        &args.out,
    ) {
        Ok(outcome) => outcome,
        Err(err) => return emit_error(err, &args.model, &args.select, stderr),
    };

    let render_result = match args.format {
        Format::Text => render::render_text(&outcome, stdout),
        Format::Json => render::render_json(&outcome, stdout),
    };
    if render_result.is_err() {
        let _ = writeln!(stderr, "cfx: failed to write output");
        return EXIT_USAGE;
    }
    EXIT_OK
}

fn run_options<W: Write, E: Write>(args: OptionsArgs, stdout: &mut W, stderr: &mut E) -> u8 {
    let selects = match parse_selects(&args.select) {
        Ok(pairs) => pairs,
        Err(err) => return emit_error(err, &args.model, &args.select, stderr),
    };

    let outcome = match options::run(&args.model, args.selection_file.as_deref(), &selects) {
        Ok(outcome) => outcome,
        Err(err) => return emit_error(err, &args.model, &args.select, stderr),
    };

    let render_result = match args.format {
        Format::Text => render::render_options_text(&outcome, stdout),
        Format::Json => render::render_options_json(&outcome, stdout),
    };
    if render_result.is_err() {
        let _ = writeln!(stderr, "cfx: failed to write output");
        return EXIT_USAGE;
    }
    EXIT_OK
}

fn run_explain<W: Write, E: Write>(args: ExplainArgs, stdout: &mut W, stderr: &mut E) -> u8 {
    let selects = match parse_selects(&args.select) {
        Ok(pairs) => pairs,
        Err(err) => return emit_error(err, &args.model, &args.select, stderr),
    };

    let result = match explain::run(&args.model, args.selection_file.as_deref(), &selects) {
        // Satisfiable — nothing to explain (exit 3, ADR-0042 §3). The distinct
        // code lets a caller tell "explained" (0) from "no conflict" (3).
        Ok(ExplainOutcome::Satisfiable) => {
            let _ = writeln!(stdout, "selection is satisfiable; nothing to explain");
            return EXIT_UNSAT;
        }
        Ok(ExplainOutcome::Explained(result)) => result,
        Err(err) => return emit_error(err, &args.model, &args.select, stderr),
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
    if render_result.is_err() {
        let _ = writeln!(stderr, "cfx: failed to write output");
        return EXIT_USAGE;
    }
    if ok {
        EXIT_OK
    } else {
        EXIT_USAGE
    }
}

/// Emit a pipeline error to stderr and return its exit code. On the
/// unsatisfiable path (exit `3`), append the canonical guidance line pointing
/// at `cfx explain` with the user's own selection flags (ADR-0042 §2).
fn emit_error<E: Write>(
    err: PipelineError,
    model: &std::path::Path,
    selects: &[String],
    stderr: &mut E,
) -> u8 {
    if err.unsatisfiable {
        let mut hint = format!(
            "selection is unsatisfiable; run: cfx explain --model {}",
            model.display()
        );
        for pair in selects {
            hint.push_str(&format!(" --select {pair}"));
        }
        let _ = writeln!(stderr, "{hint}");
    } else {
        let _ = writeln!(stderr, "cfx: {}", err.message);
    }
    err.exit_code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> (u8, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(args.iter().map(|s| s.to_string()), &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn no_subcommand_is_usage_error() {
        let (code, _out, _err) = run_args(&["cfx"]);
        assert_eq!(code, EXIT_USAGE);
    }

    #[test]
    fn help_is_success() {
        let (code, out, _err) = run_args(&["cfx", "--help"]);
        assert_eq!(code, EXIT_OK);
        assert!(out.contains("resolve"));
        assert!(out.contains("options"), "help must list the options verb: {out}");
        assert!(out.contains("explain"), "help must list the explain verb: {out}");
    }

    #[test]
    fn explain_malformed_select_is_usage_error_naming_arg() {
        let (code, _out, err) = run_args(&[
            "cfx", "explain", "--model", "/nonexistent/cmp.json", "--select", "bogus",
        ]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("bogus"), "stderr must name the bad arg: {err}");
    }

    #[test]
    fn explain_missing_model_file_is_usage_error() {
        let (code, _out, err) = run_args(&[
            "cfx",
            "explain",
            "--model",
            "/nonexistent/cmp.manifest.json",
        ]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.starts_with("cfx:"), "stderr: {err}");
    }

    #[test]
    fn options_malformed_select_is_usage_error_naming_arg() {
        let (code, _out, err) = run_args(&[
            "cfx", "options", "--model", "/nonexistent/cmp.json", "--select", "bogus",
        ]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("bogus"), "stderr must name the bad arg: {err}");
    }

    #[test]
    fn options_missing_model_file_is_usage_error() {
        let (code, _out, err) = run_args(&[
            "cfx",
            "options",
            "--model",
            "/nonexistent/cmp.manifest.json",
        ]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.starts_with("cfx:"), "stderr: {err}");
    }

    #[test]
    fn malformed_select_is_usage_error_naming_arg() {
        let (code, _out, err) = run_args(&[
            "cfx", "resolve", "--model", "/nonexistent/cmp.json", "--out", "/tmp/x", "--select",
            "bogus",
        ]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("bogus"), "stderr must name the bad arg: {err}");
    }

    #[test]
    fn missing_model_file_is_usage_error() {
        let dir = std::env::temp_dir().join(format!("cfx-test-{}", std::process::id()));
        let (code, _out, err) = run_args(&[
            "cfx",
            "resolve",
            "--model",
            "/nonexistent/cmp.manifest.json",
            "--out",
            dir.to_str().unwrap(),
        ]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.starts_with("cfx:"), "stderr: {err}");
    }
}
