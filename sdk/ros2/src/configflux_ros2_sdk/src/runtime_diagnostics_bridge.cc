// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_diagnostics_bridge.h"

#include <cctype>
#include <utility>

namespace configflux::ros2 {

namespace {

constexpr char kRuntimeHealthKey[] = "runtime.health";
constexpr char kRuntimeDirtyKey[] = "runtime.dirty";
constexpr char kRuntimeCommitKey[] = "runtime.commit";
constexpr char kRuntimeSyncKey[] = "runtime.sync";
constexpr char kRuntimeEventsKey[] = "runtime.events";

bool IsJsonValueTerminator(char ch) {
  return ch == ',' || ch == '}' || ch == ']' ||
         std::isspace(static_cast<unsigned char>(ch)) != 0;
}

const char* BoolToString(bool value) { return value ? "true" : "false"; }

RuntimeDiagnosticStatus BuildBaseStatus(
    const sdk::RuntimeEventNotification& event, std::string status_key) {
  RuntimeDiagnosticStatus status;
  status.status_key = std::move(status_key);
  status.correlation_id = event.event_id;
  status.event_kind = event.event_kind;
  status.sequence = event.sequence;
  status.values.push_back(
      DiagnosticKeyValue{.key = "scope", .value = event.scope});
  return status;
}

void AppendNumericValue(std::vector<DiagnosticKeyValue>* values, std::string key,
                        uint64_t value) {
  values->push_back(
      DiagnosticKeyValue{.key = std::move(key), .value = std::to_string(value)});
}

}  // namespace

void RuntimeDiagnosticsBridge::SkipJsonWhitespace(std::string_view json,
                                                  size_t* cursor) {
  while (*cursor < json.size() &&
         std::isspace(static_cast<unsigned char>(json[*cursor])) != 0) {
    ++(*cursor);
  }
}

bool RuntimeDiagnosticsBridge::ParseJsonString(std::string_view json, size_t* cursor,
                                               std::string* out) {
  if (*cursor >= json.size() || json[*cursor] != '"') {
    return false;
  }

  ++(*cursor);
  out->clear();
  bool escaped = false;
  while (*cursor < json.size()) {
    const char ch = json[*cursor];
    ++(*cursor);
    if (escaped) {
      switch (ch) {
        case '"':
        case '\\':
        case '/':
          out->push_back(ch);
          break;
        case 'b':
          out->push_back('\b');
          break;
        case 'f':
          out->push_back('\f');
          break;
        case 'n':
          out->push_back('\n');
          break;
        case 'r':
          out->push_back('\r');
          break;
        case 't':
          out->push_back('\t');
          break;
        default:
          return false;
      }
      escaped = false;
      continue;
    }

    if (ch == '\\') {
      escaped = true;
      continue;
    }
    if (ch == '"') {
      return true;
    }
    out->push_back(ch);
  }
  return false;
}

bool RuntimeDiagnosticsBridge::FindJsonFieldValueStart(std::string_view json,
                                                       std::string_view field_name,
                                                       size_t* value_start) {
  std::string token;
  token.reserve(field_name.size() + 2);
  token.push_back('"');
  token.append(field_name);
  token.push_back('"');

  size_t search_from = 0;
  while (true) {
    const size_t key_pos = json.find(token, search_from);
    if (key_pos == std::string_view::npos) {
      return false;
    }

    size_t cursor = key_pos + token.size();
    SkipJsonWhitespace(json, &cursor);
    if (cursor >= json.size() || json[cursor] != ':') {
      search_from = key_pos + token.size();
      continue;
    }

    ++cursor;
    SkipJsonWhitespace(json, &cursor);
    if (cursor >= json.size()) {
      return false;
    }
    *value_start = cursor;
    return true;
  }
}

bool RuntimeDiagnosticsBridge::ExtractJsonStringField(std::string_view json,
                                                      std::string_view field_name,
                                                      std::string* out) {
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }

  size_t cursor = value_start;
  return ParseJsonString(json, &cursor, out);
}

bool RuntimeDiagnosticsBridge::ExtractJsonBoolField(std::string_view json,
                                                    std::string_view field_name,
                                                    bool* out) {
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }

  constexpr std::string_view kTrue = "true";
  constexpr std::string_view kFalse = "false";

  if (json.substr(value_start, kTrue.size()) == kTrue) {
    const size_t end = value_start + kTrue.size();
    if (end == json.size() || IsJsonValueTerminator(json[end])) {
      *out = true;
      return true;
    }
  }

  if (json.substr(value_start, kFalse.size()) == kFalse) {
    const size_t end = value_start + kFalse.size();
    if (end == json.size() || IsJsonValueTerminator(json[end])) {
      *out = false;
      return true;
    }
  }

  return false;
}

bool RuntimeDiagnosticsBridge::ShouldTreatAsReconnect(
    const sdk::RuntimeEventNotification& event) const {
  return has_sequence_ && event.event_kind == "runtime_opened" &&
         event.sequence <= last_sequence_;
}

RuntimeDiagnosticStatus RuntimeDiagnosticsBridge::BuildHealthStatus(
    const sdk::RuntimeEventNotification& event, bool reconnect) const {
  RuntimeDiagnosticStatus status = BuildBaseStatus(event, kRuntimeHealthKey);
  status.severity = reconnect ? DiagnosticSeverity::kWarn : DiagnosticSeverity::kOk;
  status.status_code =
      reconnect ? "CFGX_ROS2_HEALTH_RECONNECTED"
                : "CFGX_ROS2_HEALTH_RUNTIME_OPENED";
  status.message = reconnect
                       ? "Runtime reopened with sequence reset."
                       : "Runtime session opened.";
  return status;
}

RuntimeDiagnosticStatus RuntimeDiagnosticsBridge::BuildDirtyStatus(
    const sdk::RuntimeEventNotification& event) const {
  RuntimeDiagnosticStatus status = BuildBaseStatus(event, kRuntimeDirtyKey);
  bool dirty = false;
  if (!ExtractJsonBoolField(event.event_json, "dirty", &dirty)) {
    status.severity = DiagnosticSeverity::kWarn;
    status.status_code = "CFGX_ROS2_DIRTY_STATE_UNKNOWN";
    status.message = "Dirty-state event missing dirty flag.";
    return status;
  }

  status.severity = dirty ? DiagnosticSeverity::kWarn : DiagnosticSeverity::kOk;
  status.status_code =
      dirty ? "CFGX_ROS2_DIRTY_ACTIVE" : "CFGX_ROS2_DIRTY_CLEARED";
  status.message = dirty ? "Runtime has uncommitted dirty parameters."
                         : "Runtime dirty state cleared.";
  status.values.push_back(
      DiagnosticKeyValue{.key = "dirty", .value = BoolToString(dirty)});
  return status;
}

RuntimeDiagnosticStatus RuntimeDiagnosticsBridge::BuildCommitStatus(
    const sdk::RuntimeEventNotification& event) const {
  RuntimeDiagnosticStatus status = BuildBaseStatus(event, kRuntimeCommitKey);
  status.severity = DiagnosticSeverity::kOk;
  status.status_code = "CFGX_ROS2_COMMIT_APPLIED";
  status.message = "Runtime commit applied.";
  return status;
}

RuntimeDiagnosticStatus RuntimeDiagnosticsBridge::BuildSyncStatus(
    const sdk::RuntimeEventNotification& event) const {
  RuntimeDiagnosticStatus status = BuildBaseStatus(event, kRuntimeSyncKey);
  if (event.event_kind == "sync_conflict_detected") {
    status.severity = DiagnosticSeverity::kWarn;
    status.status_code = "CFGX_ROS2_SYNC_CONFLICT_DETECTED";
    status.message = "Sync conflict detected.";
    return status;
  }

  if (event.event_kind == "sync_conflict_resolved") {
    status.severity = DiagnosticSeverity::kWarn;
    status.status_code = "CFGX_ROS2_SYNC_CONFLICT_RESOLVED";
    status.message = "Sync conflict resolved with upstream priority.";
    return status;
  }

  if (event.event_kind == "sync_apply_completed") {
    status.severity = DiagnosticSeverity::kOk;
    status.status_code = "CFGX_ROS2_SYNC_APPLY_COMPLETED";
    status.message = "Sync apply completed.";
    return status;
  }

  std::string state;
  if (!ExtractJsonStringField(event.event_json, "state", &state)) {
    status.severity = DiagnosticSeverity::kWarn;
    status.status_code = "CFGX_ROS2_SYNC_STATE_UNKNOWN";
    status.message = "Sync state event missing state value.";
    return status;
  }

  status.values.push_back(DiagnosticKeyValue{
      .key = "sync_state",
      .value = state,
  });

  if (state == "offline") {
    status.severity = DiagnosticSeverity::kWarn;
    status.status_code = "CFGX_ROS2_SYNC_STATE_OFFLINE";
  } else if (state == "error") {
    status.severity = DiagnosticSeverity::kError;
    status.status_code = "CFGX_ROS2_SYNC_STATE_ERROR";
  } else if (state == "idle") {
    status.severity = DiagnosticSeverity::kOk;
    status.status_code = "CFGX_ROS2_SYNC_STATE_IDLE";
  } else if (state == "checking") {
    status.severity = DiagnosticSeverity::kOk;
    status.status_code = "CFGX_ROS2_SYNC_STATE_CHECKING";
  } else if (state == "pulling") {
    status.severity = DiagnosticSeverity::kOk;
    status.status_code = "CFGX_ROS2_SYNC_STATE_PULLING";
  } else if (state == "applying") {
    status.severity = DiagnosticSeverity::kOk;
    status.status_code = "CFGX_ROS2_SYNC_STATE_APPLYING";
  } else {
    status.severity = DiagnosticSeverity::kWarn;
    status.status_code = "CFGX_ROS2_SYNC_STATE_UNKNOWN";
  }
  status.message = "Sync state is " + state + ".";
  return status;
}

RuntimeDiagnosticStatus RuntimeDiagnosticsBridge::BuildDroppedEventsStatus(
    const sdk::RuntimeEventDispatchResult& dispatch_result) {
  RuntimeDiagnosticStatus status;
  status.severity = DiagnosticSeverity::kWarn;
  status.status_key = kRuntimeEventsKey;
  status.status_code = "CFGX_ROS2_EVENT_BACKPRESSURE_DROPPED";
  status.message = "Runtime event stream dropped events under burst load.";
  status.event_kind = "dispatch_summary";
  status.correlation_id = "dispatch:" +
                          std::to_string(dispatch_result.from_sequence) + "-" +
                          std::to_string(dispatch_result.next_sequence);
  status.sequence = dispatch_result.next_sequence;
  AppendNumericValue(&status.values, "dropped_events", dispatch_result.dropped_events);
  AppendNumericValue(&status.values, "dropped_events_delta",
                     dispatch_result.dropped_events_delta);
  AppendNumericValue(&status.values, "callbacks_invoked",
                     dispatch_result.callbacks_invoked);
  AppendNumericValue(&status.values, "callback_failures",
                     dispatch_result.callback_failures);
  return status;
}

void RuntimeDiagnosticsBridge::RememberStatus(RuntimeDiagnosticStatus status) {
  statuses_[status.status_key] = std::move(status);
}

bool RuntimeDiagnosticsBridge::IngestEvent(const sdk::RuntimeEventNotification& event,
                                           RuntimeDiagnosticStatus* out_status) {
  const bool reconnect = ShouldTreatAsReconnect(event);
  if (!reconnect && has_sequence_ && event.sequence <= last_sequence_) {
    return false;
  }
  if (reconnect) {
    statuses_.clear();
  }

  RuntimeDiagnosticStatus status;
  if (event.event_kind == "runtime_opened") {
    status = BuildHealthStatus(event, reconnect);
  } else if (event.event_kind == "dirty_state_changed") {
    status = BuildDirtyStatus(event);
  } else if (event.event_kind == "commit_applied") {
    status = BuildCommitStatus(event);
  } else if (event.event_kind == "sync_state_changed" ||
             event.event_kind == "sync_conflict_detected" ||
             event.event_kind == "sync_conflict_resolved" ||
             event.event_kind == "sync_apply_completed") {
    status = BuildSyncStatus(event);
  } else {
    last_sequence_ = event.sequence;
    has_sequence_ = true;
    return false;
  }

  last_sequence_ = event.sequence;
  has_sequence_ = true;
  RememberStatus(status);
  if (out_status != nullptr) {
    *out_status = std::move(status);
  }
  return true;
}

std::optional<RuntimeDiagnosticStatus> RuntimeDiagnosticsBridge::ObserveDispatch(
    const sdk::RuntimeEventDispatchResult& dispatch_result) {
  RuntimeDiagnosticStatus status;
  if (!dispatch_result.ok()) {
    status.severity = DiagnosticSeverity::kError;
    status.status_key = kRuntimeEventsKey;
    status.status_code = "CFGX_ROS2_EVENT_DISPATCH_FAILED";
    status.message = dispatch_result.message.empty()
                         ? "Runtime event dispatch failed."
                         : dispatch_result.message;
    status.event_kind = "dispatch_error";
    status.correlation_id = "dispatch:" +
                            std::to_string(dispatch_result.from_sequence) + "-" +
                            std::to_string(dispatch_result.next_sequence);
    status.sequence = dispatch_result.next_sequence;
    AppendNumericValue(&status.values, "callback_failures",
                       dispatch_result.callback_failures);
    RememberStatus(status);
    if (!has_sequence_ || status.sequence > last_sequence_) {
      last_sequence_ = status.sequence;
      has_sequence_ = true;
    }
    return status;
  }

  if (dispatch_result.dropped_events_delta == 0) {
    return std::nullopt;
  }

  status = BuildDroppedEventsStatus(dispatch_result);
  RememberStatus(status);
  if (!has_sequence_ || status.sequence > last_sequence_) {
    last_sequence_ = status.sequence;
    has_sequence_ = true;
  }
  return status;
}

RuntimeDiagnosticsSnapshot RuntimeDiagnosticsBridge::Snapshot() const {
  RuntimeDiagnosticsSnapshot snapshot;
  snapshot.last_sequence = last_sequence_;
  snapshot.statuses.reserve(statuses_.size());
  for (const auto& [key, status] : statuses_) {
    (void)key;
    snapshot.statuses.push_back(status);
  }
  return snapshot;
}

void RuntimeDiagnosticsBridge::Reset() {
  last_sequence_ = 0;
  has_sequence_ = false;
  statuses_.clear();
}

}  // namespace configflux::ros2
