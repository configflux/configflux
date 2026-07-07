// SPDX-License-Identifier: BUSL-1.1

#include "configflux/sdk/runtime_session.h"

namespace configflux::sdk {

const char* RuntimeAbiStatusName(RuntimeAbiStatus status) {
  switch (status) {
    case RuntimeAbiStatus::kOk:
      return "ok";
    case RuntimeAbiStatus::kNullPointer:
      return "null_pointer";
    case RuntimeAbiStatus::kInvalidUtf8:
      return "invalid_utf8";
    case RuntimeAbiStatus::kInvalidJson:
      return "invalid_json";
    case RuntimeAbiStatus::kVersionMismatch:
      return "version_mismatch";
    case RuntimeAbiStatus::kInvalidHandle:
      return "invalid_handle";
    case RuntimeAbiStatus::kUnsupportedOperation:
      return "unsupported_operation";
    case RuntimeAbiStatus::kInternalError:
      return "internal_error";
    case RuntimeAbiStatus::kCStringContainsNul:
      return "c_string_contains_nul";
  }
  return "unknown_abi_status";
}

RuntimeSdkStatus ToRuntimeSdkStatus(RuntimeAbiStatus status) {
  switch (status) {
    case RuntimeAbiStatus::kOk:
      return RuntimeSdkStatus::kOk;
    case RuntimeAbiStatus::kNullPointer:
      return RuntimeSdkStatus::kNullPointer;
    case RuntimeAbiStatus::kInvalidUtf8:
      return RuntimeSdkStatus::kInvalidUtf8;
    case RuntimeAbiStatus::kInvalidJson:
      return RuntimeSdkStatus::kInvalidJson;
    case RuntimeAbiStatus::kVersionMismatch:
      return RuntimeSdkStatus::kVersionMismatch;
    case RuntimeAbiStatus::kInvalidHandle:
      return RuntimeSdkStatus::kInvalidHandle;
    case RuntimeAbiStatus::kUnsupportedOperation:
      return RuntimeSdkStatus::kUnsupportedOperation;
    case RuntimeAbiStatus::kInternalError:
      return RuntimeSdkStatus::kInternalError;
    case RuntimeAbiStatus::kCStringContainsNul:
      return RuntimeSdkStatus::kCStringContainsNul;
  }
  return RuntimeSdkStatus::kInternalError;
}

const char* RuntimeSdkStatusName(RuntimeSdkStatus status) {
  switch (status) {
    case RuntimeSdkStatus::kOk:
      return "ok";
    case RuntimeSdkStatus::kNullPointer:
      return "null_pointer";
    case RuntimeSdkStatus::kInvalidUtf8:
      return "invalid_utf8";
    case RuntimeSdkStatus::kInvalidJson:
      return "invalid_json";
    case RuntimeSdkStatus::kVersionMismatch:
      return "version_mismatch";
    case RuntimeSdkStatus::kInvalidHandle:
      return "invalid_handle";
    case RuntimeSdkStatus::kUnsupportedOperation:
      return "unsupported_operation";
    case RuntimeSdkStatus::kInternalError:
      return "internal_error";
    case RuntimeSdkStatus::kCStringContainsNul:
      return "c_string_contains_nul";
    case RuntimeSdkStatus::kApiNotBound:
      return "api_not_bound";
    case RuntimeSdkStatus::kSessionClosed:
      return "session_closed";
    case RuntimeSdkStatus::kInvalidArgument:
      return "invalid_argument";
  }
  return "unknown_sdk_status";
}

}  // namespace configflux::sdk
