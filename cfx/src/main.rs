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
// is the human seam. See docs/adrs/0042-cfx-unified-cli.md, and ADR-0059 for
// the environment manifest `resolve` reads (`src/manifest.rs`) and the `diff`
// verb that compares two models per deployment target (`src/diff.rs`).

mod diff;
#[cfg(test)]
mod diff_tests;
mod explain;
mod input_file;
mod manifest;
mod options;
mod pipeline;
mod render;
mod selection_input;
#[cfg(test)]
mod selection_input_tests;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{error::ErrorKind, Args, Parser, Subcommand, ValueEnum};

use compiler::product_api::OperationStatus;
use explain::ExplainOutcome;
use manifest::Cell;
use pipeline::{CellSource, PipelineError, SelectPair};
use selection_input::parse_selects;


/// Exit code contract (ADR-0042 §3, widened by ADR-0059 D4): `0` success;
/// `1` differences found (`cfx diff` only); `2` usage/IO error; `3`
/// valid-input-but-unsatisfiable (the "run cfx explain" case).
///
/// `1` was documented here as a code `cfx` never uses, on the grounds that the
/// interpreter's transport-error code had no analogue. `cfx diff` gives it an
/// unrelated meaning — the `diff(1)` / `git diff --exit-code` convention, where
/// `1` says "there IS a difference" and is an expected outcome, not a failure.
/// That is what lets a pull-request check be the bare command. `cfx diff` never
/// exits `3`: unsatisfiability is a per-cell status there, not a failure.
pub const EXIT_OK: u8 = 0;
pub const EXIT_DIFFERENCES: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_UNSAT: u8 = 3;

#[derive(Parser)]
#[command(name = "cfx")]
#[command(version)]
#[command(about = "ConfigFlux one-shot resolver \u{2014} resolve a compiled model in one command")]
#[command(after_help = "Exit codes: 0=success, 1=diff found differences, 2=usage/IO error, \
                        3=unsatisfiable selection")]
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
    /// Compare two compiled models across the deployment targets an
    /// environment manifest names, and report per target whether the delivered
    /// configuration is unchanged, changed, newly unsatisfiable, or newly
    /// satisfiable. Writes no files. Exits 0 only when no target changed.
    Diff(DiffArgs),
}

#[derive(Args)]
struct ResolveArgs {
    /// Path to the compiled model package (CMP) manifest.
    #[arg(long, value_name = "PATH")]
    model: PathBuf,
    /// A selection choice `FACET=OPTION`. Repeatable; applied on top of the
    /// selection file or manifest entry (flags override both).
    #[arg(long = "select", value_name = "FACET=OPTION")]
    select: Vec<String>,
    /// A selection-state JSON file supplying the scope, context tags, and any
    /// base choices (the existing selection JSON shape).
    #[arg(long = "selection-file", value_name = "PATH")]
    selection_file: Option<PathBuf>,
    /// An environment manifest naming deployment targets. Mutually exclusive
    /// with --selection-file; needs exactly one of --environment or --all.
    #[arg(long = "manifest", value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Resolve exactly one environment named by --manifest, into --out.
    #[arg(long = "environment", value_name = "NAME")]
    environment: Option<String>,
    /// Resolve EVERY --manifest environment into `<out>/<env>/<scope-root>/`.
    #[arg(long = "all")]
    all: bool,
    /// Comma-separated scopes replacing every environment's own scope. --all only.
    #[arg(long = "scopes", value_name = "LIST")]
    scopes: Option<String>,
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

#[derive(Args)]
struct DiffArgs {
    /// Path to the BASE compiled model package (CMP) manifest — the side you
    /// are comparing against (typically the one on the main branch).
    #[arg(long, value_name = "PATH")]
    base: PathBuf,
    /// Path to the HEAD compiled model package (CMP) manifest — the side
    /// carrying the change under review.
    #[arg(long, value_name = "PATH")]
    head: PathBuf,
    /// The environment manifest naming the deployment targets to compare.
    #[arg(long = "manifest", value_name = "PATH")]
    manifest: PathBuf,
    /// Compare only this environment. Repeatable; omit to compare EVERY
    /// environment the manifest names.
    #[arg(long = "environment", value_name = "NAME")]
    environment: Vec<String>,
    /// Comma-separated scopes replacing every environment's own scope.
    #[arg(long = "scopes", value_name = "LIST")]
    scopes: Option<String>,
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
/// integration/unit tests can drive it with explicit args and captured streams.
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
        Commands::Diff(args) => run_diff(args, stdout, stderr),
    }
}

/// `cfx diff` (ADR-0059 D4). Narrowing is the M5 model — a SET of environment
/// names, empty meaning every environment — through the SAME `resolve_cells()`
/// the matrix uses, so a diff can never visit a cell set `cfx resolve --all`
/// did not write. `--scopes` always applies here: `diff` has no file-writing
/// single-cell mode for it to be ambiguous against.
fn run_diff<W: Write, E: Write>(args: DiffArgs, stdout: &mut W, stderr: &mut E) -> u8 {
    let scopes = match args.scopes.as_deref().map(manifest::parse_scopes).transpose() {
        Ok(scopes) => scopes,
        Err(err) => return emit_diff_error(err, stderr),
    };
    let cells = match manifest::load(&args.manifest)
        .and_then(|manifest| manifest.resolve_cells(&args.environment, scopes.as_deref()))
    {
        Ok(cells) => cells,
        Err(err) => return emit_diff_error(err, stderr),
    };

    let report = match diff::run(&args.base, &args.head, &cells) {
        Ok(report) => report,
        Err(err) => return emit_diff_error(err, stderr),
    };

    let render_result = match args.format {
        Format::Text => diff::render_text(&report, stdout),
        Format::Json => diff::render_json(&report, stdout),
    };
    finish(render_result, report.exit_code(), stderr)
}

/// A `cfx diff` failure is always usage/IO class: unsatisfiability is a cell
/// status there, never the command's verdict, so there is no `cfx explain`
/// guidance line to print and no exit `3` to reach.
fn emit_diff_error<E: Write>(err: PipelineError, stderr: &mut E) -> u8 {
    let _ = writeln!(stderr, "cfx: {}", err.message);
    EXIT_USAGE
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

    // ADR-0059 D2/M5; `None` is the pre-existing, byte-unchanged path.
    let plan = match manifest::plan(
        args.manifest.as_deref(),
        args.environment.as_deref(),
        args.all,
        args.scopes.as_deref(),
        file,
    ) {
        Ok(plan) => plan,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };
    let (cells, matrix) = match plan {
        Some(plan) => (plan.cells, plan.nested),
        None => (Vec::new(), false),
    };
    if matrix {
        return run_matrix(&args, &cells, &selects, stdout, stderr);
    }

    // One cell, flat `<out>` (M4): adding `--manifest ... --environment prod`
    // to an existing invocation leaves the files exactly where they were.
    let source = match cells.first() {
        Some(cell) => CellSource::Manifest(cell),
        None => CellSource::from_selection_file(file),
    };
    let outcome = match pipeline::run_cell(&args.model, &source, &selects, &args.out) {
        Ok(outcome) => outcome,
        Err(err) => return emit_error(err, &args.model, file, &args.select, stderr),
    };

    let render_result = match args.format {
        Format::Text => render::render_text(&outcome, stdout),
        Format::Json => render::render_json(&outcome, stdout),
    };
    finish(render_result, EXIT_OK, stderr)
}

/// `cfx resolve --manifest ... --all`: the whole environment x scope matrix
/// (ADR-0059 D2). Exit `0` when every cell resolved, `3` when any was rejected
/// as unsatisfiable — resolved cells keep their files, and each rejection is
/// named on stderr with its diagnostic, so a CI log says WHICH targets broke.
/// A usage/IO failure is never a cell status: `run_cells` returns it, aborting.
fn run_matrix<W: Write, E: Write>(
    args: &ResolveArgs,
    cells: &[Cell],
    selects: &[SelectPair],
    stdout: &mut W,
    stderr: &mut E,
) -> u8 {
    let outcomes = match pipeline::run_cells(&args.model, cells, selects, &args.out) {
        Ok(outcomes) => outcomes,
        Err(err) => return emit_error(err, &args.model, None, &args.select, stderr),
    };

    let mut code = EXIT_OK;
    for cell in &outcomes {
        if let Err(err) = &cell.result {
            let (env, scope) = (&cell.environment, &cell.scope);
            let _ = writeln!(stderr, "unsatisfiable: {env} {scope}: {}", err.message);
            code = EXIT_UNSAT;
        }
    }

    let render_result = match args.format {
        Format::Text => render::render_cells_text(&outcomes, stdout),
        Format::Json => render::render_cells_json(&outcomes, stdout),
    };
    finish(render_result, code, stderr)
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

/// POSIX single-quote one term the hint interpolates, so the printed command is
/// the command that runs.
///
/// UNCONDITIONAL (configflux-zz2g): every term is wrapped, whether or not it
/// holds a metacharacter. Quoting only the terms that need it would make the
/// printed form depend on which directory the user happens to be working in and
/// on which ids their model happens to declare, and a line that is quoted some
/// of the time is one the reader has to inspect before pasting — which is the
/// doubt the hint exists to remove. Inside single quotes a POSIX shell honours
/// no escape at all, so an embedded `'` can only be encoded by closing,
/// emitting an escaped quote, and reopening.
///
/// EVERY term, not only the paths (configflux-egyj). zz2g left the `--select`
/// pairs raw on the reasoning that they are not paths, but being a path is not
/// what made them dangerous — holding a space was. Nothing screens the
/// characters in a facet or option id: a facet's `values` are a plain list of
/// strings in the CUE authoring schema AND in the Rust ingest, and the link
/// verifier checks only non-emptiness, uniqueness and default-membership. So a
/// model may legitimately declare `de bug`, and the rejection that names it
/// printed a line a shell re-split into arguments `cfx explain` never sees. The
/// whole `facet=option` is one word: `pipeline::parse_select_pair` splits it on
/// the first `=` itself, so the pair has to arrive as a single argv entry.
fn shell_quote(term: &str) -> String {
    format!("'{}'", term.replace('\'', r"'\''"))
}

/// The same encoding for a term that arrives as a path — the ONE quoting rule,
/// reached through an adapter rather than restated, so the two kinds of term
/// can never disagree about what a shell does with a quote.
///
/// `display()` is LOSSY: a path that is not valid UTF-8 renders its invalid
/// bytes as U+FFFD, so the quoted command names a file that does not exist.
/// That is a deliberate WONTFIX (configflux-egyj). The alternative is to make
/// this fallible — to withhold the pointer, or to print a different one,
/// because of the bytes in a path — and a diagnostic must not gain a failure
/// mode at the moment the user is already being refused. The reason line above
/// the hint names the violated constraint either way, so a user on such a path
/// is left with the cause even if the pointer needs retyping.
fn shell_quote_path(path: &std::path::Path) -> String {
    shell_quote(&path.display().to_string())
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
    let mut hint = format!("cfx explain --model {}", shell_quote_path(model));
    if let Some(path) = selection_file {
        hint.push_str(&format!(" --selection-file {}", shell_quote_path(path)));
    }
    for pair in selects {
        hint.push_str(&format!(" --select {}", shell_quote(pair)));
    }
    hint
}

/// Emit a pipeline error to stderr and return its exit code: the REASON first,
/// then — on the unsatisfiable path (exit `3`) — the canonical guidance line
/// pointing at `cfx explain` with the user's own flags (ADR-0042 §2). The reason
/// names the violated constraint and quotes it (ADR-0054 §6): exit 3 owes a why.
///
/// Exit `2` earns a guidance line too, on the ONE usage refusal that can say
/// where to look next (configflux-ineg): a `--select` naming a declared facet
/// with a value outside its domain — the commonest selection typo, and until
/// now the only refusal that handed the user nowhere to go. It points at
/// `cfx options`, which lists what IS valid. It does NOT reclassify: a value
/// that does not exist is a usage mistake, not a model that cannot be
/// satisfied, so the code stays `2` and the unsat branch keeps exit `3` to
/// itself.
///
/// The command is `cfx options --model <m>` and not `... --facet <f>`:
/// `OptionsArgs` declares no `--facet`, so the flag form would print a line
/// clap answers with `unexpected argument '--facet' found`, which defeats the
/// whole point of a hint — that the printed command is the command that runs.
/// The listing covers every facet, the named one included, so the facet is
/// carried in the verdict half instead. The command stays LAST in both
/// branches so everything after `run: ` can be pasted whole.
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
    } else if let Some(facet) = &err.invalid_option_facet {
        let _ = writeln!(
            stderr,
            "facet {} has no such option; run: cfx options --model {}",
            shell_quote(facet),
            shell_quote_path(model)
        );
    }
    err.exit_code
}
