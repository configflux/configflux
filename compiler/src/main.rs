// SPDX-License-Identifier: BUSL-1.1

use anyhow::{Context, Result};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use compiler::loader_api::{open_model, OpenModelRequest};
use compiler::object::ObjectHeader;
use compiler::object_compile::{compile_object, object_summary_line, CompileObjectRequest};
use compiler::product_api::{
    compile_model, compile_model_with_progress, inspect_model, link_model,
    link_model_with_progress, verify_model,
    CompileModelRequest, CompileResult, InspectModelRequest, InspectQuery, LinkModelRequest,
    OperationStatus, SourceManifestEntry, VerifyModelRequest, PRODUCT_SCHEMA_VERSION,
};
use compiler::progress::{ProgressEvent, ProgressSink};
use compiler::resource_budget::ResourceBudget;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
    /// Compile ONE unit into a content-addressed object, against zero or more interface objects
    ///
    /// Example: configflux-compiler compile-object --source vision.json --interface catalogue.cfo --out vision.cfo
    CompileObject(CompileObjectArgs),
    /// Link objects into a deployable model package — the same package `compile` produces
    ///
    /// Example: configflux-compiler link --object catalogue.cfo --object vision.cfo --out out/cmp
    Link(LinkArgs),
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

/// `compile-object` — ADR-0058 §D3. One unit, all of its chunks, compiled
/// against the HEADERS of zero or more interface objects into an object
/// directory. Deliberately a separate verb rather than a `compile` flag: it
/// produces a different artifact (no constraint model, no package index) and
/// accepts a different input set.
#[derive(Args)]
struct CompileObjectArgs {
    /// Path to a source chunk of THIS unit; every one must declare the same
    /// `package` (repeat for multiple chunks)
    #[arg(long = "source", required = true, action = ArgAction::Append)]
    sources: Vec<PathBuf>,
    /// Path to an interface object directory to compile against. Only its
    /// `object.json` is read; its chunk files are never opened
    #[arg(long = "interface", action = ArgAction::Append)]
    interfaces: Vec<PathBuf>,
    /// Output directory for the object
    #[arg(long = "out", required = true)]
    out_dir: PathBuf,
    /// Stamp the emitted `provenance.json` with a wall-clock `stamped_at`
    /// (ADR-0044 D1). OFF by default so the object stays byte-stable.
    #[arg(long = "stamp-time", default_value_t = false)]
    stamp_time: bool,
    /// Output format: a one-line human summary, or the object header as JSON
    #[arg(long = "format", value_enum, default_value_t = ObjectFormat::Text)]
    format: ObjectFormat,
}

/// `link` — ADR-0058 §D4. The CCM and resource flags are `compile`'s, because
/// the constraint model is a link product and the flags that shape it belong to
/// the step that builds it.
///
/// The five lock flags are §D5's. A lockfile pins, per unit, the `object_hash`
/// an integration expects; the linker CHECKS those pins and never fetches
/// anything, because a monorepo checkout, a submodule, an artifact store or a
/// CI download is what brings objects to the linker.
#[derive(Args)]
struct LinkArgs {
    /// Path to an object directory to link (repeat for each object). Order does
    /// not reach the output: objects are linked by unit name
    #[arg(long = "object", required = true, action = ArgAction::Append)]
    objects: Vec<PathBuf>,
    /// Output directory for the compiled model package
    #[arg(long = "out", required = true)]
    out_dir: PathBuf,
    /// Target maximum number of distinct variables per BDD partition
    /// (ADR-0012 §2). Omitted collapses every model to a single partition
    #[arg(long = "cluster-size", default_value_t = usize::MAX)]
    cluster_size: usize,
    /// Soft target peak resident memory in MiB (ADR-0039). See `compile`
    #[arg(long = "max-rss-mb")]
    max_rss_mb: Option<u64>,
    /// Maximum thread count for the parallel compile/solve paths (ADR-0039)
    #[arg(long = "max-threads")]
    max_threads: Option<u32>,
    /// Link-time progress signal (ADR-0039 §7). See `compile`: `none`
    /// (default) emits nothing and keeps the link byte-for-byte identical,
    /// `plain` writes phase / percent / RSS / ETA lines to STDERR, `json`
    /// writes the JSON-lines stream to STDOUT. The constraint model is a link
    /// product, so this is the step whose progress there is something to watch
    #[arg(long = "progress", value_enum, default_value_t = ProgressMode::None)]
    progress: ProgressMode,
    /// Stamp the emitted `provenance.json` sidecars with a wall-clock
    /// `stamped_at` (ADR-0044 D1). OFF keeps the link byte-stable
    #[arg(long = "stamp-time", default_value_t = false)]
    stamp_time: bool,
    /// Path to a lockfile whose pins this link must satisfy (ADR-0058 §D5).
    /// Every linked object's unit must be pinned at the hash being linked, and
    /// every pinned unit must be linked. Checked first, before anything else
    #[arg(long = "lock")]
    lock: Option<PathBuf>,
    /// Accept a lockfile that pins units this link does not include, for a
    /// deliberate subset link. The other half of the check still holds: a
    /// linked object the lock does not pin is still refused
    #[arg(long = "lock-allow-extra", default_value_t = false)]
    lock_allow_extra: bool,
    /// Path to write the lockfile to after a successful link. The bytes are a
    /// function of the linked objects alone, so `--object` order cannot reach
    /// them. Refuses to overwrite a file that differs without `--force-lock`
    #[arg(long = "write-lock")]
    write_lock: Option<PathBuf>,
    /// `<unit>=<text>`, repeatable: the free-text `source` note to record for
    /// one unit in the written lock. Informational only -- nothing reads it to
    /// fetch anything. A unit that was not linked has nothing to annotate
    #[arg(long = "lock-source", action = ArgAction::Append)]
    lock_source: Vec<String>,
    /// Overwrite an existing lockfile that differs from the one this link
    /// would write
    #[arg(long = "force-lock", default_value_t = false)]
    force_lock: bool,
    /// Output format: a one-line human summary, or the full result as JSON
    #[arg(long = "format", value_enum, default_value_t = LinkFormat::Text)]
    format: LinkFormat,
}

/// `compile-object --format`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ObjectFormat {
    /// One summary line on stdout.
    Text,
    /// The written `object.json`, pretty-printed on stdout.
    Json,
}

/// `link --format`. Its own enum rather than `ObjectFormat`'s: the two verbs
/// take the same two spellings but print different things, and clap renders a
/// value enum's variant docs as the flag's possible-value help, so sharing one
/// enum makes one of the two help texts describe the other verb's output.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum LinkFormat {
    /// One summary line on stdout.
    Text,
    /// The full compile result, pretty-printed on stdout.
    Json,
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
        Commands::CompileObject(args) => run_compile_object(args),
        Commands::Link(args) => run_link(args),
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

/// Compile one unit into an object (ADR-0058 §D3).
///
/// Interface headers are read HERE rather than inside `compile_object`, so an
/// unreadable or malformed `--interface` is a CLI input error (exit 1) and only
/// a fault in the model itself is a compilation error (exit 2). That split is
/// the same one `--source` already has.
fn run_compile_object(args: CompileObjectArgs) -> Result<ExitCode> {
    let sources = load_sources(&args.sources)?;
    let mut interfaces = Vec::with_capacity(args.interfaces.len());
    for path in &args.interfaces {
        interfaces.push(ObjectHeader::read_from_dir(path).with_context(|| {
            format!("Failed to read interface object '{}'", path.display())
        })?);
    }

    let output_dir = args.out_dir.to_string_lossy().into_owned();
    match compile_object(CompileObjectRequest {
        sources,
        interfaces,
        output_dir: output_dir.clone(),
        stamp_time: args.stamp_time,
    }) {
        Ok(header) => {
            match args.format {
                ObjectFormat::Text => println!("{}", object_summary_line(&header, &output_dir)),
                ObjectFormat::Json => print_json(&header)?,
            }
            Ok(ExitCode::from(0))
        }
        Err(diagnostic) => {
            eprintln!("{}: {}", diagnostic.code, diagnostic.message);
            if let Some(hint) = &diagnostic.hint {
                eprintln!("hint: {hint}");
            }
            Ok(ExitCode::from(2))
        }
    }
}

/// Link objects into a package (ADR-0058 §D4).
///
/// The text form is the operator's line — the package identity, how many
/// objects went into it, and how many partitions the constraint model has —
/// and the JSON form is the same `compile_result` envelope `compile` prints,
/// so a caller that already parses one parses the other.
fn run_link(args: LinkArgs) -> Result<ExitCode> {
    warn_in_crate_threads_once(args.max_threads);
    let cluster_size = if args.cluster_size == usize::MAX {
        None
    } else {
        Some(args.cluster_size)
    };
    let out_dir = args.out_dir.to_string_lossy().into_owned();
    let request = LinkModelRequest {
        schema_version: PRODUCT_SCHEMA_VERSION,
        object_dirs: args
            .objects
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        output_dir: out_dir.clone(),
        cluster_size,
        budget: budget_from_args(args.max_rss_mb, args.max_threads),
        stamp_time: args.stamp_time,
        lock_path: args.lock.map(|path| path.to_string_lossy().into_owned()),
        lock_allow_extra: args.lock_allow_extra,
        write_lock_path: args.write_lock.map(|path| path.to_string_lossy().into_owned()),
        lock_sources: parse_lock_sources(&args.lock_source)?,
        force_lock: args.force_lock,
    };
    // ADR-0039 §7, exactly as `run_compile` selects it: `none` takes the plain
    // `link_model` path with NO sink wired, so an unwatched link is byte-for-
    // byte what it was before the flag existed.
    let result = match args.progress {
        ProgressMode::None => link_model(request),
        ProgressMode::Plain => {
            let sink = PlainStderrSink;
            link_model_with_progress(request, Some(&sink))
        }
        ProgressMode::Json => {
            let sink = JsonStdoutSink;
            link_model_with_progress(request, Some(&sink))
        }
    };
    match args.format {
        LinkFormat::Json => print_json(&result)?,
        LinkFormat::Text => {
            if result.status == OperationStatus::Ok {
                println!("{}", link_summary_line(&result, &out_dir));
            }
            for diagnostic in &result.verify_report.diagnostics.diagnostics {
                eprintln!("{}: {}", diagnostic.code, diagnostic.message);
                if let Some(hint) = &diagnostic.hint {
                    eprintln!("hint: {hint}");
                }
            }
        }
    }
    Ok(status_exit_code(result.status))
}

/// `--lock-source <unit>=<text>`, one entry per repetition (ADR-0058 §D5).
///
/// A CLI input error rather than a link diagnostic, because the fault is in
/// what was typed on the command line and nothing about the objects has been
/// read yet: `link` exits `1` here, as it does for every other malformed
/// argument. Split at the FIRST `=` so a note may contain one.
///
/// The unit is checked against the same snake_case rule the lockfile's own keys
/// are held to, so `--lock-source` and a hand-written lock cannot disagree
/// about what a unit name is. A later repetition of one unit replaces an
/// earlier one, which is the ordinary last-wins reading of a repeated flag.
fn parse_lock_sources(entries: &[String]) -> Result<BTreeMap<String, String>> {
    let mut sources = BTreeMap::new();
    for entry in entries {
        let (unit, note) = entry.split_once('=').with_context(|| {
            format!("--lock-source expects <unit>=<text>, got '{entry}'")
        })?;
        compiler::link_lock::check_unit_name(unit)
            .with_context(|| format!("--lock-source unit name is invalid: '{entry}'"))?;
        sources.insert(unit.to_string(), note.to_string());
    }
    Ok(sources)
}

/// The `link` summary line: identity, object count, partition count.
///
/// The partition count is read from the emitted `.ccm` manifest rather than
/// carried on the result envelope. It is a property of the artifact, and the
/// artifact is on disk by the time this line is printed; adding a field to the
/// shared `compile_result` for one verb's stdout would put a number in
/// `compile`'s envelope that `compile` never sets.
fn link_summary_line(result: &CompileResult, out_dir: &str) -> String {
    let partitions = ccm_partition_count(Path::new(out_dir)).unwrap_or(1);
    format!(
        "model_hash: {}  objects: {}  partitions: {}",
        result.model_hash,
        result.objects.len(),
        partitions
    )
}

/// How many partitions the emitted `.ccm` holds.
///
/// Read from `ccm/partition-manifest.json`, which is where the roster lives
/// (ADR-0005 Amendment 1 §13) — `ccm.manifest.json` names that file and carries
/// the hashes, but not the list. The v2 layout is always multi-part, so an
/// unpartitioned model still writes `partitions: ["partition-0000"]` and this
/// answers 1 by reading rather than by defaulting.
///
/// Returns `None` when the file cannot be read or does not carry the array, and
/// the caller prints 1. A missing count is not worth failing a link that
/// succeeded.
fn ccm_partition_count(cmp_dir: &Path) -> Option<usize> {
    let manifest = std::fs::read(cmp_dir.join("ccm").join("partition-manifest.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    value
        .get("partitions")
        .and_then(|partitions| partitions.as_array())
        .map(|partitions| partitions.len())
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
    fn partition_count_is_read_from_the_partition_manifest() {
        // Pins the FILENAME. The roster is in `partition-manifest.json`; an
        // earlier draft read `ccm.manifest.json`, which parses fine and carries
        // no `partitions` array, so the summary line silently answered 1 for
        // every model — a partitioned link would have reported the same number
        // as an unpartitioned one.
        let dir = std::env::temp_dir().join(format!(
            "cfx-link-partitions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let ccm = dir.join("ccm");
        std::fs::create_dir_all(&ccm).expect("mkdir");
        std::fs::write(
            ccm.join("partition-manifest.json"),
            br#"{"has_bridge":true,"partitions":["partition-0000","partition-0001"],"schema_version":1}"#,
        )
        .expect("write manifest");
        // The file the earlier draft read: present, valid, and silent about
        // partitions. It must not become the answer.
        std::fs::write(ccm.join("ccm.manifest.json"), br#"{"schema_version":1}"#)
            .expect("write ccm manifest");

        assert_eq!(ccm_partition_count(&dir), Some(2));
        assert_eq!(ccm_partition_count(&dir.join("absent")), None);
        std::fs::remove_dir_all(&dir).ok();
    }

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
    fn cli_link_accepts_progress_flag() {
        // ADR-0058 §D4 item 1: `link` takes the CCM and resource flags exactly
        // as `compile` does, and `--progress` is one of them — the constraint
        // model is a LINK product, so the step that builds it is the step whose
        // progress an operator watches. It was the only one of the five the
        // verb did not accept (configflux-p0jz.2 QA).
        for (arg, expected) in [
            ("none", ProgressMode::None),
            ("plain", ProgressMode::Plain),
            ("json", ProgressMode::Json),
        ] {
            let cli = Cli::try_parse_from([
                "configflux-compiler",
                "link",
                "--object",
                "unit.cfo",
                "--out",
                "out/cmp",
                "--progress",
                arg,
            ])
            .expect("parse link cli with --progress");
            let Commands::Link(args) = cli.command else {
                panic!("expected link command");
            };
            assert_eq!(args.progress, expected, "--progress {arg} parsed wrong");
        }
    }

    #[test]
    fn cli_link_defaults_progress_to_none() {
        // Omitting `--progress` keeps the byte-identical, no-output path, which
        // is what the §D8 oracle links through.
        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "link",
            "--object",
            "unit.cfo",
            "--out",
            "out/cmp",
        ])
        .expect("parse link cli without --progress");
        let Commands::Link(args) = cli.command else {
            panic!("expected link command");
        };
        assert_eq!(args.progress, ProgressMode::None);
    }

    #[test]
    fn cli_link_accepts_every_lock_flag() {
        // ADR-0058 §D5's five. A link with none of them behaves exactly as it
        // did before the lockfile existed, which is what the two `None`s and
        // the two `false`s below pin.
        let bare = Cli::try_parse_from([
            "configflux-compiler",
            "link",
            "--object",
            "unit.cfo",
            "--out",
            "out/cmp",
        ])
        .expect("parse link cli without any lock flag");
        let Commands::Link(args) = bare.command else {
            panic!("expected link command");
        };
        assert!(args.lock.is_none() && args.write_lock.is_none());
        assert!(!args.lock_allow_extra && !args.force_lock);
        assert!(args.lock_source.is_empty());

        let cli = Cli::try_parse_from([
            "configflux-compiler",
            "link",
            "--object",
            "unit.cfo",
            "--out",
            "out/cmp",
            "--lock",
            "configflux.lock",
            "--lock-allow-extra",
            "--write-lock",
            "next.lock",
            "--lock-source",
            "site_catalogue=an artifact store",
            "--force-lock",
        ])
        .expect("parse link cli with every lock flag");
        let Commands::Link(args) = cli.command else {
            panic!("expected link command");
        };
        assert_eq!(args.lock.as_deref(), Some(Path::new("configflux.lock")));
        assert_eq!(args.write_lock.as_deref(), Some(Path::new("next.lock")));
        assert!(args.lock_allow_extra && args.force_lock);
        assert_eq!(args.lock_source, vec!["site_catalogue=an artifact store"]);
    }

    #[test]
    fn lock_sources_split_at_the_first_equals_and_refuse_a_non_unit() {
        // A note may contain '=' — an artifact-store URL routinely does — so
        // only the first one separates. The unit is held to the lockfile's own
        // key rule, so a note can never be written under a key a lock could
        // not carry.
        let parsed = parse_lock_sources(&[
            "site_catalogue=store://objects?unit=site_catalogue".to_string(),
        ])
        .expect("parses");
        assert_eq!(
            parsed["site_catalogue"],
            "store://objects?unit=site_catalogue"
        );

        // Last repetition of one unit wins, which is the ordinary reading of a
        // repeated flag.
        let repeated = parse_lock_sources(&["a_unit=first".to_string(), "a_unit=second".to_string()])
            .expect("parses");
        assert_eq!(repeated["a_unit"], "second");

        assert!(
            parse_lock_sources(&["no_separator".to_string()]).is_err(),
            "an entry without '=' names no unit"
        );
        let error = parse_lock_sources(&["Site-Catalogue=x".to_string()]).expect_err("refused");
        assert!(
            format!("{error:#}").contains("Site-Catalogue"),
            "the message must name what it refused: {error:#}"
        );
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
