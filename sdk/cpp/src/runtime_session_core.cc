// SPDX-License-Identifier: BUSL-1.1

#include "configflux/sdk/runtime_session.h"

#include <utility>

namespace configflux::sdk {

namespace {

std::string StatusMessage(const char* prefix, RuntimeSdkStatus status) {
  std::string message(prefix);
  message.append(": ");
  message.append(RuntimeSdkStatusName(status));
  return message;
}

}  // namespace

RuntimeSession::RuntimeSession(RuntimeCAbiApi api) : api_(api) {}

RuntimeSession::~RuntimeSession() { (void)Close(); }

RuntimeSession::RuntimeSession(RuntimeSession&& other) noexcept : api_(other.api_) {
#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
  std::scoped_lock<std::mutex, std::mutex> lock(other.mutex_, other.callbacks_mutex_);
  handle_ = other.handle_;
  other.handle_ = nullptr;
  next_subscription_id_ = other.next_subscription_id_;
  last_observed_dropped_events_ = other.last_observed_dropped_events_;
  event_subscriptions_ = std::move(other.event_subscriptions_);
  other.next_subscription_id_ = 1;
  other.last_observed_dropped_events_ = 0;
#else
  std::lock_guard<std::mutex> lock(other.mutex_);
  handle_ = other.handle_;
  other.handle_ = nullptr;
#endif
}

RuntimeSession& RuntimeSession::operator=(RuntimeSession&& other) noexcept {
  if (this == &other) {
    return *this;
  }

#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
  std::scoped_lock<std::mutex, std::mutex, std::mutex, std::mutex> lock(
      mutex_, callbacks_mutex_, other.mutex_, other.callbacks_mutex_);
#else
  std::scoped_lock<std::mutex, std::mutex> lock(mutex_, other.mutex_);
#endif
  if (handle_ != nullptr && api_.session_close != nullptr) {
    (void)api_.session_close(handle_);
  }
  api_ = other.api_;
  handle_ = other.handle_;
  other.handle_ = nullptr;

#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
  next_subscription_id_ = other.next_subscription_id_;
  last_observed_dropped_events_ = other.last_observed_dropped_events_;
  event_subscriptions_ = std::move(other.event_subscriptions_);
  other.next_subscription_id_ = 1;
  other.last_observed_dropped_events_ = 0;
#endif
  return *this;
}

RuntimeOpenResult RuntimeSession::Open(std::string_view runtime_open_request_json,
                                       uint32_t expected_abi_major,
                                       uint32_t expected_abi_minor) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!api_.IsBound()) {
    return BuildLocalOpenError(RuntimeSdkStatus::kApiNotBound,
                               "runtime C ABI table is not fully bound");
  }
  if (handle_ != nullptr) {
    return BuildLocalOpenError(RuntimeSdkStatus::kInvalidArgument,
                               "session is already open");
  }
  if (HasEmbeddedNul(runtime_open_request_json)) {
    return BuildLocalOpenError(
        RuntimeSdkStatus::kInvalidArgument,
        "runtime_open_request_json must not contain embedded NUL bytes");
  }

  RuntimeOpenResult result;
  RuntimeAbiVersion version = api_.abi_version();
  result.abi_version = version;
  RuntimeSdkStatus handshake_status = ToRuntimeSdkStatus(
      api_.abi_handshake(expected_abi_major, expected_abi_minor, &result.abi_version));
  if (handshake_status != RuntimeSdkStatus::kOk) {
    result.status = handshake_status;
    result.message = StatusMessage("abi handshake failed", handshake_status);
    return result;
  }

  std::string request(runtime_open_request_json);
  RuntimeSessionHandle* opened_handle = nullptr;
  char* response_json_ptr = nullptr;
  RuntimeSdkStatus open_status = ToRuntimeSdkStatus(
      api_.session_open(request.c_str(), &opened_handle, &response_json_ptr));
  if (response_json_ptr != nullptr) {
    result.open_response_json = response_json_ptr;
    api_.string_free(response_json_ptr);
  }

  result.status = open_status;
  if (open_status != RuntimeSdkStatus::kOk) {
    result.message = StatusMessage("session_open failed", open_status);
    if (opened_handle != nullptr) {
      (void)api_.session_close(opened_handle);
    }
    return result;
  }

  handle_ = opened_handle;
  return result;
}

RuntimeCallResult RuntimeSession::Execute(RuntimeOperation operation,
                                          std::string_view request_json) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!api_.IsBound()) {
    return BuildLocalCallError(RuntimeSdkStatus::kApiNotBound,
                               "runtime C ABI table is not fully bound");
  }
  if (handle_ == nullptr) {
    return BuildLocalCallError(RuntimeSdkStatus::kSessionClosed,
                               "session must be opened before Execute");
  }
  if (HasEmbeddedNul(request_json)) {
    return BuildLocalCallError(RuntimeSdkStatus::kInvalidArgument,
                               "request_json must not contain embedded NUL bytes");
  }

  std::string request(request_json);
  RuntimeCallResult result;
  char* response_json_ptr = nullptr;
  RuntimeSdkStatus execute_status = ToRuntimeSdkStatus(api_.session_execute_json(
      handle_, static_cast<uint32_t>(operation), request.c_str(), &response_json_ptr));
  if (response_json_ptr != nullptr) {
    result.response_json = response_json_ptr;
    api_.string_free(response_json_ptr);
  }
  result.status = execute_status;
  if (execute_status != RuntimeSdkStatus::kOk) {
    result.message = StatusMessage("session_execute_json failed", execute_status);
  }
  return result;
}

RuntimeCallResult RuntimeSession::SnapshotJson() const {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!api_.IsBound()) {
    return BuildLocalCallError(RuntimeSdkStatus::kApiNotBound,
                               "runtime C ABI table is not fully bound");
  }
  if (handle_ == nullptr) {
    return BuildLocalCallError(RuntimeSdkStatus::kSessionClosed,
                               "session must be opened before SnapshotJson");
  }

  RuntimeCallResult result;
  char* snapshot_json_ptr = nullptr;
  RuntimeSdkStatus snapshot_status =
      ToRuntimeSdkStatus(api_.session_snapshot_json(handle_, &snapshot_json_ptr));
  if (snapshot_json_ptr != nullptr) {
    result.response_json = snapshot_json_ptr;
    api_.string_free(snapshot_json_ptr);
  }
  result.status = snapshot_status;
  if (snapshot_status != RuntimeSdkStatus::kOk) {
    result.message = StatusMessage("session_snapshot_json failed", snapshot_status);
  }
  return result;
}

RuntimeSdkStatus RuntimeSession::Close() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (!api_.IsBound()) {
    return RuntimeSdkStatus::kApiNotBound;
  }
  if (handle_ == nullptr) {
#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
    std::lock_guard<std::mutex> callbacks_lock(callbacks_mutex_);
    event_subscriptions_.clear();
    last_observed_dropped_events_ = 0;
#endif
    return RuntimeSdkStatus::kOk;
  }

  RuntimeSdkStatus close_status = ToRuntimeSdkStatus(api_.session_close(handle_));
  if (close_status == RuntimeSdkStatus::kOk) {
    handle_ = nullptr;
#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
    std::lock_guard<std::mutex> callbacks_lock(callbacks_mutex_);
    event_subscriptions_.clear();
    last_observed_dropped_events_ = 0;
#endif
  }
  return close_status;
}

bool RuntimeSession::is_open() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return handle_ != nullptr;
}

#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
RuntimeCallResult RuntimeSession::GetScopeMetadata(std::string_view request_json) {
  return Execute(RuntimeOperation::kGetScopeMetadata, request_json);
}

RuntimeCallResult RuntimeSession::ListParameters(std::string_view request_json) {
  return Execute(RuntimeOperation::kListParameters, request_json);
}

RuntimeCallResult RuntimeSession::GetParameter(std::string_view request_json) {
  return Execute(RuntimeOperation::kGetParameter, request_json);
}

RuntimeCallResult RuntimeSession::SetParameter(std::string_view request_json) {
  return Execute(RuntimeOperation::kSetParameter, request_json);
}

RuntimeCallResult RuntimeSession::SetParametersAtomically(
    std::string_view request_json) {
  return Execute(RuntimeOperation::kSetParametersAtomically, request_json);
}

RuntimeCallResult RuntimeSession::ListDirtyParameters(std::string_view request_json) {
  return Execute(RuntimeOperation::kListDirtyParameters, request_json);
}

RuntimeCallResult RuntimeSession::GetDirtyMetadata(std::string_view request_json) {
  return Execute(RuntimeOperation::kGetDirtyMetadata, request_json);
}

RuntimeCallResult RuntimeSession::RollbackDirty(std::string_view request_json) {
  return Execute(RuntimeOperation::kRollbackDirty, request_json);
}

RuntimeCallResult RuntimeSession::CommitConfiguration(std::string_view request_json) {
  return Execute(RuntimeOperation::kCommitConfiguration, request_json);
}

RuntimeCallResult RuntimeSession::GetConfigurationIdentity(
    std::string_view request_json) {
  return Execute(RuntimeOperation::kGetConfigurationIdentity, request_json);
}

RuntimeCallResult RuntimeSession::SetAutoResetPolicy(std::string_view request_json) {
  return Execute(RuntimeOperation::kSetAutoResetPolicy, request_json);
}

RuntimeCallResult RuntimeSession::GetAutoResetPolicy(std::string_view request_json) {
  return Execute(RuntimeOperation::kGetAutoResetPolicy, request_json);
}

RuntimeCallResult RuntimeSession::CheckForUpdates(std::string_view request_json) {
  return Execute(RuntimeOperation::kCheckForUpdates, request_json);
}

RuntimeCallResult RuntimeSession::PullUpdates(std::string_view request_json) {
  return Execute(RuntimeOperation::kPullUpdates, request_json);
}

RuntimeCallResult RuntimeSession::GetSyncStatus(std::string_view request_json) {
  return Execute(RuntimeOperation::kGetSyncStatus, request_json);
}

RuntimeCallResult RuntimeSession::SubscribeEvents(std::string_view request_json) {
  return Execute(RuntimeOperation::kSubscribeEvents, request_json);
}

RuntimeCallResult RuntimeSession::PushAuditEvents(std::string_view request_json) {
  return Execute(RuntimeOperation::kPushAuditEvents, request_json);
}

RuntimeCallResult RuntimeSession::ExportPendingSyncBundle(
    std::string_view request_json) {
  return Execute(RuntimeOperation::kExportPendingSyncBundle, request_json);
}
#endif

bool RuntimeSession::HasEmbeddedNul(std::string_view value) {
  return value.find('\0') != std::string_view::npos;
}

RuntimeCallResult RuntimeSession::BuildLocalCallError(RuntimeSdkStatus status,
                                                      std::string message) const {
  RuntimeCallResult result;
  result.status = status;
  result.message = std::move(message);
  return result;
}

RuntimeOpenResult RuntimeSession::BuildLocalOpenError(RuntimeSdkStatus status,
                                                      std::string message) const {
  RuntimeOpenResult result;
  result.status = status;
  result.message = std::move(message);
  return result;
}

}  // namespace configflux::sdk
