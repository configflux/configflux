// SPDX-License-Identifier: BUSL-1.1

pub(crate) use clap::{error::ErrorKind, Args, Parser, Subcommand};
pub(crate) use compiler::product_api::OperationStatus;
// The three write commands are NOT imported here: `commit_configuration`,
// `set_parameter` and `set_parameters_atomically` are reached through the
// `crate::write_enforcement::*_with_solver_validation` wrappers, which run the
// compiler operation and then enforce the model's declared constraints over the
// resulting assignment (configflux-jraj, ADR-0017 amendment D1). The wrappers
// live in `write_enforcement` rather than here because the C ABI staticlib must
// call them too and its crate root excludes this clap-based shell.
pub(crate) use compiler::runtime_api::{
    check_for_updates, export_pending_sync_bundle, get_auto_reset_policy,
    get_configuration_identity, get_dirty_metadata, get_parameter, get_scope_metadata,
    get_sync_status, list_dirty_parameters, list_parameters, pull_updates, push_audit_events,
    rollback_dirty, set_auto_reset_policy,
    subscribe_events, CheckForUpdatesRequest, CheckForUpdatesResult,
    CommitConfigurationRequest, CommitConfigurationResult, ExportPendingSyncBundleRequest,
    ExportPendingSyncBundleResult, GetAutoResetPolicyRequest, GetAutoResetPolicyResult,
    GetConfigurationIdentityRequest, GetConfigurationIdentityResult, GetDirtyMetadataRequest,
    GetDirtyMetadataResult, GetParameterRequest, GetParameterResult, GetScopeMetadataRequest,
    GetScopeMetadataResult, GetSyncStatusRequest, GetSyncStatusResult,
    ListDirtyParametersRequest, ListDirtyParametersResult, ListParametersRequest,
    ListParametersResult, PullUpdatesRequest, PullUpdatesResult, PushAuditEventsRequest,
    PushAuditEventsResult, RollbackDirtyRequest, RollbackDirtyResult, RuntimeExplainRejectionRequest,
    RuntimeExplainRejectionResult, RuntimeOpenRequest, RuntimeOpenResult, SetAutoResetPolicyRequest,
    SetAutoResetPolicyResult, SetParameterRequest, SetParameterResult,
    SetParametersAtomicallyRequest, SetParametersAtomicallyResult, SubscribeEventsRequest,
    SubscribeEventsResult,
};
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

pub(crate) const E_RUNTIME_CLI_ARGS_INVALID: &str = "E_RUNTIME_CLI_ARGS_INVALID";
pub(crate) const E_RUNTIME_CLI_REQUEST_IO: &str = "E_RUNTIME_CLI_REQUEST_IO";
pub(crate) const E_RUNTIME_CLI_REQUEST_TOO_LARGE: &str = "E_RUNTIME_CLI_REQUEST_TOO_LARGE";
pub(crate) const E_RUNTIME_CLI_REQUEST_INVALID: &str = "E_RUNTIME_CLI_REQUEST_INVALID";
pub(crate) const E_RUNTIME_CLI_RESPONSE_IO: &str = "E_RUNTIME_CLI_RESPONSE_IO";

#[derive(Parser)]
#[command(name = "configflux-runtime")]
#[command(version)]
#[command(about = "ConfigFlux runtime \u{2014} live parameter management, synchronization, and audit")]
#[command(after_help = "Exit codes: 0=success, 1=transport/input error, 2=command error")]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Open a compiled model snapshot for runtime parameter management
    #[command(name = "runtime-open")]
    RuntimeOpen(CommandIoArgs),
    /// Retrieve metadata for a configuration scope
    #[command(name = "get-scope-metadata")]
    GetScopeMetadata(CommandIoArgs),
    /// List all parameters visible within a scope
    #[command(name = "list-parameters")]
    ListParameters(CommandIoArgs),
    /// Read the current value and metadata of a single parameter
    #[command(name = "get-parameter")]
    GetParameter(CommandIoArgs),
    /// Write a new value to a single parameter
    #[command(name = "set-parameter")]
    SetParameter(CommandIoArgs),
    /// Write multiple parameters in a single atomic transaction
    #[command(name = "set-parameters-atomically", alias = "set-many-atomic")]
    SetParametersAtomically(CommandIoArgs),
    /// Explain why setting a parameter to a value would be rejected
    #[command(name = "explain-rejection")]
    ExplainRejection(CommandIoArgs),
    /// List parameters that have uncommitted (dirty) changes
    #[command(name = "list-dirty-parameters")]
    ListDirtyParameters(CommandIoArgs),
    /// Retrieve metadata about the current dirty-state changeset
    #[command(name = "get-dirty-metadata")]
    GetDirtyMetadata(CommandIoArgs),
    /// Discard all uncommitted parameter changes
    #[command(name = "rollback-dirty")]
    RollbackDirty(CommandIoArgs),
    /// Commit the current dirty changeset as a new configuration version
    #[command(name = "commit-configuration")]
    CommitConfiguration(CommandIoArgs),
    /// Retrieve the identity and hash of the current configuration version
    #[command(name = "get-configuration-identity")]
    GetConfigurationIdentity(CommandIoArgs),
    /// Configure the automatic reset policy for uncommitted changes
    #[command(name = "set-auto-reset-policy")]
    SetAutoResetPolicy(CommandIoArgs),
    /// Retrieve the current automatic reset policy
    #[command(name = "get-auto-reset-policy")]
    GetAutoResetPolicy(CommandIoArgs),
    /// Check whether upstream configuration updates are available
    #[command(name = "check-for-updates")]
    CheckForUpdates(CommandIoArgs),
    /// Pull and apply upstream configuration updates
    #[command(name = "pull-updates")]
    PullUpdates(CommandIoArgs),
    /// Retrieve the current synchronization status
    #[command(name = "get-sync-status")]
    GetSyncStatus(CommandIoArgs),
    /// Subscribe to runtime events (parameter changes, sync events)
    #[command(name = "subscribe-events")]
    SubscribeEvents(CommandIoArgs),
    /// Push deferred audit events to the upstream audit sink
    #[command(name = "push-audit-events")]
    PushAuditEvents(CommandIoArgs),
    /// Export uncommitted sync data as an offline transfer bundle
    #[command(name = "export-pending-sync-bundle")]
    ExportPendingSyncBundle(CommandIoArgs),
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
    RuntimeOpenResult,
    GetScopeMetadataResult,
    ListParametersResult,
    GetParameterResult,
    SetParameterResult,
    SetParametersAtomicallyResult,
    RuntimeExplainRejectionResult,
    ListDirtyParametersResult,
    GetDirtyMetadataResult,
    RollbackDirtyResult,
    CommitConfigurationResult,
    GetConfigurationIdentityResult,
    SetAutoResetPolicyResult,
    GetAutoResetPolicyResult,
    CheckForUpdatesResult,
    PullUpdatesResult,
    GetSyncStatusResult,
    SubscribeEventsResult,
    PushAuditEventsResult,
    ExportPendingSyncBundleResult,
);

pub(crate) fn main_entry() -> ExitCode {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    ExitCode::from(run(std::env::args_os(), &mut stdin, &mut stdout, &mut stderr))
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
        Commands::RuntimeOpen(io) => {
            execute_json_command::<RuntimeOpenRequest, RuntimeOpenResult, _, _, _, _>(
                "runtime-open",
                io,
                stdin,
                stdout,
                stderr,
                crate::solver_validation::runtime_open_with_solver_validation,
            )
        }
        Commands::GetScopeMetadata(io) => execute_json_command::<
            GetScopeMetadataRequest,
            GetScopeMetadataResult,
            _,
            _,
            _,
            _,
        >(
            "get-scope-metadata",
            io,
            stdin,
            stdout,
            stderr,
            get_scope_metadata,
        ),
        Commands::ListParameters(io) => execute_json_command::<
            ListParametersRequest,
            ListParametersResult,
            _,
            _,
            _,
            _,
        >(
            "list-parameters",
            io,
            stdin,
            stdout,
            stderr,
            list_parameters,
        ),
        Commands::GetParameter(io) => execute_json_command::<
            GetParameterRequest,
            GetParameterResult,
            _,
            _,
            _,
            _,
        >(
            "get-parameter",
            io,
            stdin,
            stdout,
            stderr,
            get_parameter,
        ),
        Commands::SetParameter(io) => execute_json_command::<
            SetParameterRequest,
            SetParameterResult,
            _,
            _,
            _,
            _,
        >(
            "set-parameter",
            io,
            stdin,
            stdout,
            stderr,
            crate::write_enforcement::set_parameter_with_solver_validation,
        ),
        Commands::SetParametersAtomically(io) => execute_json_command::<
            SetParametersAtomicallyRequest,
            SetParametersAtomicallyResult,
            _,
            _,
            _,
            _,
        >(
            "set-parameters-atomically",
            io,
            stdin,
            stdout,
            stderr,
            crate::write_enforcement::set_parameters_atomically_with_solver_validation,
        ),
        Commands::ExplainRejection(io) => execute_json_command::<
            RuntimeExplainRejectionRequest,
            RuntimeExplainRejectionResult,
            _,
            _,
            _,
            _,
        >(
            "explain-rejection",
            io,
            stdin,
            stdout,
            stderr,
            crate::explain_rejection::explain_rejection_via_solver,
        ),
        Commands::ListDirtyParameters(io) => execute_json_command::<
            ListDirtyParametersRequest,
            ListDirtyParametersResult,
            _,
            _,
            _,
            _,
        >(
            "list-dirty-parameters",
            io,
            stdin,
            stdout,
            stderr,
            list_dirty_parameters,
        ),
        Commands::GetDirtyMetadata(io) => execute_json_command::<
            GetDirtyMetadataRequest,
            GetDirtyMetadataResult,
            _,
            _,
            _,
            _,
        >(
            "get-dirty-metadata",
            io,
            stdin,
            stdout,
            stderr,
            get_dirty_metadata,
        ),
        Commands::RollbackDirty(io) => execute_json_command::<
            RollbackDirtyRequest,
            RollbackDirtyResult,
            _,
            _,
            _,
            _,
        >(
            "rollback-dirty",
            io,
            stdin,
            stdout,
            stderr,
            rollback_dirty,
        ),
        Commands::CommitConfiguration(io) => execute_json_command::<
            CommitConfigurationRequest,
            CommitConfigurationResult,
            _,
            _,
            _,
            _,
        >(
            "commit-configuration",
            io,
            stdin,
            stdout,
            stderr,
            crate::write_enforcement::commit_configuration_with_solver_validation,
        ),
        Commands::GetConfigurationIdentity(io) => execute_json_command::<
            GetConfigurationIdentityRequest,
            GetConfigurationIdentityResult,
            _,
            _,
            _,
            _,
        >(
            "get-configuration-identity",
            io,
            stdin,
            stdout,
            stderr,
            get_configuration_identity,
        ),
        Commands::SetAutoResetPolicy(io) => execute_json_command::<
            SetAutoResetPolicyRequest,
            SetAutoResetPolicyResult,
            _,
            _,
            _,
            _,
        >(
            "set-auto-reset-policy",
            io,
            stdin,
            stdout,
            stderr,
            set_auto_reset_policy,
        ),
        Commands::GetAutoResetPolicy(io) => execute_json_command::<
            GetAutoResetPolicyRequest,
            GetAutoResetPolicyResult,
            _,
            _,
            _,
            _,
        >(
            "get-auto-reset-policy",
            io,
            stdin,
            stdout,
            stderr,
            get_auto_reset_policy,
        ),
        Commands::CheckForUpdates(io) => execute_json_command::<
            CheckForUpdatesRequest,
            CheckForUpdatesResult,
            _,
            _,
            _,
            _,
        >(
            "check-for-updates",
            io,
            stdin,
            stdout,
            stderr,
            check_for_updates,
        ),
        Commands::PullUpdates(io) => execute_json_command::<
            PullUpdatesRequest,
            PullUpdatesResult,
            _,
            _,
            _,
            _,
        >(
            "pull-updates",
            io,
            stdin,
            stdout,
            stderr,
            pull_updates,
        ),
        Commands::GetSyncStatus(io) => execute_json_command::<
            GetSyncStatusRequest,
            GetSyncStatusResult,
            _,
            _,
            _,
            _,
        >(
            "get-sync-status",
            io,
            stdin,
            stdout,
            stderr,
            get_sync_status,
        ),
        Commands::SubscribeEvents(io) => execute_json_command::<
            SubscribeEventsRequest,
            SubscribeEventsResult,
            _,
            _,
            _,
            _,
        >(
            "subscribe-events",
            io,
            stdin,
            stdout,
            stderr,
            subscribe_events,
        ),
        Commands::PushAuditEvents(io) => execute_json_command::<
            PushAuditEventsRequest,
            PushAuditEventsResult,
            _,
            _,
            _,
            _,
        >(
            "push-audit-events",
            io,
            stdin,
            stdout,
            stderr,
            push_audit_events,
        ),
        Commands::ExportPendingSyncBundle(io) => execute_json_command::<
            ExportPendingSyncBundleRequest,
            ExportPendingSyncBundleResult,
            _,
            _,
            _,
            _,
        >(
            "export-pending-sync-bundle",
            io,
            stdin,
            stdout,
            stderr,
            export_pending_sync_bundle,
        ),
    }
}

pub(crate) fn render_cli_parse_error<W: Write, E: Write>(err: clap::Error, stdout: &mut W, stderr: &mut E) -> u8 {
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
                    E_RUNTIME_CLI_ARGS_INVALID,
                    "Invalid CLI arguments. Use --help for usage.",
                ),
            );
            EXIT_TRANSPORT_ERROR
        }
    }
}

pub(crate) fn execute_json_command<Req, Res, F, R, W, E>(
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

pub(crate) fn read_request_payload<R: Read>(io: &CommandIoArgs, stdin: &mut R) -> Result<Vec<u8>, TransportError> {
    if let Some(path) = &io.request_file {
        read_request_file(path)
    } else {
        read_request_stdin(stdin)
    }
}

pub(crate) fn read_request_file(path: &Path) -> Result<Vec<u8>, TransportError> {
    let source = path_display(path);
    let metadata = fs::metadata(path).map_err(|err| {
        TransportError::new(
            E_RUNTIME_CLI_REQUEST_IO,
            format!(
                "Unable to read request file '{source}' ({})",
                io_error_class(&err)
            ),
        )
    })?;

    if metadata.len() > REQUEST_SIZE_LIMIT_BYTES as u64 {
        return Err(TransportError::new(
            E_RUNTIME_CLI_REQUEST_TOO_LARGE,
            format!(
                "Request payload from '{source}' exceeds {} bytes",
                REQUEST_SIZE_LIMIT_BYTES
            ),
        ));
    }

    let payload = fs::read(path).map_err(|err| {
        TransportError::new(
            E_RUNTIME_CLI_REQUEST_IO,
            format!(
                "Unable to read request file '{source}' ({})",
                io_error_class(&err)
            ),
        )
    })?;

    if payload.len() > REQUEST_SIZE_LIMIT_BYTES {
        return Err(TransportError::new(
            E_RUNTIME_CLI_REQUEST_TOO_LARGE,
            format!(
                "Request payload from '{source}' exceeds {} bytes",
                REQUEST_SIZE_LIMIT_BYTES
            ),
        ));
    }

    Ok(payload)
}

pub(crate) fn read_request_stdin<R: Read>(stdin: &mut R) -> Result<Vec<u8>, TransportError> {
    let mut payload = Vec::new();
    let mut limited = stdin.take((REQUEST_SIZE_LIMIT_BYTES + 1) as u64);
    limited.read_to_end(&mut payload).map_err(|err| {
        TransportError::new(
            E_RUNTIME_CLI_REQUEST_IO,
            format!("Unable to read request payload from stdin ({})", io_error_class(&err)),
        )
    })?;

    if payload.len() > REQUEST_SIZE_LIMIT_BYTES {
        return Err(TransportError::new(
            E_RUNTIME_CLI_REQUEST_TOO_LARGE,
            format!(
                "Request payload from 'stdin' exceeds {} bytes",
                REQUEST_SIZE_LIMIT_BYTES
            ),
        ));
    }

    Ok(payload)
}

pub(crate) fn parse_request_payload<Req: DeserializeOwned>(
    command_name: &str,
    payload: &[u8],
) -> Result<Req, TransportError> {
    serde_json::from_slice(payload).map_err(|err| {
        TransportError::new(
            E_RUNTIME_CLI_REQUEST_INVALID,
            describe_request_parse_error(command_name, &err),
        )
    })
}

/// Build an `E_RUNTIME_CLI_REQUEST_INVALID` diagnostic that names the offending
/// request field when serde can identify one, without ever echoing a field
/// *value* (`docs/runtime-cli-contract.md` section 7, non-leaky stderr policy).
/// serde embeds the value inline for type/shape errors (e.g.
/// `invalid type: string "..."`), so those are reported by failure category and
/// location only; a missing required field carries no value, so its schema
/// field name is surfaced verbatim to tell the caller what to add.
pub(crate) fn describe_request_parse_error(command_name: &str, err: &serde_json::Error) -> String {
    let location = format!("line {}, column {}", err.line(), err.column());

    // serde's canonical "missing field `<name>`" text carries only the schema
    // field name (never a user-supplied value), so it is safe to surface.
    if let Some(field) = missing_field_name(&err.to_string()) {
        return format!(
            "Request for '{command_name}' command is missing required field '{field}' ({location})"
        );
    }

    // Any other error may embed the offending value in serde's own message, so
    // report the failure category and location only — never serde's text.
    let category = if err.is_syntax() {
        "malformed JSON syntax"
    } else if err.is_eof() {
        "truncated or empty JSON request"
    } else if err.is_io() {
        "request I/O error"
    } else {
        // Data error: a field has the wrong type or shape.
        "a request field has an invalid type or value"
    };
    format!("Request for '{command_name}' command is invalid: {category} ({location})")
}

/// Extract the field name from serde's canonical "missing field `<name>`"
/// message, returning `None` for any other error text. That wording is defined
/// by `serde::de::Error::missing_field` and is stable across serde versions; the
/// captured name is a compile-time struct field, never user input.
fn missing_field_name(message: &str) -> Option<&str> {
    let rest = message.strip_prefix("missing field `")?;
    let end = rest.find('`')?;
    Some(&rest[..end])
}

pub(crate) fn write_response_payload<Res: Serialize, W: Write>(
    io: &CommandIoArgs,
    stdout: &mut W,
    response: &Res,
) -> Result<(), TransportError> {
    let mut payload = serde_json::to_vec(response).map_err(|_| {
        TransportError::new(
            E_RUNTIME_CLI_RESPONSE_IO,
            "Failed to serialize response JSON",
        )
    })?;
    payload.push(b'\n');

    if let Some(path) = &io.response_file {
        let target = path_display(path);
        fs::write(path, payload).map_err(|err| {
            TransportError::new(
                E_RUNTIME_CLI_RESPONSE_IO,
                format!(
                    "Unable to write response file '{target}' ({})",
                    io_error_class(&err)
                ),
            )
        })
    } else {
        stdout.write_all(&payload).map_err(|err| {
            TransportError::new(
                E_RUNTIME_CLI_RESPONSE_IO,
                format!("Unable to write response payload to stdout ({})", io_error_class(&err)),
            )
        })
    }
}

pub(crate) fn write_transport_error<E: Write>(stderr: &mut E, err: &TransportError) {
    let _ = writeln!(stderr, "{}: {}", err.code, err.message);
}

pub(crate) fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn io_error_class(err: &std::io::Error) -> &'static str {
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

pub(crate) fn status_to_exit_code(status: OperationStatus) -> u8 {
    match status {
        OperationStatus::Ok => EXIT_OK,
        OperationStatus::Error => EXIT_COMMAND_ERROR,
    }
}
