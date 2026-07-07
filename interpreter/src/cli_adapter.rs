// SPDX-License-Identifier: BUSL-1.1

pub(crate) use clap::{error::ErrorKind, Args, Parser, Subcommand};
// Selection-chain commands route through the shared `session_compose` crate
// (configflux-g3f.2 / ADR-0017; extracted from the interpreter's `solver_session`
// module, configflux-kkhc); the legacy selection functions are called internally
// by the crate as its fallback. explain also routes through it (configflux-whyt /
// ADR-0031: solver-decided labeled unsat core → compiler `explain_rejection`).
// open / init / export stay on the compiler API (§4 "left in place").
pub(crate) use compiler::loader_api::{
    export_resolved, export_software_bom, initialize_selection_state, open_model,
    ApplySelectionRequest, ApplySelectionResult, ExplainRejectionRequest, ExplainRejectionResult,
    ExportResolvedRequest, ExportResolvedResult, ExportSoftwareBomRequest, ExportSoftwareBomResult,
    GetSelectionOptionsRequest, GetSelectionOptionsResult, InitializeSelectionStateRequest,
    InitializeSelectionStateResult, OpenModelRequest, OpenModelResult, ResolveFromSelectionRequest,
    ResolveResult,
};
pub(crate) use compiler::product_api::OperationStatus;
pub(crate) use serde::de::DeserializeOwned;
pub(crate) use serde::Serialize;
pub(crate) use std::ffi::OsString;
pub(crate) use std::fs;
pub(crate) use std::io::{Read, Write};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::ExitCode;

pub(crate) const REQUEST_SIZE_LIMIT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const EXIT_OK: u8 = 0;
pub(crate) const EXIT_TRANSPORT_ERROR: u8 = 1;
pub(crate) const EXIT_COMMAND_ERROR: u8 = 2;

pub(crate) const E_INTERPRETER_CLI_ARGS_INVALID: &str = "E_INTERPRETER_CLI_ARGS_INVALID";
pub(crate) const E_INTERPRETER_CLI_REQUEST_IO: &str = "E_INTERPRETER_CLI_REQUEST_IO";
pub(crate) const E_INTERPRETER_CLI_REQUEST_TOO_LARGE: &str = "E_INTERPRETER_CLI_REQUEST_TOO_LARGE";
pub(crate) const E_INTERPRETER_CLI_REQUEST_INVALID: &str = "E_INTERPRETER_CLI_REQUEST_INVALID";
pub(crate) const E_INTERPRETER_CLI_RESPONSE_IO: &str = "E_INTERPRETER_CLI_RESPONSE_IO";

#[derive(Parser)]
#[command(name = "configflux-interpreter")]
#[command(version)]
#[command(about = "ConfigFlux model interpreter \u{2014} selection, resolution, and export")]
#[command(after_help = "Exit codes: 0=success, 1=transport/input error, 2=command error")]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Open a compiled model package for interpretation
    Open(CommandIoArgs),
    /// Initialize the selection state for interactive configuration
    #[command(name = "init-selection-state")]
    InitSelectionState(CommandIoArgs),
    /// List available selection options for the current model state
    Options(CommandIoArgs),
    /// Apply a selection choice to the model state
    Select(CommandIoArgs),
    /// Explain why a parameter or component was rejected
    Explain(CommandIoArgs),
    /// Resolve the current selection into a concrete configuration
    Resolve(CommandIoArgs),
    /// Export the resolved configuration in a target-specific format
    #[command(name = "export-resolved")]
    ExportResolved(CommandIoArgs),
    /// Export a software bill of materials for the resolved model
    #[command(name = "export-software-bom")]
    ExportSoftwareBom(CommandIoArgs),
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CommandIoArgs {
    /// Read the JSON request from a file instead of stdin
    #[arg(long = "request-file")]
    request_file: Option<PathBuf>,
    /// Write the JSON response to a file instead of stdout
    #[arg(long = "response-file")]
    response_file: Option<PathBuf>,
}

pub(crate) struct TransportError {
    code: &'static str,
    message: String,
}

impl TransportError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) trait HasStatus {
    fn status(&self) -> OperationStatus;
}

macro_rules! impl_has_status {
    ($($result:ty),+ $(,)?) => {
        $(
            impl HasStatus for $result {
                fn status(&self) -> OperationStatus {
                    self.status
                }
            }
        )+
    };
}

impl_has_status!(
    OpenModelResult,
    InitializeSelectionStateResult,
    GetSelectionOptionsResult,
    ApplySelectionResult,
    ExplainRejectionResult,
    ResolveResult,
    ExportResolvedResult,
    ExportSoftwareBomResult,
);

pub(crate) fn main_entry() -> ExitCode {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    ExitCode::from(run(
        std::env::args_os(),
        &mut stdin,
        &mut stdout,
        &mut stderr,
    ))
}

pub(crate) fn run<I, S, R, W, E>(args: I, stdin: &mut R, stdout: &mut W, stderr: &mut E) -> u8
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
    R: Read,
    W: Write,
    E: Write,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => return render_cli_parse_error(err, stdout, stderr),
    };

    match cli.command {
        Commands::Open(io) => {
            execute_json_command::<OpenModelRequest, OpenModelResult, _, _, _, _>(
                "open", io, stdin, stdout, stderr, open_model,
            )
        }
        Commands::InitSelectionState(io) => execute_json_command::<
            InitializeSelectionStateRequest,
            InitializeSelectionStateResult,
            _,
            _,
            _,
            _,
        >(
            "init-selection-state",
            io,
            stdin,
            stdout,
            stderr,
            initialize_selection_state,
        ),
        // configflux-g3f.2 / ADR-0017 §4/§5: options/select route the
        // constraint decision through `solver::Session` via the shared
        // `session_compose` crate (legacy compiler fallback when no `.ccm`).
        Commands::Options(io) => {
            execute_json_command::<GetSelectionOptionsRequest, GetSelectionOptionsResult, _, _, _, _>(
                "options",
                io,
                stdin,
                stdout,
                stderr,
                session_compose::options,
            )
        }
        Commands::Select(io) => execute_json_command::<
            ApplySelectionRequest,
            ApplySelectionResult,
            _,
            _,
            _,
            _,
        >(
            "select",
            io,
            stdin,
            stdout,
            stderr,
            session_compose::apply,
        ),
        // configflux-whyt / ADR-0031: explain-rejection is a read-side,
        // solver-decided query. The arm is structurally unchanged; only the
        // impl it routes to changes — from the compiler-only `explain_rejection`
        // to the solver-wired `session_compose::explain`, which sources the
        // labeled unsat core from `solver::Session::explain_rejection` (ADR-0031
        // D3) and reports a rejection as the successful query it is (ADR-0031 D2).
        Commands::Explain(io) => execute_json_command::<
            ExplainRejectionRequest,
            ExplainRejectionResult,
            _,
            _,
            _,
            _,
        >(
            "explain",
            io,
            stdin,
            stdout,
            stderr,
            session_compose::explain,
        ),
        Commands::Resolve(io) => {
            // configflux-g3f.2 / ADR-0017 amendment: solver gates
            // satisfiability; compiler composes the rich ResolveResult.
            execute_json_command::<ResolveFromSelectionRequest, ResolveResult, _, _, _, _>(
                "resolve",
                io,
                stdin,
                stdout,
                stderr,
                session_compose::resolve,
            )
        }
        Commands::ExportResolved(io) => {
            execute_json_command::<ExportResolvedRequest, ExportResolvedResult, _, _, _, _>(
                "export-resolved",
                io,
                stdin,
                stdout,
                stderr,
                export_resolved,
            )
        }
        Commands::ExportSoftwareBom(io) => {
            execute_json_command::<ExportSoftwareBomRequest, ExportSoftwareBomResult, _, _, _, _>(
                "export-software-bom",
                io,
                stdin,
                stdout,
                stderr,
                export_software_bom,
            )
        }
    }
}

fn render_cli_parse_error<W: Write, E: Write>(
    err: clap::Error,
    stdout: &mut W,
    stderr: &mut E,
) -> u8 {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            if stdout.write_all(err.to_string().as_bytes()).is_err() {
                return EXIT_TRANSPORT_ERROR;
            }
            EXIT_OK
        }
        _ => {
            write_transport_error(
                stderr,
                &TransportError::new(
                    E_INTERPRETER_CLI_ARGS_INVALID,
                    "Invalid CLI arguments. Use --help for usage.",
                ),
            );
            EXIT_TRANSPORT_ERROR
        }
    }
}

fn execute_json_command<Req, Res, F, R, W, E>(
    command_name: &'static str,
    io: CommandIoArgs,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
    handler: F,
) -> u8
where
    Req: DeserializeOwned,
    Res: Serialize + HasStatus,
    F: FnOnce(Req) -> Res,
    R: Read,
    W: Write,
    E: Write,
{
    let request_payload = match read_request_payload(&io, stdin) {
        Ok(payload) => payload,
        Err(err) => {
            write_transport_error(stderr, &err);
            return EXIT_TRANSPORT_ERROR;
        }
    };

    let request = match parse_request_payload::<Req>(command_name, &request_payload) {
        Ok(request) => request,
        Err(err) => {
            write_transport_error(stderr, &err);
            return EXIT_TRANSPORT_ERROR;
        }
    };

    let response = handler(request);
    if let Err(err) = write_response_payload(&io, stdout, &response) {
        write_transport_error(stderr, &err);
        return EXIT_TRANSPORT_ERROR;
    }
    status_to_exit_code(response.status())
}

fn read_request_payload<R: Read>(
    io: &CommandIoArgs,
    stdin: &mut R,
) -> Result<Vec<u8>, TransportError> {
    if let Some(path) = &io.request_file {
        read_request_file(path)
    } else {
        read_request_stdin(stdin)
    }
}

fn read_request_file(path: &Path) -> Result<Vec<u8>, TransportError> {
    let source = path_display(path);
    let metadata = fs::metadata(path).map_err(|err| {
        TransportError::new(
            E_INTERPRETER_CLI_REQUEST_IO,
            format!(
                "Unable to read request file '{source}' ({})",
                io_error_class(&err)
            ),
        )
    })?;

    if metadata.len() > REQUEST_SIZE_LIMIT_BYTES as u64 {
        return Err(TransportError::new(
            E_INTERPRETER_CLI_REQUEST_TOO_LARGE,
            format!(
                "Request payload from '{source}' exceeds {} bytes",
                REQUEST_SIZE_LIMIT_BYTES
            ),
        ));
    }

    let payload = fs::read(path).map_err(|err| {
        TransportError::new(
            E_INTERPRETER_CLI_REQUEST_IO,
            format!(
                "Unable to read request file '{source}' ({})",
                io_error_class(&err)
            ),
        )
    })?;

    if payload.len() > REQUEST_SIZE_LIMIT_BYTES {
        return Err(TransportError::new(
            E_INTERPRETER_CLI_REQUEST_TOO_LARGE,
            format!(
                "Request payload from '{source}' exceeds {} bytes",
                REQUEST_SIZE_LIMIT_BYTES
            ),
        ));
    }

    Ok(payload)
}

fn read_request_stdin<R: Read>(stdin: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut payload = Vec::new();
    let mut limited = stdin.take((REQUEST_SIZE_LIMIT_BYTES + 1) as u64);
    limited.read_to_end(&mut payload).map_err(|err| {
        TransportError::new(
            E_INTERPRETER_CLI_REQUEST_IO,
            format!(
                "Unable to read request payload from stdin ({})",
                io_error_class(&err)
            ),
        )
    })?;

    if payload.len() > REQUEST_SIZE_LIMIT_BYTES {
        return Err(TransportError::new(
            E_INTERPRETER_CLI_REQUEST_TOO_LARGE,
            format!(
                "Request payload from 'stdin' exceeds {} bytes",
                REQUEST_SIZE_LIMIT_BYTES
            ),
        ));
    }

    Ok(payload)
}

fn parse_request_payload<Req: DeserializeOwned>(
    command_name: &str,
    payload: &[u8],
) -> Result<Req, TransportError> {
    serde_json::from_slice(payload).map_err(|_| {
        TransportError::new(
            E_INTERPRETER_CLI_REQUEST_INVALID,
            format!(
                "Malformed JSON request envelope for '{}' command",
                command_name
            ),
        )
    })
}

fn write_response_payload<Res: Serialize, W: Write>(
    io: &CommandIoArgs,
    stdout: &mut W,
    response: &Res,
) -> Result<(), TransportError> {
    let mut payload = serde_json::to_vec(response)
        .map_err(|_| TransportError::new(E_INTERPRETER_CLI_RESPONSE_IO, "Failed to serialize response JSON"))?;
    payload.push(b'\n');

    if let Some(path) = &io.response_file {
        let target = path_display(path);
        fs::write(path, payload).map_err(|err| {
            TransportError::new(
                E_INTERPRETER_CLI_RESPONSE_IO,
                format!(
                    "Unable to write response file '{target}' ({})",
                    io_error_class(&err)
                ),
            )
        })
    } else {
        stdout.write_all(&payload).map_err(|err| {
            TransportError::new(
                E_INTERPRETER_CLI_RESPONSE_IO,
                format!(
                    "Unable to write response payload to stdout ({})",
                    io_error_class(&err)
                ),
            )
        })
    }
}

fn write_transport_error<E: Write>(stderr: &mut E, err: &TransportError) {
    let _ = writeln!(stderr, "{}: {}", err.code, err.message);
}

pub(crate) fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn io_error_class(err: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::NotFound => "not_found",
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::AlreadyExists => "already_exists",
        ErrorKind::InvalidInput => "invalid_input",
        ErrorKind::InvalidData => "invalid_data",
        ErrorKind::WriteZero => "write_zero",
        ErrorKind::UnexpectedEof => "unexpected_eof",
        ErrorKind::WouldBlock => "would_block",
        ErrorKind::TimedOut => "timed_out",
        _ => "io_error",
    }
}

fn status_to_exit_code(status: OperationStatus) -> u8 {
    match status {
        OperationStatus::Ok => EXIT_OK,
        OperationStatus::Error => EXIT_COMMAND_ERROR,
    }
}
