// SPDX-License-Identifier: BUSL-1.1

use anyhow::{Context, Result};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use compiler::loader_api::{open_model, OpenModelRequest};
use compiler::product_api::{
    compile_model, compile_model_with_progress, inspect_model, verify_model, CompileModelRequest,
    InspectModelRequest, InspectQuery, OperationStatus, SourceManifestEntry, VerifyModelRequest,
    PRODUCT_SCHEMA_VERSION,
};
use compiler::progress::{ProgressEvent, ProgressSink};
use compiler::resource_budget::ResourceBudget;
use serde::Serialize;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "configflux-compiler")]
#[command(version)]
#[command(about = "ConfigFlux configuration model compiler")]
#[command(after_help = "Exit codes: 0=success, 1=user/input error, 2=compilation error")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile source chunks (CUE-authored, exported to JSON) into a deployable model package
    ///
    /// Example: configflux-compiler compile --source defs.json --source comps.json --out out/cmp
    Compile(CompileArgs),
    /// Verify a configuration model for structural and semantic correctness
    ///
    /// Example: configflux-compiler verify --source defs.json --source comps.json
    Verify(VerifyArgs),
    /// Inspect model internals: components, definitions, artifacts, parameters
    ///
    /// Example: configflux-compiler inspect --source defs.json --source comps.json summary
    Inspect(InspectArgs),
    /// Open a compiled model package (.cmp manifest) for runtime handoff
    ///
    /// Example: configflux-compiler open-model --cmp-manifest out/cmp/manifest.json
    OpenModel(OpenModelArgs),
}

#[derive(Args)]
struct CompileArgs {
    /// Path to a source chunk: CUE-authored, exported to JSON (repeat for multiple chunks)
    #[arg(long = "source", required = true, action = ArgAction::Append)]
    sources: Vec<PathBuf>,
    /// Output directory for the compiled model package
    #[arg(long = "out", default_value = "out/cmp")]
    out_dir: PathBuf,
    /// Target maximum number of distinct variables per BDD partition
    /// (configflux-0qo3, ADR-0012 §2). When omitted, defaults to
    /// `usize::MAX` so every model collapses to a single partition
    /// (full backward compatibility). End-to-end wiring through the
    /// v2 multi-part emission path lands in configflux-vmlb; this
    /// flag is intentionally a no-op end-to-end today and is plumbed
    /// here so the CLI surface is present on `--help` before the
    /// emitter rewires.
    #[arg(long = "cluster-size", default_value_t = usize::MAX)]
    cluster_size: usize,
    /// Soft target peak resident memory in MiB (configflux-9pjy.2,
    /// ADR-0039). A *soft* budget the compiler honours by trading
    /// wall-clock for a lower peak (it never kills the build — the
    /// kernel-aware resource guard remains the hard backstop). When set,
    /// the compiler shrinks the byte-neutral apply-memo cache and, if the
    /// projected node table would still exceed the budget, partitions the
    /// model (an explicit `--cluster-size` always wins). When omitted, the
    /// compile is byte-for-byte identical to today.
    #[arg(long = "max-rss-mb")]
    max_rss_mb: Option<u64>,
    /// Maximum thread count for the parallel compile/solve paths
    /// (configflux-9pjy.2, ADR-0039). Honestly narrow: the in-crate BDD
    /// compile is single-threaded, so this knob only bites where
    /// parallelism actually exists (the CUDD compile path and the runtime
    /// solver session). It does NOT reduce the in-crate compile's CPU
    /// peak.
    #[arg(long = "max-threads")]
    max_threads: Option<u32>,
    /// Compile-time progress signal (configflux-9pjy.3, ADR-0039 §7).
    /// `none` (default) emits nothing and is byte-for-byte identical to a
    /// compile without the flag. `plain` writes human-readable phase /
    /// percent / RSS / ETA lines to STDERR. `json` writes a JSON-lines
    /// progress stream to STDOUT (the parsed contract, ADR-0005
    /// Amendment 2). Progress never enters the compiled `.ccm` artifact.
    #[arg(long = "progress", value_enum, default_value_t = ProgressMode::None)]
    progress: ProgressMode,
    /// Stamp the emitted `provenance.json` sidecars with a wall-clock
    /// `stamped_at` (ADR-0044 D1, configflux-pq2w.1). OFF by default so the
    /// compile stays byte-stable end-to-end (same inputs → same bytes,
    /// sidecar included). The timestamp never enters any hashed artifact.
    #[arg(long = "stamp-time", default_value_t = false)]
    stamp_time: bool,
}

/// The `--progress` output mode (configflux-9pjy.3 / ADR-0039 §7).
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ProgressMode {
    /// No progress output; byte-identical to a compile without the flag.
    None,
    /// Human-readable progress lines on stderr.
    Plain,
    /// JSON-lines progress stream on stdout (the parsed contract).
    Json,
}

#[derive(Args)]
struct VerifyArgs {
    /// Path to a source chunk: CUE-authored, exported to JSON (repeat for multiple chunks)
    #[arg(long = "source", required = true, action = ArgAction::Append)]
    sources: Vec<PathBuf>,
}

#[derive(Args)]
struct InspectArgs {
    /// Path to a source chunk: CUE-authored, exported to JSON (repeat for multiple chunks)
    #[arg(long = "source", required = true, action = ArgAction::Append)]
    sources: Vec<PathBuf>,
    #[command(subcommand)]
    query: InspectQueryArgs,
}

#[derive(Args)]
struct OpenModelArgs {
    /// Path to the compiled model package manifest
    #[arg(long = "cmp-manifest", required = true)]
    cmp_manifest: PathBuf,
}

#[derive(Subcommand)]
enum InspectQueryArgs {
    /// Print a high-level model summary (component and definition counts)
    Summary,
    /// Show details for a specific component
    Component {
        /// The component identifier to inspect
        component_id: String,
    },
    /// Show details for a specific definition
    Definition {
        /// The definition identifier to inspect
        definition_id: String,
    },
    /// Show details for a specific artifact
    Artifact {
        /// The artifact identifier to inspect
        artifact_id: String,
    },
    /// Show a parameter's resolved metadata within a component
    Parameter {
        /// The component that owns the parameter
        component_id: String,
        /// The parameter key within the component
        param_key: String,
    },
    /// Show statistics scoped to a component, platform, or region
    ScopedStats {
        /// Scope selector (e.g. "component:thermal_control", "platform:all")
        scope: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("E_COMPILER_CLI_INPUT: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            match err.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    print!("{err}");
                    return Ok(ExitCode::from(0));
                }
                _ => {
                    eprintln!("E_COMPILER_CLI_ARGS_INVALID: Invalid CLI arguments. Use --help for usage.");
                    return Ok(ExitCode::from(1));
                }
            }
        }
    };
    match cli.command {
        Commands::Compile(args) => run_compile(args),
        Commands::Verify(args) => run_verify(args),
        Commands::Inspect(args) => run_inspect(args),
        Commands::OpenModel(args) => run_open_model(args),
    }
}

/// The product CLI's `compile` always builds in-crate (the single-threaded
/// `BddBuilder` recursion; `compiler_core` hardcodes the `"in-crate"`
/// construction). Per ADR-0039 §6 / §Context honesty flag, `--max-threads`
/// is therefore **inert** on a pure in-crate compile: it cannot shrink that
/// compile's ~1-core CPU peak. Rather than silently imply CPU scaling that
/// cannot happen, return a warning string when the operator sets the knob.
///
/// Pure and testable: returns `Some(message)` exactly when `max_threads` is
/// set, `None` otherwise. `run_compile` prints the message once (guarded by
/// a [`std::sync::Once`]) so a single invocation warns at most once.
fn in_crate_threads_warning(max_threads: Option<u32>) -> Option<String> {
    max_threads.map(|n| {
        format!(
            "warning: --max-threads={n} is inert on this in-crate compile. \
             The in-crate BDD builder is single-threaded, so the thread knob \
             cannot reduce its CPU peak; it bites only where parallelism \
             exists (the runtime solver session). RAM is the configurable \
             peak here — use --max-rss-mb to trade wall-clock for a lower \
             resident set."
        )
    })
}

/// Emit the in-crate `--max-threads` inertness warning to stderr at most
/// once per process (configflux-9pjy.4 / ADR-0039 §6).
fn warn_in_crate_threads_once(max_threads: Option<u32>) {
    use std::sync::Once;
    static WARNED: Once = Once::new();
    if let Some(message) = in_crate_threads_warning(max_threads) {
        WARNED.call_once(|| eprintln!("{message}"));
    }
}

/// Build the soft [`ResourceBudget`] from the parsed `--max-rss-mb` /
/// `--max-threads` flags (configflux-9pjy.2 / ADR-0039). Returns `None`
/// when BOTH knobs are unset, so the resulting `CompileModelRequest` carries
/// NO `budget` key for an unbudgeted compile — a byte-identical wire surface
/// that mirrors how `--cluster-size` is dropped when omitted.
///
/// Pure and testable: a straight mapping from the two optional flags to the
/// optional budget, with no I/O. `run_compile` calls this to assemble the
/// request; the unit test pins the parse → budget round-trip.
fn budget_from_args(max_rss_mb: Option<u64>, max_threads: Option<u32>) -> Option<ResourceBudget> {
    if max_rss_mb.is_none() && max_threads.is_none() {
        None
    } else {
        Some(ResourceBudget {
            max_rss_mb,
            max_threads,
        })
    }
}

fn run_compile(args: CompileArgs) -> Result<ExitCode> {
    let source_manifest = load_sources(&args.sources)?;
    // configflux-9pjy.4 / ADR-0039 §6: warn (once) that --max-threads does
    // nothing for a pure in-crate compile, rather than implying CPU scaling
    // that cannot happen. Output and behaviour are otherwise unchanged.
    warn_in_crate_threads_once(args.max_threads);
    // configflux-vmlb: cluster_size = usize::MAX collapses to a
    // single partition (the no-flag default). Pass `None` in that
    // case so the canonical JSON form of CompileModelRequest does
    // not carry a redundant `cluster_size: 18446744073709551615`
    // field; the wire surface stays clean.
    let cluster_size = if args.cluster_size == usize::MAX {
        None
    } else {
        Some(args.cluster_size)
    };
    // configflux-9pjy.2 / ADR-0039: build the soft budget only when at
    // least one knob is set, so the canonical JSON form of
    // CompileModelRequest carries NO `budget` key for an unbudgeted
    // compile (byte-identical wire surface, mirroring `cluster_size`).
    let budget = budget_from_args(args.max_rss_mb, args.max_threads);
    let request = CompileModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest,
        output_dir: Some(args.out_dir.to_string_lossy().into_owned()),
        cluster_size,
        budget,
        stamp_time: args.stamp_time,
    };
    // configflux-9pjy.3 / ADR-0039 §7: select the progress sink. `none`
    // takes the plain `compile_model` path with NO sink wired — byte-for-
    // byte identical to a compile without the flag (ADR-0005 Amendment 2).
    let result = match args.progress {
        ProgressMode::None => compile_model(request),
        ProgressMode::Plain => {
            let sink = PlainStderrSink;
            compile_model_with_progress(request, Some(&sink))
        }
        ProgressMode::Json => {
            let sink = JsonStdoutSink;
            compile_model_with_progress(request, Some(&sink))
        }
    };
    print_json(&result)?;
    Ok(status_exit_code(result.status))
}

/// `--progress plain` sink: one human-readable line per event on STDERR.
/// Stderr keeps the progress stream off the parsed stdout surface (the
/// final `CompileResult` JSON), and matches the existing non-contractual
/// `BDD-PROFILE:` / `CUDD-CHECKPOINT:` stderr diagnostics.
struct PlainStderrSink;

impl ProgressSink for PlainStderrSink {
    fn on_event(&self, ev: &ProgressEvent) {
        let rss = match ev.rss_mb {
            Some(mb) => format!("{mb} MiB"),
            None => "n/a".to_string(),
        };
        let eta = match ev.eta_s {
            Some(s) => format!("{s:.1}s"),
            None => "—".to_string(),
        };
        eprintln!(
            "progress: {:>3.0}% phase={:?} clauses={}/{} rss={rss} eta={eta}",
            ev.pct * 100.0,
            ev.phase,
            ev.processed_clauses,
            ev.total_clauses,
        );
    }
}

/// `--progress json` sink: one JSON object per line on STDOUT — the new
/// parsed progress contract (ADR-0005 Amendment 2). Each event is written
/// as a compact, single-line JSON value followed by a newline (JSON-lines).
/// A serialization failure is silently dropped: progress is observational
/// and must never abort or corrupt the compile.
struct JsonStdoutSink;

impl ProgressSink for JsonStdoutSink {
    fn on_event(&self, ev: &ProgressEvent) {
        if let Ok(line) = serde_json::to_string(ev) {
            println!("{line}");
        }
    }
}

fn run_verify(args: VerifyArgs) -> Result<ExitCode> {
    let source_manifest = load_sources(&args.sources)?;
    let report = verify_model(VerifyModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest,
    });
    print_json(&report)?;
    Ok(status_exit_code(report.status))
}

fn run_inspect(args: InspectArgs) -> Result<ExitCode> {
    let source_manifest = load_sources(&args.sources)?;
    let query = inspect_query_from_args(args.query);
    let result = inspect_model(InspectModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        source_manifest,
        query,
    });
    print_json(&result)?;
    Ok(status_exit_code(result.status))
}

fn run_open_model(args: OpenModelArgs) -> Result<ExitCode> {
    let result = open_model(OpenModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        cmp_manifest_ref: args.cmp_manifest.to_string_lossy().into_owned(),
    });
    print_json(&result)?;
    Ok(status_exit_code(result.status))
}

fn load_sources(paths: &[PathBuf]) -> Result<Vec<SourceManifestEntry>> {
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let inline_content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read source '{}'", path.display()))?;
        sources.push(SourceManifestEntry {
            source_id: path.to_string_lossy().into_owned(),
            inline_content,
        });
    }
    Ok(sources)
}

fn inspect_query_from_args(args: InspectQueryArgs) -> InspectQuery {
    match args {
        InspectQueryArgs::Summary => InspectQuery::Summary,
        InspectQueryArgs::Component { component_id } => InspectQuery::Component { component_id },
        InspectQueryArgs::Definition { definition_id } => {
            InspectQuery::Definition { definition_id }
        }
        InspectQueryArgs::Artifact { artifact_id } => InspectQuery::Artifact { artifact_id },
        InspectQueryArgs::Parameter {
            component_id,
            param_key,
        } => InspectQuery::Parameter {
            component_id,
            param_key,
        },
        InspectQueryArgs::ScopedStats { scope } => InspectQuery::ScopedStats { scope },
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value).context("Failed to serialize output JSON")?;
    println!();
    Ok(())
}

fn status_exit_code(status: OperationStatus) -> ExitCode {
    match status {
        OperationStatus::Ok => ExitCode::from(0),
        OperationStatus::Error => ExitCode::from(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parses_inspect_parameter_subcommand() {
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "inspect",
            "--source",
            "defs.json",
            "--source",
            "comps.json",
            "parameter",
            "thermal_control",
            "control_driver",
        ])
        .expect("parse cli");

        let Commands::Inspect(args) = cli.command else {
            panic!("expected inspect command");
        };
        let query = inspect_query_from_args(args.query);
        assert_eq!(
            query,
            InspectQuery::Parameter {
                component_id: "thermal_control".to_string(),
                param_key: "control_driver".to_string(),
            }
        );
    }

    #[test]
    fn cli_compile_accepts_cluster_size_flag() {
        // configflux-0qo3 acceptance bar 4: `--cluster-size` must be
        // present on `configflux-compiler compile --help`. We verify
        // by parsing a compile invocation that uses the flag and
        // confirming the parsed value round-trips into CompileArgs.
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "compile",
            "--source",
            "defs.json",
            "--out",
            "out/cmp",
            "--cluster-size",
            "100",
        ])
        .expect("parse cli with cluster-size");
        let Commands::Compile(args) = cli.command else {
            panic!("expected compile command");
        };
        assert_eq!(args.cluster_size, 100);
    }

    #[test]
    fn cli_compile_accepts_progress_flag() {
        // configflux-9pjy.3 acceptance: `--progress {none|plain|json}`
        // must be present on `configflux-compiler compile --help` and
        // parse into the ProgressMode enum.
        for (arg, expected) in [
            ("none", ProgressMode::None),
            ("plain", ProgressMode::Plain),
            ("json", ProgressMode::Json),
        ] {
            let cli = Cli::try_parse_from([
                "configflux-compiler",
                "compile",
                "--source",
                "defs.json",
                "--progress",
                arg,
            ])
            .expect("parse cli with --progress");
            let Commands::Compile(args) = cli.command else {
                panic!("expected compile command");
            };
            assert_eq!(args.progress, expected, "--progress {arg} parsed wrong");
        }
    }

    #[test]
    fn cli_compile_defaults_progress_to_none() {
        // Omitting `--progress` defaults to `none` — the byte-identical,
        // no-output path (ADR-0039 §7 / ADR-0005 Amendment 2).
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "compile",
            "--source",
            "defs.json",
        ])
        .expect("parse cli without --progress");
        let Commands::Compile(args) = cli.command else {
            panic!("expected compile command");
        };
        assert_eq!(args.progress, ProgressMode::None);
    }

    #[test]
    fn cli_compile_accepts_max_threads_flag() {
        // configflux-9pjy.4 / ADR-0039 §6: `--max-threads` must parse into
        // CompileArgs so it can be plumbed to the budget (where it is honest
        // and narrow — it bites only the runtime solver, not the in-crate
        // compile).
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "compile",
            "--source",
            "defs.json",
            "--max-threads",
            "8",
        ])
        .expect("parse cli with --max-threads");
        let Commands::Compile(args) = cli.command else {
            panic!("expected compile command");
        };
        assert_eq!(args.max_threads, Some(8));
    }

    #[test]
    fn cli_compile_accepts_max_rss_mb_flag() {
        // configflux-9pjy.5 / ADR-0039: `--max-rss-mb` must be present on
        // `configflux-compiler compile --help` and parse into CompileArgs as
        // the soft RSS target (MiB).
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "compile",
            "--source",
            "defs.json",
            "--max-rss-mb",
            "2048",
        ])
        .expect("parse cli with --max-rss-mb");
        let Commands::Compile(args) = cli.command else {
            panic!("expected compile command");
        };
        assert_eq!(args.max_rss_mb, Some(2048));
    }

    #[test]
    fn cli_max_rss_and_threads_round_trip_into_resource_budget() {
        // configflux-9pjy.5 / ADR-0039 — hardening (4b): the parsed
        // `--max-rss-mb` / `--max-threads` flags must round-trip into the
        // exact `ResourceBudget` the compile request carries. Drives the
        // same `budget_from_args` mapping `run_compile` uses, so the wire
        // surface is pinned end-to-end from CLI args to the budget.

        // Both knobs set.
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "compile",
            "--source",
            "defs.json",
            "--max-rss-mb",
            "1024",
            "--max-threads",
            "4",
        ])
        .expect("parse cli with both budget knobs");
        let Commands::Compile(args) = cli.command else {
            panic!("expected compile command");
        };
        assert_eq!(
            budget_from_args(args.max_rss_mb, args.max_threads),
            Some(ResourceBudget {
                max_rss_mb: Some(1024),
                max_threads: Some(4),
            }),
            "both knobs must round-trip into the budget"
        );

        // Only --max-rss-mb.
        assert_eq!(
            budget_from_args(Some(512), None),
            Some(ResourceBudget {
                max_rss_mb: Some(512),
                max_threads: None,
            }),
            "an RSS-only budget must carry only max_rss_mb"
        );

        // Only --max-threads.
        assert_eq!(
            budget_from_args(None, Some(8)),
            Some(ResourceBudget {
                max_rss_mb: None,
                max_threads: Some(8),
            }),
            "a threads-only budget must carry only max_threads"
        );

        // Neither: NO budget, so the request omits the `budget` key entirely
        // (byte-identical unbudgeted wire surface).
        assert_eq!(
            budget_from_args(None, None),
            None,
            "an unbudgeted compile must build no ResourceBudget"
        );
    }

    #[test]
    fn in_crate_threads_warning_fires_only_when_set() {
        // configflux-9pjy.4 / ADR-0039 §6 (AC4): setting --max-threads on an
        // in-crate compile must produce a warning rather than silently
        // implying CPU scaling; omitting it must produce none. The warning
        // text must name the knob and point at the real RAM lever
        // (--max-rss-mb) so the operator is not misled.
        assert!(
            in_crate_threads_warning(None).is_none(),
            "no warning when --max-threads is unset"
        );
        let msg = in_crate_threads_warning(Some(8))
            .expect("a warning must fire when --max-threads is set");
        assert!(msg.contains("--max-threads"), "warning must name the knob: {msg}");
        assert!(
            msg.contains("--max-rss-mb"),
            "warning must point at the real RAM lever: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("inert") || msg.to_lowercase().contains("single-threaded"),
            "warning must explain the inertness: {msg}"
        );
    }

    #[test]
    fn cli_compile_defaults_cluster_size_to_usize_max() {
        // When `--cluster-size` is omitted, the default is `usize::MAX`
        // (per ADR-0012 §2 single-partition collapse rule).
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "compile",
            "--source",
            "defs.json",
        ])
        .expect("parse cli without cluster-size");
        let Commands::Compile(args) = cli.command else {
            panic!("expected compile command");
        };
        assert_eq!(args.cluster_size, usize::MAX);
    }

    #[test]
    fn cli_parses_inspect_scoped_stats_subcommand() {
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "inspect",
            "--source",
            "defs.json",
            "--source",
            "comps.json",
            "scoped-stats",
            "component:thermal_control",
        ])
        .expect("parse cli");

        let Commands::Inspect(args) = cli.command else {
            panic!("expected inspect command");
        };
        let query = inspect_query_from_args(args.query);
        assert_eq!(
            query,
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            }
        );
    }
}
