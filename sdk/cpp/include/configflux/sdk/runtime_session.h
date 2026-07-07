// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>
#include <functional>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

#include "configflux/sdk/runtime_c_abi.h"

namespace configflux::sdk {

enum class RuntimeSdkStatus : uint32_t {
  kOk = static_cast<uint32_t>(RuntimeAbiStatus::kOk),
  kNullPointer = static_cast<uint32_t>(RuntimeAbiStatus::kNullPointer),
  kInvalidUtf8 = static_cast<uint32_t>(RuntimeAbiStatus::kInvalidUtf8),
  kInvalidJson = static_cast<uint32_t>(RuntimeAbiStatus::kInvalidJson),
  kVersionMismatch = static_cast<uint32_t>(RuntimeAbiStatus::kVersionMismatch),
  kInvalidHandle = static_cast<uint32_t>(RuntimeAbiStatus::kInvalidHandle),
  kUnsupportedOperation =
      static_cast<uint32_t>(RuntimeAbiStatus::kUnsupportedOperation),
  kInternalError = static_cast<uint32_t>(RuntimeAbiStatus::kInternalError),
  kCStringContainsNul =
      static_cast<uint32_t>(RuntimeAbiStatus::kCStringContainsNul),
  kApiNotBound = 100,
  kSessionClosed = 101,
  kInvalidArgument = 102,
};

RuntimeSdkStatus ToRuntimeSdkStatus(RuntimeAbiStatus status);
const char* RuntimeSdkStatusName(RuntimeSdkStatus status);

struct RuntimeCallResult {
  RuntimeSdkStatus status = RuntimeSdkStatus::kOk;
  std::string response_json;
  std::string message;

  bool ok() const { return status == RuntimeSdkStatus::kOk; }
};

struct RuntimeOpenResult {
  RuntimeSdkStatus status = RuntimeSdkStatus::kOk;
  RuntimeAbiVersion abi_version{0, 0, 0};
  std::string open_response_json;
  std::string message;

  bool ok() const { return status == RuntimeSdkStatus::kOk; }
};

#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
enum class RuntimeEventChannel : uint8_t {
  kParameterChanged = 0,
  kDirtyStateChanged = 1,
  kAutoResetOccurred = 2,
  kCommitApplied = 3,
  kSyncEvent = 4,
};

enum class RuntimeCallbackAction : uint8_t {
  kKeepSubscription = 0,
  kUnsubscribe = 1,
};

struct RuntimeEventNotification {
  uint64_t sequence = 0;
  std::string event_id;
  std::string event_kind;
  std::string scope;
  std::string event_json;
};

using RuntimeEventCallback =
    std::function<RuntimeCallbackAction(const RuntimeEventNotification&)>;

struct RuntimeEventSubscription {
  uint64_t id = 0;
  RuntimeEventChannel channel = RuntimeEventChannel::kSyncEvent;

  bool valid() const { return id != 0; }
};

struct RuntimeEventDispatchResult {
  RuntimeSdkStatus status = RuntimeSdkStatus::kOk;
  uint64_t from_sequence = 0;
  uint64_t next_sequence = 0;
  uint64_t dropped_events = 0;
  uint64_t dropped_events_delta = 0;
  uint32_t callbacks_invoked = 0;
  uint32_t callback_failures = 0;
  std::string message;

  bool ok() const { return status == RuntimeSdkStatus::kOk; }
};
#endif

// Thread-safety contract:
// - all public methods are safe to call concurrently from multiple threads.
// - operation dispatch is serialized per session handle.
class RuntimeSession {
 public:
  explicit RuntimeSession(RuntimeCAbiApi api);
  ~RuntimeSession();

  RuntimeSession(const RuntimeSession&) = delete;
  RuntimeSession& operator=(const RuntimeSession&) = delete;

  RuntimeSession(RuntimeSession&& other) noexcept;
  RuntimeSession& operator=(RuntimeSession&& other) noexcept;

  RuntimeOpenResult Open(std::string_view runtime_open_request_json,
                         uint32_t expected_abi_major = 1,
                         uint32_t expected_abi_minor = 0);
  RuntimeCallResult Execute(RuntimeOperation operation,
                            std::string_view request_json);
  RuntimeCallResult SnapshotJson() const;
  RuntimeSdkStatus Close();

  bool is_open() const;

#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
  RuntimeCallResult GetScopeMetadata(std::string_view request_json);
  RuntimeCallResult ListParameters(std::string_view request_json);
  RuntimeCallResult GetParameter(std::string_view request_json);
  RuntimeCallResult SetParameter(std::string_view request_json);
  RuntimeCallResult SetParametersAtomically(std::string_view request_json);
  RuntimeCallResult ListDirtyParameters(std::string_view request_json);
  RuntimeCallResult GetDirtyMetadata(std::string_view request_json);
  RuntimeCallResult RollbackDirty(std::string_view request_json);
  RuntimeCallResult CommitConfiguration(std::string_view request_json);
  RuntimeCallResult GetConfigurationIdentity(std::string_view request_json);
  RuntimeCallResult SetAutoResetPolicy(std::string_view request_json);
  RuntimeCallResult GetAutoResetPolicy(std::string_view request_json);
  RuntimeCallResult CheckForUpdates(std::string_view request_json);
  RuntimeCallResult PullUpdates(std::string_view request_json);
  RuntimeCallResult GetSyncStatus(std::string_view request_json);
  RuntimeCallResult SubscribeEvents(std::string_view request_json);
  RuntimeCallResult PushAuditEvents(std::string_view request_json);
  RuntimeCallResult ExportPendingSyncBundle(std::string_view request_json);

  // Event callback contract:
  // - callbacks are invoked synchronously on the caller thread.
  // - callback invocation order matches runtime event sequence ordering.
  // - callback exceptions are contained and counted as failures.
  RuntimeEventSubscription SubscribeEventChannel(RuntimeEventChannel channel,
                                                 RuntimeEventCallback callback);
  RuntimeSdkStatus UnsubscribeEventChannel(RuntimeEventSubscription subscription);
  RuntimeEventDispatchResult PollAndDispatchEvents(uint64_t from_sequence,
                                                   uint32_t max_events = 256);
  RuntimeEventDispatchResult DispatchEventsFromSubscribeResponse(
      std::string_view subscribe_response_json);
#endif

 private:
#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
  struct EventSubscriptionRecord {
    RuntimeEventChannel channel = RuntimeEventChannel::kSyncEvent;
    RuntimeEventCallback callback;
  };

  RuntimeEventDispatchResult BuildLocalDispatchError(RuntimeSdkStatus status,
                                                     std::string message) const;
#endif
  static bool HasEmbeddedNul(std::string_view value);
  RuntimeCallResult BuildLocalCallError(RuntimeSdkStatus status,
                                        std::string message) const;
  RuntimeOpenResult BuildLocalOpenError(RuntimeSdkStatus status,
                                        std::string message) const;

  RuntimeCAbiApi api_;
  mutable std::mutex mutex_;
  RuntimeSessionHandle* handle_ = nullptr;
#if !defined(CONFIGFLUX_SDK_MINIMAL_PROFILE) || CONFIGFLUX_SDK_MINIMAL_PROFILE == 0
  mutable std::mutex callbacks_mutex_;
  uint64_t next_subscription_id_ = 1;
  uint64_t last_observed_dropped_events_ = 0;
  std::unordered_map<uint64_t, EventSubscriptionRecord> event_subscriptions_;
#endif
};

}  // namespace configflux::sdk
