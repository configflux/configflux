// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "configflux/sdk/runtime_session.h"

namespace configflux::ros2 {

enum class DiagnosticSeverity : uint8_t {
  kOk = 0,
  kWarn = 1,
  kError = 2,
  kStale = 3,
};

struct DiagnosticKeyValue {
  std::string key;
  std::string value;
};

struct RuntimeDiagnosticStatus {
  DiagnosticSeverity severity = DiagnosticSeverity::kOk;
  std::string status_key;
  std::string status_code;
  std::string message;
  std::string event_kind;
  std::string correlation_id;
  uint64_t sequence = 0;
  std::vector<DiagnosticKeyValue> values;
};

struct RuntimeDiagnosticsSnapshot {
  uint64_t last_sequence = 0;
  std::vector<RuntimeDiagnosticStatus> statuses;
};

class RuntimeDiagnosticsBridge {
 public:
  RuntimeDiagnosticsBridge() = default;

  bool IngestEvent(const sdk::RuntimeEventNotification& event,
                   RuntimeDiagnosticStatus* out_status = nullptr);

  std::optional<RuntimeDiagnosticStatus> ObserveDispatch(
      const sdk::RuntimeEventDispatchResult& dispatch_result);

  RuntimeDiagnosticsSnapshot Snapshot() const;
  void Reset();

 private:
  using StatusStore = std::map<std::string, RuntimeDiagnosticStatus>;

  static void SkipJsonWhitespace(std::string_view json, size_t* cursor);
  static bool ParseJsonString(std::string_view json, size_t* cursor,
                              std::string* out);
  static bool FindJsonFieldValueStart(std::string_view json,
                                      std::string_view field_name,
                                      size_t* value_start);
  static bool ExtractJsonStringField(std::string_view json,
                                     std::string_view field_name,
                                     std::string* out);
  static bool ExtractJsonBoolField(std::string_view json,
                                   std::string_view field_name, bool* out);

  bool ShouldTreatAsReconnect(const sdk::RuntimeEventNotification& event) const;
  RuntimeDiagnosticStatus BuildHealthStatus(
      const sdk::RuntimeEventNotification& event, bool reconnect) const;
  RuntimeDiagnosticStatus BuildDirtyStatus(
      const sdk::RuntimeEventNotification& event) const;
  RuntimeDiagnosticStatus BuildCommitStatus(
      const sdk::RuntimeEventNotification& event) const;
  RuntimeDiagnosticStatus BuildSyncStatus(
      const sdk::RuntimeEventNotification& event) const;
  static RuntimeDiagnosticStatus BuildDroppedEventsStatus(
      const sdk::RuntimeEventDispatchResult& dispatch_result);
  void RememberStatus(RuntimeDiagnosticStatus status);

  uint64_t last_sequence_ = 0;
  bool has_sequence_ = false;
  StatusStore statuses_;
};

}  // namespace configflux::ros2
