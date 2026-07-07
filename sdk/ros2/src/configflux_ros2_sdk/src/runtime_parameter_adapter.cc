// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_parameter_adapter.h"

#include <cctype>
#include <iomanip>
#include <limits>
#include <optional>
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
    char ch = json[*cursor];
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

bool ParseJsonUnsigned(std::string_view json, size_t* cursor, uint64_t* out) {
  if (*cursor >= json.size() ||
      std::isdigit(static_cast<unsigned char>(json[*cursor])) == 0) {
    return false;
  }

  uint64_t value = 0;
  while (*cursor < json.size() &&
         std::isdigit(static_cast<unsigned char>(json[*cursor])) != 0) {
    const uint64_t digit = static_cast<uint64_t>(json[*cursor] - '0');
    if (value > (std::numeric_limits<uint64_t>::max() - digit) / 10) {
      return false;
    }
    value = (value * 10) + digit;
    ++(*cursor);
  }

  *out = value;
  return true;
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

bool ExtractJsonValueField(std::string_view json, std::string_view field_name,
                           std::string_view* out_value_token) {
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }

  size_t cursor = value_start;
  const char first = json[cursor];
  if (first == '"') {
    std::string ignored;
    if (!ParseJsonString(json, &cursor, &ignored)) {
      return false;
    }
    *out_value_token = json.substr(value_start, cursor - value_start);
    return true;
  }

  if (first == '{' || first == '[') {
    const char open = first;
    const char close = open == '{' ? '}' : ']';
    int depth = 0;
    bool in_string = false;
    bool escaped = false;
    while (cursor < json.size()) {
      const char ch = json[cursor];
      if (in_string) {
        if (escaped) {
          escaped = false;
        } else if (ch == '\\') {
          escaped = true;
        } else if (ch == '"') {
          in_string = false;
        }
        ++cursor;
        continue;
      }

      if (ch == '"') {
        in_string = true;
        ++cursor;
        continue;
      }
      if (ch == open) {
        ++depth;
        ++cursor;
        continue;
      }
      if (ch == close) {
        --depth;
        ++cursor;
        if (depth == 0) {
          *out_value_token = json.substr(value_start, cursor - value_start);
          return true;
        }
        continue;
      }
      ++cursor;
    }
    return false;
  }

  while (cursor < json.size()) {
    const char ch = json[cursor];
    if (ch == ',' || ch == '}' || ch == ']') {
      break;
    }
    ++cursor;
  }
  size_t end = cursor;
  while (end > value_start &&
         std::isspace(static_cast<unsigned char>(json[end - 1])) != 0) {
    --end;
  }
  *out_value_token = json.substr(value_start, end - value_start);
  return !out_value_token->empty();
}

bool ExtractJsonStringField(std::string_view json, std::string_view field_name,
                            std::string* out) {
  std::string_view value_token;
  if (!ExtractJsonValueField(json, field_name, &value_token)) {
    return false;
  }
  size_t cursor = 0;
  return ParseJsonString(value_token, &cursor, out) &&
         cursor == value_token.size();
}

bool ExtractJsonUnsignedField(std::string_view json, std::string_view field_name,
                              uint64_t* out) {
  std::string_view value_token;
  if (!ExtractJsonValueField(json, field_name, &value_token)) {
    return false;
  }
  size_t cursor = 0;
  return ParseJsonUnsigned(value_token, &cursor, out) &&
         cursor == value_token.size();
}

bool ParseJsonStringArrayToken(std::string_view value_token,
                               std::vector<std::string>* out_values) {
  if (value_token.size() < 2 || value_token.front() != '[' ||
      value_token.back() != ']') {
    return false;
  }

  out_values->clear();
  size_t cursor = 1;
  while (cursor + 1 <= value_token.size()) {
    SkipJsonWhitespace(value_token, &cursor);
    if (cursor >= value_token.size() - 1) {
      return true;
    }
    if (value_token[cursor] == ',') {
      ++cursor;
      continue;
    }

    std::string value;
    if (!ParseJsonString(value_token, &cursor, &value)) {
      return false;
    }
    out_values->push_back(std::move(value));
    SkipJsonWhitespace(value_token, &cursor);
    if (cursor < value_token.size() && value_token[cursor] == ',') {
      ++cursor;
    }
  }

  return true;
}

bool ParseBoolToken(std::string_view value_token, bool* out_value) {
  if (value_token == "true") {
    *out_value = true;
    return true;
  }
  if (value_token == "false") {
    *out_value = false;
    return true;
  }
  return false;
}

bool ParseInt64Token(std::string_view value_token, int64_t* out_value) {
  try {
    const std::string value_string(value_token);
    size_t consumed = 0;
    const long long parsed = std::stoll(value_string, &consumed, 10);
    if (consumed != value_string.size()) {
      return false;
    }
    *out_value = static_cast<int64_t>(parsed);
    return true;
  } catch (...) {
    return false;
  }
}

bool ParseDoubleToken(std::string_view value_token, double* out_value) {
  try {
    const std::string value_string(value_token);
    size_t consumed = 0;
    const double parsed = std::stod(value_string, &consumed);
    if (consumed != value_string.size()) {
      return false;
    }
    *out_value = parsed;
    return true;
  } catch (...) {
    return false;
  }
}

bool ParseStringToken(std::string_view value_token, std::string* out_value) {
  size_t cursor = 0;
  return ParseJsonString(value_token, &cursor, out_value) &&
         cursor == value_token.size();
}

bool ParseParameterValueToken(std::string_view runtime_type,
                              std::string_view value_token,
                              ParameterValue* out_value) {
  if (runtime_type == "boolean" || runtime_type == "bool") {
    bool parsed = false;
    if (!ParseBoolToken(value_token, &parsed)) {
      return false;
    }
    *out_value = parsed;
    return true;
  }

  if (runtime_type == "integer" || runtime_type == "int") {
    int64_t parsed = 0;
    if (!ParseInt64Token(value_token, &parsed)) {
      return false;
    }
    *out_value = parsed;
    return true;
  }

  if (runtime_type == "float" || runtime_type == "double") {
    double parsed = 0.0;
    if (!ParseDoubleToken(value_token, &parsed)) {
      return false;
    }
    *out_value = parsed;
    return true;
  }

  if (runtime_type == "string" || runtime_type == "artifact") {
    std::string parsed;
    if (!ParseStringToken(value_token, &parsed)) {
      return false;
    }
    *out_value = std::move(parsed);
    return true;
  }

  if (value_token == "true" || value_token == "false") {
    bool parsed = false;
    if (!ParseBoolToken(value_token, &parsed)) {
      return false;
    }
    *out_value = parsed;
    return true;
  }
  if (!value_token.empty() && value_token.front() == '"') {
    std::string parsed;
    if (!ParseStringToken(value_token, &parsed)) {
      return false;
    }
    *out_value = std::move(parsed);
    return true;
  }
  if (value_token.find_first_of(".eE") != std::string_view::npos) {
    double parsed = 0.0;
    if (!ParseDoubleToken(value_token, &parsed)) {
      return false;
    }
    *out_value = parsed;
    return true;
  }
  int64_t parsed = 0;
  if (!ParseInt64Token(value_token, &parsed)) {
    return false;
  }
  *out_value = parsed;
  return true;
}

std::string EscapeJsonString(std::string_view value) {
  std::string escaped;
  escaped.reserve(value.size());
  for (const char ch : value) {
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
        escaped.push_back(ch);
        break;
    }
  }
  return escaped;
}

AdapterResult BuildAdapterError(sdk::RuntimeSdkStatus status,
                                std::string reason) {
  AdapterResult result;
  result.status = status;
  result.successful = false;
  result.reason = std::move(reason);
  return result;
}

sdk::RuntimeCallResult BuildCallError(sdk::RuntimeSdkStatus status,
                                      std::string reason) {
  sdk::RuntimeCallResult result;
  result.status = status;
  result.message = std::move(reason);
  return result;
}

AtomicSetResult BuildAtomicSetError(sdk::RuntimeSdkStatus status,
                                    std::string reason) {
  AtomicSetResult result;
  result.status = status;
  result.successful = false;
  result.reason = std::move(reason);
  return result;
}

}  // namespace

RuntimeParameterAdapter::RuntimeParameterAdapter(sdk::RuntimeSession* session)
    : session_(session) {}

bool RuntimeParameterAdapter::RuntimePathToRosName(std::string_view runtime_path,
                                                   std::string* ros_name) {
  return parameter_path::RuntimePathToRosName(runtime_path, ros_name);
}

bool RuntimeParameterAdapter::RosNameToRuntimePath(std::string_view ros_name,
                                                   std::string* runtime_path) {
  return parameter_path::RosNameToRuntimePath(ros_name, runtime_path);
}

sdk::RuntimeCallResult RuntimeParameterAdapter::ListRuntimeParameterNames(
    std::string_view scope_root, std::vector<std::string>* out_runtime_paths) const {
  if (session_ == nullptr) {
    return BuildCallError(sdk::RuntimeSdkStatus::kInvalidArgument,
                          "runtime session is null");
  }
  if (out_runtime_paths == nullptr) {
    return BuildCallError(sdk::RuntimeSdkStatus::kInvalidArgument,
                          "output vector is null");
  }

  std::string request = R"({"schema_version":)";
  request.append(std::to_string(kSchemaVersion));
  request.append(R"(,"scope_root":")");
  request.append(EscapeJsonString(scope_root));
  request.append(R"("})");

  auto call_result = session_->ListParameters(request);
  if (!call_result.ok()) {
    return call_result;
  }

  bool status_ok = false;
  if (!ParseStatusIsOk(call_result.response_json, &status_ok)) {
    call_result.status = sdk::RuntimeSdkStatus::kInvalidJson;
    call_result.message = "list_parameters response missing status";
    return call_result;
  }
  if (!status_ok) {
    call_result.message = "list_parameters returned runtime error";
    return call_result;
  }

  std::string parse_failure;
  if (!ParseListParameterPaths(call_result.response_json, out_runtime_paths,
                               &parse_failure)) {
    call_result.status = sdk::RuntimeSdkStatus::kInvalidJson;
    call_result.message = std::move(parse_failure);
  }
  return call_result;
}

AdapterResult RuntimeParameterAdapter::ReadParameters(
    std::string_view scope_root,
    std::vector<RuntimeBackedParameter>* out_parameters) const {
  if (out_parameters == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "output vector is null");
  }

  std::vector<std::string> runtime_paths;
  const auto list_result = ListRuntimeParameterNames(scope_root, &runtime_paths);
  if (!list_result.ok()) {
    return BuildAdapterError(list_result.status,
                             "list_parameters failed: " + list_result.message);
  }

  bool status_ok = false;
  if (!ParseStatusIsOk(list_result.response_json, &status_ok)) {
    return BuildAdapterError(
        sdk::RuntimeSdkStatus::kInvalidJson,
        "list_parameters response missing top-level status");
  }
  if (!status_ok) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kOk,
                             "list_parameters runtime failure");
  }

  out_parameters->clear();
  out_parameters->reserve(runtime_paths.size());
  for (const std::string& path : runtime_paths) {
    RuntimeBackedParameter parameter;
    AdapterResult fetch_result = FetchParameterByRuntimePath(path, &parameter);
    if (fetch_result.status != sdk::RuntimeSdkStatus::kOk ||
        !fetch_result.successful) {
      return fetch_result;
    }
    out_parameters->push_back(std::move(parameter));
  }

  return AdapterResult{};
}

AdapterResult RuntimeParameterAdapter::SetParameter(
    std::string_view ros_parameter_name, const ParameterValue& value,
    RuntimeBackedParameter* out_parameter) const {
  if (session_ == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "runtime session is null");
  }
  if (out_parameter == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "output parameter is null");
  }

  std::string runtime_path;
  if (!RosNameToRuntimePath(ros_parameter_name, &runtime_path)) {
    return BuildAdapterError(
        sdk::RuntimeSdkStatus::kInvalidArgument,
        "invalid ROS parameter name: " + std::string(ros_parameter_name));
  }

  std::string request = R"({"schema_version":)";
  request.append(std::to_string(kSchemaVersion));
  request.append(R"(,"path":")");
  request.append(EscapeJsonString(runtime_path));
  request.append(R"(","value":)");
  request.append(ParameterValueToJson(value));
  request.append("}");

  auto call_result = session_->SetParameter(request);
  if (!call_result.ok()) {
    return BuildAdapterError(call_result.status,
                             "set_parameter call failed: " + call_result.message);
  }

  bool status_ok = false;
  if (!ParseStatusIsOk(call_result.response_json, &status_ok)) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidJson,
                             "set_parameter response missing status");
  }
  if (!status_ok) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kOk, call_result.response_json);
  }

  std::string parse_failure;
  if (!ParseParameterFromGetResponse(call_result.response_json, out_parameter,
                                     &parse_failure)) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidJson,
                             "set_parameter parse failure: " + parse_failure);
  }

  return AdapterResult{};
}

AtomicSetResult RuntimeParameterAdapter::SetParametersAtomically(
    const std::vector<ParameterWrite>& writes, std::string_view actor,
    std::optional<std::string_view> reason) const {
  if (session_ == nullptr) {
    return BuildAtomicSetError(sdk::RuntimeSdkStatus::kInvalidArgument,
                               "runtime session is null");
  }

  if (actor.empty()) {
    actor = "ros2_parameter_adapter";
  }

  std::string request = R"({"schema_version":)";
  request.append(std::to_string(kSchemaVersion));
  request.append(R"(,"writes":[)");
  for (size_t idx = 0; idx < writes.size(); ++idx) {
    std::string runtime_path;
    if (!RosNameToRuntimePath(writes[idx].ros_parameter_name, &runtime_path)) {
      return BuildAtomicSetError(
          sdk::RuntimeSdkStatus::kInvalidArgument,
          "invalid ROS parameter name: " + writes[idx].ros_parameter_name);
    }
    if (idx != 0) {
      request.push_back(',');
    }
    request.append(R"({"path":")");
    request.append(EscapeJsonString(runtime_path));
    request.append(R"(","value":)");
    request.append(ParameterValueToJson(writes[idx].value));
    request.push_back('}');
  }
  request.append(R"(],"actor":")");
  request.append(EscapeJsonString(actor));
  request.push_back('"');
  if (reason.has_value() && !reason->empty()) {
    request.append(R"(,"reason":")");
    request.append(EscapeJsonString(*reason));
    request.push_back('"');
  }
  request.push_back('}');

  AtomicSetResult result;
  const auto call_result = session_->SetParametersAtomically(request);
  if (!call_result.ok()) {
    result.status = call_result.status;
    result.reason = "set_parameters_atomically call failed: " + call_result.message;
    return result;
  }

  bool status_ok = false;
  if (!ParseStatusIsOk(call_result.response_json, &status_ok)) {
    return BuildAtomicSetError(sdk::RuntimeSdkStatus::kInvalidJson,
                               "set_parameters_atomically response missing status");
  }

  std::vector<std::string> rejected_paths;
  std::string parse_failure;
  if (!ParseAtomicSetSummary(call_result.response_json, &result.applied_count,
                             &rejected_paths, &parse_failure)) {
    return BuildAtomicSetError(
        sdk::RuntimeSdkStatus::kInvalidJson,
        "set_parameters_atomically parse failure: " + parse_failure);
  }

  result.status = sdk::RuntimeSdkStatus::kOk;
  result.successful = status_ok;
  result.reason = status_ok ? std::string{} : call_result.response_json;
  for (const std::string& rejected_path : rejected_paths) {
    std::string ros_name;
    if (RuntimePathToRosName(rejected_path, &ros_name)) {
      result.rejected_parameter_names.push_back(std::move(ros_name));
    } else {
      result.rejected_parameter_names.push_back(rejected_path);
    }
  }
  if (!result.rejected_parameter_names.empty()) {
    result.successful = false;
  }
  return result;
}

AdapterResult RuntimeParameterAdapter::FetchParameterByRuntimePath(
    std::string_view runtime_path, RuntimeBackedParameter* out_parameter) const {
  if (session_ == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "runtime session is null");
  }
  if (out_parameter == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "output parameter is null");
  }

  std::string request = R"({"schema_version":)";
  request.append(std::to_string(kSchemaVersion));
  request.append(R"(,"path":")");
  request.append(EscapeJsonString(runtime_path));
  request.append(R"("})");

  auto call_result = session_->GetParameter(request);
  if (!call_result.ok()) {
    return BuildAdapterError(call_result.status,
                             "get_parameter call failed: " + call_result.message);
  }

  bool status_ok = false;
  if (!ParseStatusIsOk(call_result.response_json, &status_ok)) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidJson,
                             "get_parameter response missing status");
  }
  if (!status_ok) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kOk, call_result.response_json);
  }

  std::string parse_failure;
  if (!ParseParameterFromGetResponse(call_result.response_json, out_parameter,
                                     &parse_failure)) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidJson,
                             "get_parameter parse failure: " + parse_failure);
  }

  if (out_parameter->runtime_path.empty()) {
    out_parameter->runtime_path = std::string(runtime_path);
  }
  return AdapterResult{};
}

std::string RuntimeParameterAdapter::ParameterValueToJson(
    const ParameterValue& value) {
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
  const auto& string_value = std::get<std::string>(value);
  std::string json = "\"";
  json.append(EscapeJsonString(string_value));
  json.push_back('"');
  return json;
}

bool RuntimeParameterAdapter::ParseParameterFromGetResponse(
    std::string_view response_json, RuntimeBackedParameter* out_parameter,
    std::string* failure_reason) {
  if (out_parameter == nullptr || failure_reason == nullptr) {
    return false;
  }

  std::string_view parameter_token;
  if (!ExtractJsonValueField(response_json, "parameter", &parameter_token)) {
    *failure_reason = "missing parameter field";
    return false;
  }
  if (parameter_token == "null") {
    *failure_reason = "parameter field is null";
    return false;
  }
  if (parameter_token.size() < 2 || parameter_token.front() != '{' ||
      parameter_token.back() != '}') {
    *failure_reason = "parameter field is not an object";
    return false;
  }

  std::string runtime_path;
  if (!ExtractJsonStringField(parameter_token, "path", &runtime_path)) {
    *failure_reason = "parameter.path missing";
    return false;
  }

  std::string runtime_type;
  if (!ExtractJsonStringField(parameter_token, "type", &runtime_type)) {
    *failure_reason = "parameter.type missing";
    return false;
  }

  std::string_view value_token;
  if (!ExtractJsonValueField(parameter_token, "value", &value_token)) {
    *failure_reason = "parameter.value missing";
    return false;
  }

  ParameterValue value;
  if (!ParseParameterValueToken(runtime_type, value_token, &value)) {
    *failure_reason = "parameter.value parse failed";
    return false;
  }

  std::string ros_name;
  if (!RuntimePathToRosName(runtime_path, &ros_name)) {
    ros_name = runtime_path;
  }

  out_parameter->runtime_path = std::move(runtime_path);
  out_parameter->ros_parameter_name = std::move(ros_name);
  out_parameter->runtime_type = std::move(runtime_type);
  out_parameter->value = std::move(value);
  return true;
}

bool RuntimeParameterAdapter::ParseListParameterPaths(
    std::string_view response_json, std::vector<std::string>* out_paths,
    std::string* failure_reason) {
  if (out_paths == nullptr || failure_reason == nullptr) {
    return false;
  }
  std::string_view value_token;
  if (!ExtractJsonValueField(response_json, "parameter_paths", &value_token)) {
    *failure_reason = "missing parameter_paths field";
    return false;
  }
  if (!ParseJsonStringArrayToken(value_token, out_paths)) {
    *failure_reason = "parameter_paths is not a string array";
    return false;
  }
  return true;
}

bool RuntimeParameterAdapter::ParseStatusIsOk(std::string_view response_json,
                                              bool* out_ok) {
  if (out_ok == nullptr) {
    return false;
  }
  std::string status;
  if (!ExtractJsonStringField(response_json, "status", &status)) {
    return false;
  }
  *out_ok = status == "ok";
  return true;
}

bool RuntimeParameterAdapter::ParseAtomicSetSummary(
    std::string_view response_json, uint32_t* out_applied_count,
    std::vector<std::string>* out_rejected_paths, std::string* failure_reason) {
  if (out_applied_count == nullptr || out_rejected_paths == nullptr ||
      failure_reason == nullptr) {
    return false;
  }

  uint64_t applied_count = 0;
  if (!ExtractJsonUnsignedField(response_json, "applied_count", &applied_count)) {
    *failure_reason = "missing applied_count";
    return false;
  }
  if (applied_count > std::numeric_limits<uint32_t>::max()) {
    *failure_reason = "applied_count out of range";
    return false;
  }
  *out_applied_count = static_cast<uint32_t>(applied_count);

  std::string_view rejected_token;
  if (!ExtractJsonValueField(response_json, "rejected_paths", &rejected_token)) {
    out_rejected_paths->clear();
    return true;
  }
  if (!ParseJsonStringArrayToken(rejected_token, out_rejected_paths)) {
    *failure_reason = "rejected_paths is not a string array";
    return false;
  }
  return true;
}

}  // namespace configflux::ros2
