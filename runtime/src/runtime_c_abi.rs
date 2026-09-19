// SPDX-License-Identifier: BUSL-1.1
//
// Runtime C ABI surface for ConfigFlux runtime-core operations.
//
// # Where this lives and why (configflux-u32v; ADR-0003 §2, ADR-0030 D2 Amendment 1)
//
// This module was moved out of the compiler crate. The exported
// `configflux_runtime_session_open` symbol must enforce the ADR-0030 D2
// open-time `.ccm` solver-model precondition, and that check requires the
// `solver` crate (`Session::load_ccm`). The compiler crate may NEVER import
// `solver` (ADR-0003 §2, mechanically enforced via the crate graph), so the
// C ABI cannot live there and still fail closed. The runtime crate already
// depends on both `compiler` and `solver`, so the open entrypoint delegates to
// `crate::solver_validation::runtime_open_with_solver_validation` — the SAME
// enforcement point the runtime CLI handler uses. CLI and ABI opens now apply
// the precondition uniformly; an SDK-driven open (C++/ROS2) can no longer
// bypass D2.

use crate::solver_validation::runtime_open_with_solver_validation;
use crate::write_enforcement::{
    commit_configuration_with_solver_validation, set_parameter_with_solver_validation,
    set_parameters_atomically_with_solver_validation,
};
use compiler::product_api::OperationStatus;
use compiler::runtime_api::{
    check_for_updates, export_pending_sync_bundle, get_auto_reset_policy,
    get_configuration_identity, get_dirty_metadata, get_parameter, get_scope_metadata,
    get_sync_status, list_dirty_parameters, list_parameters, pull_updates, push_audit_events,
    rollback_dirty, set_auto_reset_policy, subscribe_events,
    CheckForUpdatesRequest, CheckForUpdatesResult, CommitConfigurationRequest,
    CommitConfigurationResult, ExportPendingSyncBundleRequest, ExportPendingSyncBundleResult,
    GetAutoResetPolicyRequest, GetAutoResetPolicyResult, GetConfigurationIdentityRequest,
    GetConfigurationIdentityResult, GetDirtyMetadataRequest, GetDirtyMetadataResult,
    GetParameterRequest, GetParameterResult, GetScopeMetadataRequest, GetScopeMetadataResult,
    GetSyncStatusRequest, GetSyncStatusResult, ListDirtyParametersRequest,
    ListDirtyParametersResult, ListParametersRequest, ListParametersResult, PullUpdatesRequest,
    PullUpdatesResult, PushAuditEventsRequest, PushAuditEventsResult, RollbackDirtyRequest,
    RollbackDirtyResult, RuntimeOpenRequest, RuntimeSnapshot, SetAutoResetPolicyRequest,
    SetAutoResetPolicyResult, SetParameterRequest, SetParameterResult,
    SetParametersAtomicallyRequest, SetParametersAtomicallyResult, SubscribeEventsRequest,
    SubscribeEventsResult,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value as JsonValue;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

// ABI version 1.3 (ADR-0064 D5.5): the exported symbols and their C signatures
// are byte-identical to 1.2. Two behaviours are new, and the minor is the only
// channel that advertises them: (a) `configflux_runtime_session_snapshot_json`
// output may carry a `facet` key inside a resolved parameter, naming the facet
// that parameter is the declared runtime handle for, and (b) constraint
// enforcement on the write path follows those DECLARED bindings — a parameter
// that merely shares a facet's name is no longer treated as that facet, and is
// therefore no longer constraint-checked. `PRODUCT_SCHEMA_VERSION` deliberately
// stays 5: the field is additive and `#[serde(default)]`, so no existing payload
// changes meaning and no golden pre-image moves.
//
// ABI version 1.2 (configflux-tkwt, ADR-0060 D5): the exported symbols and
// their C signatures are byte-identical to 1.1, but the runtime now (a) honours
// a `closed_facet_domains` key on the open payload, (b) emits that key from
// `configflux_runtime_session_snapshot_json`, and (c) FAILS the open with
// `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN` when a supplied table names a facet or
// value the bound `.ccm` does not carry. The minor is the only channel that
// advertises those three; a CLI client gets the same behaviour with no version
// signal, because `PRODUCT_SCHEMA_VERSION` deliberately stays put (both
// additions are `#[serde(default)]`, so no existing payload's meaning changes
// and no golden pre-image moves).
//
// ABI version 1.1 (configflux-u32v): the exported symbols and their C
// signatures were byte-identical to 1.0, but `configflux_runtime_session_open`
// began enforcing the ADR-0030 D2 `.ccm` precondition and can fail closed with
// an `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` envelope where 1.0 always
// returned a snapshot for any loadable request.
//
// All three are additive behavior changes, so the minor moves while the major
// stays 1: the handshake rule (`expected_minor <= ABI minor`) keeps existing
// `expected_minor = 0`, `= 1` and `= 2` clients passing while advertising the
// stricter behavior honestly.
pub const CONFIGFLUX_RUNTIME_C_ABI_VERSION_MAJOR: u32 = 1;
pub const CONFIGFLUX_RUNTIME_C_ABI_VERSION_MINOR: u32 = 3;
pub const CONFIGFLUX_RUNTIME_C_ABI_VERSION_PATCH: u32 = 0;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigFluxRuntimeAbiVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFluxRuntimeAbiStatus {
    Ok = 0,
    NullPointer = 1,
    InvalidUtf8 = 2,
    InvalidJson = 3,
    VersionMismatch = 4,
    InvalidHandle = 5,
    UnsupportedOperation = 6,
    InternalError = 7,
    CStringContainsNul = 8,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFluxRuntimeOperation {
    GetScopeMetadata = 1,
    ListParameters = 2,
    GetParameter = 3,
    SetParameter = 4,
    SetParametersAtomically = 5,
    ListDirtyParameters = 6,
    GetDirtyMetadata = 7,
    RollbackDirty = 8,
    CommitConfiguration = 9,
    GetConfigurationIdentity = 10,
    SetAutoResetPolicy = 11,
    GetAutoResetPolicy = 12,
    CheckForUpdates = 13,
    PullUpdates = 14,
    GetSyncStatus = 15,
    SubscribeEvents = 16,
    PushAuditEvents = 17,
    ExportPendingSyncBundle = 18,
}

impl TryFrom<u32> for ConfigFluxRuntimeOperation {
    type Error = ConfigFluxRuntimeAbiStatus;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::GetScopeMetadata),
            2 => Ok(Self::ListParameters),
            3 => Ok(Self::GetParameter),
            4 => Ok(Self::SetParameter),
            5 => Ok(Self::SetParametersAtomically),
            6 => Ok(Self::ListDirtyParameters),
            7 => Ok(Self::GetDirtyMetadata),
            8 => Ok(Self::RollbackDirty),
            9 => Ok(Self::CommitConfiguration),
            10 => Ok(Self::GetConfigurationIdentity),
            11 => Ok(Self::SetAutoResetPolicy),
            12 => Ok(Self::GetAutoResetPolicy),
            13 => Ok(Self::CheckForUpdates),
            14 => Ok(Self::PullUpdates),
            15 => Ok(Self::GetSyncStatus),
            16 => Ok(Self::SubscribeEvents),
            17 => Ok(Self::PushAuditEvents),
            18 => Ok(Self::ExportPendingSyncBundle),
            _ => Err(ConfigFluxRuntimeAbiStatus::UnsupportedOperation),
        }
    }
}

#[repr(C)]
pub struct ConfigFluxRuntimeSessionHandle {
    _private: [u8; 0],
}

struct RuntimeSessionState {
    snapshot: RuntimeSnapshot,
}

/// A Rust panic must never unwind into the C caller: on rustc >= 1.81 that
/// unwind is a defined process abort, which would kill the embedding host
/// (C++ SDK, ROS 2 node) instead of returning a status. Every exported entry
/// point below routes its body through this guard so an unexpected panic
/// surfaces as `InternalError` and the host keeps running (configflux-xowl.1).
fn abi_panic_guard(
    body: impl FnOnce() -> ConfigFluxRuntimeAbiStatus,
) -> ConfigFluxRuntimeAbiStatus {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(_) => ConfigFluxRuntimeAbiStatus::InternalError,
    }
}

#[no_mangle]
pub extern "C" fn configflux_runtime_abi_version() -> ConfigFluxRuntimeAbiVersion {
    ConfigFluxRuntimeAbiVersion {
        major: CONFIGFLUX_RUNTIME_C_ABI_VERSION_MAJOR,
        minor: CONFIGFLUX_RUNTIME_C_ABI_VERSION_MINOR,
        patch: CONFIGFLUX_RUNTIME_C_ABI_VERSION_PATCH,
    }
}

#[no_mangle]
pub unsafe extern "C" fn configflux_runtime_abi_handshake(
    expected_major: u32,
    expected_minor: u32,
    out_version: *mut ConfigFluxRuntimeAbiVersion,
) -> ConfigFluxRuntimeAbiStatus {
    abi_panic_guard(|| {
        if out_version.is_null() {
            return ConfigFluxRuntimeAbiStatus::NullPointer;
        }
        *out_version = configflux_runtime_abi_version();
        if expected_major != CONFIGFLUX_RUNTIME_C_ABI_VERSION_MAJOR
            || expected_minor > CONFIGFLUX_RUNTIME_C_ABI_VERSION_MINOR
        {
            return ConfigFluxRuntimeAbiStatus::VersionMismatch;
        }
        ConfigFluxRuntimeAbiStatus::Ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn configflux_runtime_session_open(
    runtime_open_request_json: *const c_char,
    out_handle: *mut *mut ConfigFluxRuntimeSessionHandle,
    out_response_json: *mut *mut c_char,
) -> ConfigFluxRuntimeAbiStatus {
    abi_panic_guard(|| {
        if out_handle.is_null() || out_response_json.is_null() {
            return ConfigFluxRuntimeAbiStatus::NullPointer;
        }
        *out_handle = ptr::null_mut();
        *out_response_json = ptr::null_mut();

        let request_json = match c_string_to_rust(runtime_open_request_json) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let request: RuntimeOpenRequest = match serde_json::from_str(&request_json) {
            Ok(value) => value,
            Err(_) => return ConfigFluxRuntimeAbiStatus::InvalidJson,
        };

        // ADR-0030 D2 (configflux-u32v): go through the runtime-crate validation
        // wrapper, NOT bare `runtime_open`. This is the same fail-closed `.ccm`
        // precondition the runtime CLI handler enforces, so an SDK-driven open
        // cannot accept a snapshot without a loadable, symbol-bearing `.ccm`. A
        // failed precondition is surfaced as a domain error in the response JSON
        // (`status=error` + `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`) and yields no
        // session handle, mirroring every other runtime-domain rejection.
        let response = runtime_open_with_solver_validation(request);
        let response_json = match serde_json::to_string(&response) {
            Ok(value) => value,
            Err(_) => return ConfigFluxRuntimeAbiStatus::InternalError,
        };
        let status = write_rust_string_to_c(out_response_json, response_json);
        if status != ConfigFluxRuntimeAbiStatus::Ok {
            return status;
        }

        if response.status == OperationStatus::Ok {
            if let Some(snapshot) = response.runtime_snapshot {
                let state = Box::new(RuntimeSessionState { snapshot });
                *out_handle = Box::into_raw(state) as *mut ConfigFluxRuntimeSessionHandle;
            }
        }

        ConfigFluxRuntimeAbiStatus::Ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn configflux_runtime_session_execute_json(
    handle: *mut ConfigFluxRuntimeSessionHandle,
    operation: u32,
    request_json: *const c_char,
    out_response_json: *mut *mut c_char,
) -> ConfigFluxRuntimeAbiStatus {
    abi_panic_guard(|| {
        if out_response_json.is_null() {
            return ConfigFluxRuntimeAbiStatus::NullPointer;
        }
        *out_response_json = ptr::null_mut();

        let session = match session_from_handle(handle) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let op = match ConfigFluxRuntimeOperation::try_from(operation) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let request_payload = match c_string_to_rust(request_json) {
            Ok(value) => value,
            Err(status) => return status,
        };

        macro_rules! dispatch {
            ($request_ty:ty, $response_ty:ty, $handler:path) => {
                execute_with_snapshot::<$request_ty, $response_ty, _>(
                    session,
                    &request_payload,
                    $handler,
                )
            };
        }

        let response_json = match op {
            ConfigFluxRuntimeOperation::GetScopeMetadata => {
                dispatch!(GetScopeMetadataRequest, GetScopeMetadataResult, get_scope_metadata)
            }
            ConfigFluxRuntimeOperation::ListParameters => {
                dispatch!(ListParametersRequest, ListParametersResult, list_parameters)
            }
            ConfigFluxRuntimeOperation::GetParameter => {
                dispatch!(GetParameterRequest, GetParameterResult, get_parameter)
            }
            // configflux-jraj (ADR-0017 amendment D7): the three write arms go
            // through the runtime-crate validation wrappers, NOT the raw
            // `compiler::runtime_api` entry points. Until this change they
            // dispatched straight to the compiler, so every C++/ROS2 SDK write
            // ran with no solver check at all — not even the `choices`-based one
            // the CLI already had. That is the same split configflux-u32v closed
            // for open, and it is closed here for the same reason: enforcement
            // that a caller can sidestep by picking a transport is not
            // enforcement.
            ConfigFluxRuntimeOperation::SetParameter => dispatch!(
                SetParameterRequest,
                SetParameterResult,
                set_parameter_with_solver_validation
            ),
            ConfigFluxRuntimeOperation::SetParametersAtomically => dispatch!(
                SetParametersAtomicallyRequest,
                SetParametersAtomicallyResult,
                set_parameters_atomically_with_solver_validation
            ),
            ConfigFluxRuntimeOperation::ListDirtyParameters => {
                dispatch!(ListDirtyParametersRequest, ListDirtyParametersResult, list_dirty_parameters)
            }
            ConfigFluxRuntimeOperation::GetDirtyMetadata => {
                dispatch!(GetDirtyMetadataRequest, GetDirtyMetadataResult, get_dirty_metadata)
            }
            ConfigFluxRuntimeOperation::RollbackDirty => {
                dispatch!(RollbackDirtyRequest, RollbackDirtyResult, rollback_dirty)
            }
            ConfigFluxRuntimeOperation::CommitConfiguration => dispatch!(
                CommitConfigurationRequest,
                CommitConfigurationResult,
                commit_configuration_with_solver_validation
            ),
            ConfigFluxRuntimeOperation::GetConfigurationIdentity => dispatch!(
                GetConfigurationIdentityRequest,
                GetConfigurationIdentityResult,
                get_configuration_identity
            ),
            ConfigFluxRuntimeOperation::SetAutoResetPolicy => {
                dispatch!(SetAutoResetPolicyRequest, SetAutoResetPolicyResult, set_auto_reset_policy)
            }
            ConfigFluxRuntimeOperation::GetAutoResetPolicy => {
                dispatch!(GetAutoResetPolicyRequest, GetAutoResetPolicyResult, get_auto_reset_policy)
            }
            ConfigFluxRuntimeOperation::CheckForUpdates => {
                dispatch!(CheckForUpdatesRequest, CheckForUpdatesResult, check_for_updates)
            }
            ConfigFluxRuntimeOperation::PullUpdates => {
                dispatch!(PullUpdatesRequest, PullUpdatesResult, pull_updates)
            }
            ConfigFluxRuntimeOperation::GetSyncStatus => {
                dispatch!(GetSyncStatusRequest, GetSyncStatusResult, get_sync_status)
            }
            ConfigFluxRuntimeOperation::SubscribeEvents => {
                dispatch!(SubscribeEventsRequest, SubscribeEventsResult, subscribe_events)
            }
            ConfigFluxRuntimeOperation::PushAuditEvents => {
                dispatch!(PushAuditEventsRequest, PushAuditEventsResult, push_audit_events)
            }
            ConfigFluxRuntimeOperation::ExportPendingSyncBundle => dispatch!(
                ExportPendingSyncBundleRequest,
                ExportPendingSyncBundleResult,
                export_pending_sync_bundle
            ),
        };

        match response_json {
            Ok(payload) => write_rust_string_to_c(out_response_json, payload),
            Err(status) => status,
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn configflux_runtime_session_snapshot_json(
    handle: *const ConfigFluxRuntimeSessionHandle,
    out_snapshot_json: *mut *mut c_char,
) -> ConfigFluxRuntimeAbiStatus {
    abi_panic_guard(|| {
        if out_snapshot_json.is_null() {
            return ConfigFluxRuntimeAbiStatus::NullPointer;
        }
        *out_snapshot_json = ptr::null_mut();

        if handle.is_null() {
            return ConfigFluxRuntimeAbiStatus::InvalidHandle;
        }
        let session = &*(handle as *const RuntimeSessionState);
        let snapshot_json = match serde_json::to_string(&session.snapshot) {
            Ok(value) => value,
            Err(_) => return ConfigFluxRuntimeAbiStatus::InternalError,
        };
        write_rust_string_to_c(out_snapshot_json, snapshot_json)
    })
}

#[no_mangle]
pub unsafe extern "C" fn configflux_runtime_session_close(
    handle: *mut ConfigFluxRuntimeSessionHandle,
) -> ConfigFluxRuntimeAbiStatus {
    abi_panic_guard(|| {
        if handle.is_null() {
            return ConfigFluxRuntimeAbiStatus::InvalidHandle;
        }
        drop(Box::from_raw(handle as *mut RuntimeSessionState));
        ConfigFluxRuntimeAbiStatus::Ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn configflux_runtime_string_free(value: *mut c_char) {
    // Returns no status, so a panic here can only be swallowed — the guard
    // still must exist to stop the unwind-into-C abort (configflux-xowl.1).
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if value.is_null() {
            return;
        }
        let _ = CString::from_raw(value);
    }));
}

unsafe fn session_from_handle<'a>(
    handle: *mut ConfigFluxRuntimeSessionHandle,
) -> Result<&'a mut RuntimeSessionState, ConfigFluxRuntimeAbiStatus> {
    if handle.is_null() {
        return Err(ConfigFluxRuntimeAbiStatus::InvalidHandle);
    }
    Ok(&mut *(handle as *mut RuntimeSessionState))
}

fn execute_with_snapshot<Req, Res, F>(
    session: &mut RuntimeSessionState,
    request_json: &str,
    handler: F,
) -> Result<String, ConfigFluxRuntimeAbiStatus>
where
    Req: DeserializeOwned,
    Res: Serialize,
    F: FnOnce(Req) -> Res,
{
    let mut request_value: JsonValue =
        serde_json::from_str(request_json).map_err(|_| ConfigFluxRuntimeAbiStatus::InvalidJson)?;
    let request_obj = request_value
        .as_object_mut()
        .ok_or(ConfigFluxRuntimeAbiStatus::InvalidJson)?;
    let snapshot_value =
        serde_json::to_value(&session.snapshot).map_err(|_| ConfigFluxRuntimeAbiStatus::InternalError)?;
    request_obj.insert("runtime_snapshot".to_string(), snapshot_value);

    let request: Req =
        serde_json::from_value(request_value).map_err(|_| ConfigFluxRuntimeAbiStatus::InvalidJson)?;
    let response = handler(request);

    let response_value =
        serde_json::to_value(&response).map_err(|_| ConfigFluxRuntimeAbiStatus::InternalError)?;
    if let Some(snapshot_json) = response_value
        .get("runtime_snapshot")
        .filter(|snapshot| !snapshot.is_null())
    {
        let snapshot: RuntimeSnapshot = serde_json::from_value(snapshot_json.clone())
            .map_err(|_| ConfigFluxRuntimeAbiStatus::InternalError)?;
        session.snapshot = snapshot;
    }

    serde_json::to_string(&response).map_err(|_| ConfigFluxRuntimeAbiStatus::InternalError)
}

unsafe fn c_string_to_rust(ptr: *const c_char) -> Result<String, ConfigFluxRuntimeAbiStatus> {
    if ptr.is_null() {
        return Err(ConfigFluxRuntimeAbiStatus::NullPointer);
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(|value| value.to_string())
        .map_err(|_| ConfigFluxRuntimeAbiStatus::InvalidUtf8)
}

unsafe fn write_rust_string_to_c(
    out: *mut *mut c_char,
    value: String,
) -> ConfigFluxRuntimeAbiStatus {
    if out.is_null() {
        return ConfigFluxRuntimeAbiStatus::NullPointer;
    }
    let c_string = match CString::new(value) {
        Ok(value) => value,
        Err(_) => return ConfigFluxRuntimeAbiStatus::CStringContainsNul,
    };
    *out = c_string.into_raw();
    ConfigFluxRuntimeAbiStatus::Ok
}

#[cfg(test)]
mod abi_panic_guard_tests {
    use super::*;

    #[test]
    fn passes_through_the_body_status() {
        let status = abi_panic_guard(|| ConfigFluxRuntimeAbiStatus::VersionMismatch);
        assert_eq!(status, ConfigFluxRuntimeAbiStatus::VersionMismatch);
    }

    #[test]
    fn maps_a_panicking_body_to_internal_error() {
        // The deliberate panic prints one message to the captured test log;
        // touching the process-global panic hook to silence it would race
        // with parallel tests, which is worse than the noise.
        let status = abi_panic_guard(|| panic!("deliberate panic for the guard test"));
        assert_eq!(status, ConfigFluxRuntimeAbiStatus::InternalError);
    }
}
