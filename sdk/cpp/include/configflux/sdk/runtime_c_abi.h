// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>

namespace configflux::sdk {

struct RuntimeAbiVersion {
  uint32_t major;
  uint32_t minor;
  uint32_t patch;
};

enum class RuntimeAbiStatus : uint32_t {
  kOk = 0,
  kNullPointer = 1,
  kInvalidUtf8 = 2,
  kInvalidJson = 3,
  kVersionMismatch = 4,
  kInvalidHandle = 5,
  kUnsupportedOperation = 6,
  kInternalError = 7,
  kCStringContainsNul = 8,
};

enum class RuntimeOperation : uint32_t {
  kGetScopeMetadata = 1,
  kListParameters = 2,
  kGetParameter = 3,
  kSetParameter = 4,
  kSetParametersAtomically = 5,
  kListDirtyParameters = 6,
  kGetDirtyMetadata = 7,
  kRollbackDirty = 8,
  kCommitConfiguration = 9,
  kGetConfigurationIdentity = 10,
  kSetAutoResetPolicy = 11,
  kGetAutoResetPolicy = 12,
  kCheckForUpdates = 13,
  kPullUpdates = 14,
  kGetSyncStatus = 15,
  kSubscribeEvents = 16,
  kPushAuditEvents = 17,
  kExportPendingSyncBundle = 18,
};

struct RuntimeSessionHandle;

using RuntimeAbiVersionFn = RuntimeAbiVersion (*)();
using RuntimeAbiHandshakeFn = RuntimeAbiStatus (*)(
    uint32_t expected_major, uint32_t expected_minor,
    RuntimeAbiVersion* out_version);
using RuntimeSessionOpenFn = RuntimeAbiStatus (*)(
    const char* runtime_open_request_json, RuntimeSessionHandle** out_handle,
    char** out_response_json);
using RuntimeSessionExecuteJsonFn = RuntimeAbiStatus (*)(
    RuntimeSessionHandle* handle, uint32_t operation, const char* request_json,
    char** out_response_json);
using RuntimeSessionSnapshotJsonFn = RuntimeAbiStatus (*)(
    const RuntimeSessionHandle* handle, char** out_snapshot_json);
using RuntimeSessionCloseFn = RuntimeAbiStatus (*)(RuntimeSessionHandle* handle);
using RuntimeStringFreeFn = void (*)(char* value);

struct RuntimeCAbiApi {
  RuntimeAbiVersionFn abi_version = nullptr;
  RuntimeAbiHandshakeFn abi_handshake = nullptr;
  RuntimeSessionOpenFn session_open = nullptr;
  RuntimeSessionExecuteJsonFn session_execute_json = nullptr;
  RuntimeSessionSnapshotJsonFn session_snapshot_json = nullptr;
  RuntimeSessionCloseFn session_close = nullptr;
  RuntimeStringFreeFn string_free = nullptr;

  bool IsBound() const {
    return abi_version != nullptr && abi_handshake != nullptr &&
           session_open != nullptr && session_execute_json != nullptr &&
           session_snapshot_json != nullptr && session_close != nullptr &&
           string_free != nullptr;
  }
};

inline RuntimeCAbiApi MakeRuntimeCAbiApi(
    RuntimeAbiVersionFn abi_version, RuntimeAbiHandshakeFn abi_handshake,
    RuntimeSessionOpenFn session_open,
    RuntimeSessionExecuteJsonFn session_execute_json,
    RuntimeSessionSnapshotJsonFn session_snapshot_json,
    RuntimeSessionCloseFn session_close, RuntimeStringFreeFn string_free) {
  RuntimeCAbiApi api;
  api.abi_version = abi_version;
  api.abi_handshake = abi_handshake;
  api.session_open = session_open;
  api.session_execute_json = session_execute_json;
  api.session_snapshot_json = session_snapshot_json;
  api.session_close = session_close;
  api.string_free = string_free;
  return api;
}

const char* RuntimeAbiStatusName(RuntimeAbiStatus status);

}  // namespace configflux::sdk
