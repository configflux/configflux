// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_control_adapter.h"

#include <cmath>
#include <cctype>
#include <iomanip>
#include <sstream>
#include <utility>

#include "configflux/ros2/runtime_parameter_path.h"

namespace configflux::ros2 {

namespace {

constexpr uint32_t kSchemaVersion = 1;

void SkipJsonWhitespace(std::string_view json, size_t* cursor) {
  while (*cursor < json.size() &&
         std::isspace(static_cast<unsigned char>(json[*cursor])) != 0) {
    ++(*cursor);
  }
}

bool ParseJsonString(std::string_view json, size_t* cursor, std::string* out) {
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

bool FindJsonFieldValueStart(std::string_view json, std::string_view field_name,
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

RuntimeControlResult BuildTransportError(sdk::RuntimeSdkStatus status,
                                         std::string reason) {
  RuntimeControlResult result;
  result.status = status;
  result.outcome = RuntimeControlOutcome::kTransportError;
  result.successful = false;
  result.reason = std::move(reason);
  return result;
}

RuntimeControlResult BuildInvalidResponse(std::string response_json,
                                          std::string reason) {
  RuntimeControlResult result;
  result.status = sdk::RuntimeSdkStatus::kInvalidJson;
  result.outcome = RuntimeControlOutcome::kInvalidResponse;
  result.successful = false;
  result.response_json = std::move(response_json);
  result.reason = std::move(reason);
  return result;
}

const char* JsonBool(bool value) { return value ? "true" : "false"; }

}  // namespace

RuntimeControlAdapter::RuntimeControlAdapter(sdk::RuntimeSession* session)
    : session_(session) {}

RuntimeControlResult RuntimeControlAdapter::CommitConfiguration(
    const CommitConfigurationServiceRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(
        sdk::RuntimeSdkStatus::kInvalidArgument,
        "commit_configuration requires runtime_snapshot_json");
  }

  std::string actor = request.context.actor;
  if (actor.empty()) {
    actor = "ros2.commit_configuration";
  }

  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.append(R"(,"actor":")");
  payload.append(EscapeJsonString(actor));
  payload.push_back('"');
  if (request.context.reason.has_value() && !request.context.reason->empty()) {
    payload.append(R"(,"reason":")");
    payload.append(EscapeJsonString(*request.context.reason));
    payload.push_back('"');
  }
  if (request.expected_base_configuration_id.has_value() &&
      !request.expected_base_configuration_id->empty()) {
    payload.append(R"(,"expected_base_configuration_id":")");
    payload.append(EscapeJsonString(*request.expected_base_configuration_id));
    payload.push_back('"');
  }

  payload.append(R"(,"changed_paths_hint":[)");
  for (size_t idx = 0; idx < request.changed_ros_parameter_hints.size(); ++idx) {
    std::string runtime_path;
    if (!parameter_path::NormalizeRuntimePath(request.changed_ros_parameter_hints[idx],
                                              &runtime_path)) {
      return BuildTransportError(
          sdk::RuntimeSdkStatus::kInvalidArgument,
          "invalid changed path hint: " + request.changed_ros_parameter_hints[idx]);
    }
    if (idx != 0) {
      payload.push_back(',');
    }
    payload.push_back('"');
    payload.append(EscapeJsonString(runtime_path));
    payload.push_back('"');
  }
  payload.append("]}");

  return ExecuteMappedCall(std::move(payload), &sdk::RuntimeSession::CommitConfiguration,
                           "commit_configuration");
}

RuntimeControlResult RuntimeControlAdapter::RollbackDirty(
    const RollbackDirtyServiceRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "rollback_dirty requires runtime_snapshot_json");
  }

  std::string actor = request.context.actor;
  if (actor.empty()) {
    actor = "ros2.rollback_dirty";
  }

  const bool subset = !request.rollback_ros_parameter_names.empty();
  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.append(R"(,"mode":")");
  payload.append(subset ? "subset" : "all");
  payload.push_back('"');
  payload.append(R"(,"paths":[)");
  for (size_t idx = 0; idx < request.rollback_ros_parameter_names.size(); ++idx) {
    std::string runtime_path;
    if (!parameter_path::NormalizeRuntimePath(
            request.rollback_ros_parameter_names[idx], &runtime_path)) {
      return BuildTransportError(
          sdk::RuntimeSdkStatus::kInvalidArgument,
          "invalid rollback path: " + request.rollback_ros_parameter_names[idx]);
    }
    if (idx != 0) {
      payload.push_back(',');
    }
    payload.push_back('"');
    payload.append(EscapeJsonString(runtime_path));
    payload.push_back('"');
  }
  payload.append(R"(],"actor":")");
  payload.append(EscapeJsonString(actor));
  payload.push_back('"');
  if (request.context.reason.has_value() && !request.context.reason->empty()) {
    payload.append(R"(,"reason":")");
    payload.append(EscapeJsonString(*request.context.reason));
    payload.push_back('"');
  }
  payload.push_back('}');

  return ExecuteMappedCall(std::move(payload), &sdk::RuntimeSession::RollbackDirty,
                           "rollback_dirty");
}

RuntimeControlResult RuntimeControlAdapter::CheckForUpdates(
    const CheckForUpdatesServiceRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "check_for_updates requires runtime_snapshot_json");
  }

  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.append(R"(,"backend_connected":)");
  payload.append(JsonBool(request.backend_connected));
  if (request.pending_update_summary.has_value() &&
      !request.pending_update_summary->empty()) {
    payload.append(R"(,"pending_update_summary":")");
    payload.append(EscapeJsonString(*request.pending_update_summary));
    payload.push_back('"');
  }
  payload.push_back('}');

  return ExecuteMappedCall(std::move(payload), &sdk::RuntimeSession::CheckForUpdates,
                           "check_for_updates");
}

RuntimeControlResult RuntimeControlAdapter::PullUpdatesAction(
    const PullUpdatesActionRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "pull_updates requires runtime_snapshot_json");
  }

  std::string actor = request.context.actor;
  if (actor.empty()) {
    actor = request.source == PullUpdatesSource::kDirectPush
                ? "ros2.reconcile_offline"
                : "ros2.pull_updates";
  }

  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.append(R"(,"actor":")");
  payload.append(EscapeJsonString(actor));
  payload.push_back('"');
  if (request.context.reason.has_value() && !request.context.reason->empty()) {
    payload.append(R"(,"reason":")");
    payload.append(EscapeJsonString(*request.context.reason));
    payload.push_back('"');
  }
  payload.append(R"(,"backend_connected":)");
  payload.append(JsonBool(request.backend_connected));
  const std::string source_json = PullUpdatesSourceToJson(request.source);
  if (source_json.empty()) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "pull_updates has unknown source");
  }
  payload.append(R"(,"source":")");
  payload.append(source_json);
  payload.push_back('"');
  payload.append(R"(,"writes":[)");

  for (size_t idx = 0; idx < request.writes.size(); ++idx) {
    std::string runtime_path;
    if (!parameter_path::NormalizeRuntimePath(request.writes[idx].ros_parameter_name,
                                              &runtime_path)) {
      return BuildTransportError(
          sdk::RuntimeSdkStatus::kInvalidArgument,
          "invalid pull write path: " + request.writes[idx].ros_parameter_name);
    }
    if (idx != 0) {
      payload.push_back(',');
    }
    payload.append(R"({"path":")");
    payload.append(EscapeJsonString(runtime_path));
    payload.append(R"(","value":)");
    if (const auto* double_value = std::get_if<double>(&request.writes[idx].value);
        double_value != nullptr && !std::isfinite(*double_value)) {
      return BuildTransportError(
          sdk::RuntimeSdkStatus::kInvalidArgument,
          "invalid pull write value for path: " + runtime_path +
              " (non-finite double)");
    }
    payload.append(PullUpdateValueToJson(request.writes[idx].value));
    if (request.writes[idx].before_leaf_hash.has_value() &&
        !request.writes[idx].before_leaf_hash->empty()) {
      payload.append(R"(,"before_leaf_hash":")");
      payload.append(EscapeJsonString(*request.writes[idx].before_leaf_hash));
      payload.push_back('"');
    }
    if (request.writes[idx].after_leaf_hash.has_value() &&
        !request.writes[idx].after_leaf_hash->empty()) {
      payload.append(R"(,"after_leaf_hash":")");
      payload.append(EscapeJsonString(*request.writes[idx].after_leaf_hash));
      payload.push_back('"');
    }
    payload.push_back('}');
  }
  payload.push_back(']');

  if (request.base_configuration_id.has_value() &&
      !request.base_configuration_id->empty()) {
    payload.append(R"(,"base_configuration_id":")");
    payload.append(EscapeJsonString(*request.base_configuration_id));
    payload.push_back('"');
  }
  payload.append(R"(,"full_snapshot":)");
  payload.append(JsonBool(request.full_snapshot));
  if (request.pending_update_summary.has_value() &&
      !request.pending_update_summary->empty()) {
    payload.append(R"(,"pending_update_summary":")");
    payload.append(EscapeJsonString(*request.pending_update_summary));
    payload.push_back('"');
  }
  if (request.target_configuration_id.has_value() &&
      !request.target_configuration_id->empty()) {
    payload.append(R"(,"target_configuration_id":")");
    payload.append(EscapeJsonString(*request.target_configuration_id));
    payload.push_back('"');
  }
  payload.push_back('}');

  return ExecuteMappedCall(std::move(payload), &sdk::RuntimeSession::PullUpdates,
                           "pull_updates");
}

RuntimeControlResult RuntimeControlAdapter::GetSyncStatus(
    const GetSyncStatusServiceRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "get_sync_status requires runtime_snapshot_json");
  }

  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.push_back('}');

  return ExecuteMappedCall(std::move(payload), &sdk::RuntimeSession::GetSyncStatus,
                           "get_sync_status");
}

RuntimeControlResult RuntimeControlAdapter::ExportPendingSyncBundle(
    const ExportPendingSyncBundleServiceRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(
        sdk::RuntimeSdkStatus::kInvalidArgument,
        "export_pending_sync_bundle requires runtime_snapshot_json");
  }

  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.append(R"(,"max_audit_events":)");
  payload.append(std::to_string(request.max_audit_events));
  payload.push_back('}');

  return ExecuteMappedCall(std::move(payload),
                           &sdk::RuntimeSession::ExportPendingSyncBundle,
                           "export_pending_sync_bundle");
}

RuntimeControlResult RuntimeControlAdapter::PushAuditEvents(
    const PushAuditEventsServiceRequest& request) const {
  if (request.runtime_snapshot_json.empty()) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "push_audit_events requires runtime_snapshot_json");
  }

  std::string payload = R"({"schema_version":)";
  payload.append(std::to_string(kSchemaVersion));
  payload.append(R"(,"runtime_snapshot":)");
  payload.append(request.runtime_snapshot_json);
  payload.append(R"(,"backend_connected":)");
  payload.append(JsonBool(request.backend_connected));
  payload.append(R"(,"max_events":)");
  payload.append(std::to_string(request.max_events));
  payload.push_back('}');

  return ExecuteMappedCall(std::move(payload), &sdk::RuntimeSession::PushAuditEvents,
                           "push_audit_events");
}

RuntimeControlResult RuntimeControlAdapter::ExecuteMappedCall(
    std::string request_json, RuntimeCall call, std::string_view operation_name) const {
  if (session_ == nullptr) {
    return BuildTransportError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "runtime session is null");
  }

  const sdk::RuntimeCallResult call_result = (session_->*call)(request_json);
  if (!call_result.ok()) {
    std::string reason(operation_name);
    reason.append(" call failed: ");
    reason.append(call_result.message);
    return BuildTransportError(call_result.status, std::move(reason));
  }

  std::string runtime_status;
  if (!ExtractJsonStringField(call_result.response_json, "status", &runtime_status)) {
    std::string reason(operation_name);
    reason.append(" response missing status");
    return BuildInvalidResponse(call_result.response_json, std::move(reason));
  }

  RuntimeControlResult result;
  result.status = sdk::RuntimeSdkStatus::kOk;
  result.response_json = call_result.response_json;
  result.runtime_status = std::move(runtime_status);

  if (result.runtime_status == "ok") {
    result.outcome = RuntimeControlOutcome::kOk;
    result.successful = true;
    return result;
  }

  result.outcome = RuntimeControlOutcome::kRuntimeError;
  result.successful = false;
  ExtractRuntimeCode(call_result.response_json, &result.runtime_code);

  result.reason.assign(operation_name);
  result.reason.append(" runtime error");
  if (!result.runtime_code.empty()) {
    result.reason.append(": ");
    result.reason.append(result.runtime_code);
  }
  return result;
}

bool RuntimeControlAdapter::ExtractJsonStringField(std::string_view json,
                                                   std::string_view field_name,
                                                   std::string* out) {
  if (out == nullptr) {
    return false;
  }
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }
  return ParseJsonString(json, &value_start, out);
}

bool RuntimeControlAdapter::ExtractRuntimeCode(std::string_view response_json,
                                               std::string* out_code) {
  if (out_code == nullptr) {
    return false;
  }
  out_code->clear();

  if (ExtractJsonStringField(response_json, "code", out_code) &&
      !out_code->empty()) {
    return true;
  }

  size_t search_from = 0;
  while (search_from < response_json.size()) {
    const size_t code_pos = response_json.find("\"code\"", search_from);
    if (code_pos == std::string_view::npos) {
      return false;
    }
    size_t cursor = code_pos + 6;
    SkipJsonWhitespace(response_json, &cursor);
    if (cursor >= response_json.size() || response_json[cursor] != ':') {
      search_from = code_pos + 6;
      continue;
    }
    ++cursor;
    SkipJsonWhitespace(response_json, &cursor);
    if (cursor >= response_json.size() || response_json[cursor] != '"') {
      search_from = code_pos + 6;
      continue;
    }
    if (ParseJsonString(response_json, &cursor, out_code) && !out_code->empty()) {
      return true;
    }
    search_from = code_pos + 6;
  }
  return false;
}

std::string RuntimeControlAdapter::PullUpdateValueToJson(
    const PullUpdateValue& value) {
  if (const auto* bool_value = std::get_if<bool>(&value)) {
    return *bool_value ? "true" : "false";
  }
  if (const auto* int_value = std::get_if<int64_t>(&value)) {
    return std::to_string(*int_value);
  }
  if (const auto* double_value = std::get_if<double>(&value)) {
    std::ostringstream stream;
    stream << std::setprecision(17) << *double_value;
    return stream.str();
  }
  std::string json = "\"";
  json.append(EscapeJsonString(std::get<std::string>(value)));
  json.push_back('"');
  return json;
}

std::string RuntimeControlAdapter::PullUpdatesSourceToJson(
    PullUpdatesSource source) {
  switch (source) {
    case PullUpdatesSource::kBackend:
      return "backend";
    case PullUpdatesSource::kDirectPush:
      return "direct_push";
    default:
      return "";
  }
}

std::string RuntimeControlAdapter::EscapeJsonString(std::string_view value) {
  std::string escaped;
  escaped.reserve(value.size());
  for (const char ch : value) {
    const unsigned char byte = static_cast<unsigned char>(ch);
    switch (ch) {
      case '"':
        escaped.append("\\\"");
        break;
      case '\\':
        escaped.append("\\\\");
        break;
      case '\b':
        escaped.append("\\b");
        break;
      case '\f':
        escaped.append("\\f");
        break;
      case '\n':
        escaped.append("\\n");
        break;
      case '\r':
        escaped.append("\\r");
        break;
      case '\t':
        escaped.append("\\t");
        break;
      default:
        if (byte < 0x20U) {
          constexpr char kHexDigits[] = "0123456789abcdef";
          escaped.append("\\u00");
          escaped.push_back(kHexDigits[(byte >> 4U) & 0x0FU]);
          escaped.push_back(kHexDigits[byte & 0x0FU]);
          break;
        }
        escaped.push_back(static_cast<char>(byte));
        break;
    }
  }
  return escaped;
}

}  // namespace configflux::ros2
