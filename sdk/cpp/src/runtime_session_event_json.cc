// SPDX-License-Identifier: BUSL-1.1

#include "runtime_session_internal.h"

#include <cctype>
#include <limits>

namespace configflux::sdk::internal {

namespace {

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
    uint64_t digit = static_cast<uint64_t>(json[*cursor] - '0');
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
    size_t key_pos = json.find(token, search_from);
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

}  // namespace

bool ExtractJsonStringField(std::string_view json, std::string_view field_name,
                            std::string* out) {
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }
  return ParseJsonString(json, &value_start, out);
}

bool ExtractJsonUnsignedField(std::string_view json, std::string_view field_name,
                              uint64_t* out) {
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }
  return ParseJsonUnsigned(json, &value_start, out);
}

bool ExtractJsonArrayField(std::string_view json, std::string_view field_name,
                           std::string_view* out_array_body) {
  size_t value_start = 0;
  if (!FindJsonFieldValueStart(json, field_name, &value_start)) {
    return false;
  }
  if (value_start >= json.size() || json[value_start] != '[') {
    return false;
  }

  size_t cursor = value_start;
  int depth = 0;
  bool in_string = false;
  bool escaped = false;
  while (cursor < json.size()) {
    char ch = json[cursor];
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
    if (ch == '[') {
      ++depth;
      ++cursor;
      continue;
    }
    if (ch == ']') {
      --depth;
      if (depth == 0) {
        *out_array_body =
            json.substr(value_start + 1, cursor - (value_start + 1));
        return true;
      }
      ++cursor;
      continue;
    }
    ++cursor;
  }
  return false;
}

bool SplitTopLevelJsonObjects(std::string_view array_body,
                              std::vector<std::string>* out_objects) {
  out_objects->clear();
  size_t cursor = 0;
  while (true) {
    SkipJsonWhitespace(array_body, &cursor);
    if (cursor >= array_body.size()) {
      return true;
    }
    if (array_body[cursor] == ',') {
      ++cursor;
      continue;
    }
    if (array_body[cursor] != '{') {
      return false;
    }

    size_t start = cursor;
    int depth = 0;
    bool in_string = false;
    bool escaped = false;
    while (cursor < array_body.size()) {
      char ch = array_body[cursor];
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
      if (ch == '{') {
        ++depth;
        ++cursor;
        continue;
      }
      if (ch == '}') {
        --depth;
        ++cursor;
        if (depth == 0) {
          out_objects->emplace_back(array_body.substr(start, cursor - start));
          break;
        }
        continue;
      }
      ++cursor;
    }
    if (depth != 0) {
      return false;
    }
  }
}

std::optional<RuntimeEventChannel> EventChannelForKind(
    std::string_view event_kind) {
  if (event_kind == "parameter_changed") {
    return RuntimeEventChannel::kParameterChanged;
  }
  if (event_kind == "dirty_state_changed") {
    return RuntimeEventChannel::kDirtyStateChanged;
  }
  if (event_kind == "reset_applied") {
    return RuntimeEventChannel::kAutoResetOccurred;
  }
  if (event_kind == "commit_applied") {
    return RuntimeEventChannel::kCommitApplied;
  }
  if (event_kind == "sync_state_changed" ||
      event_kind == "sync_conflict_detected" ||
      event_kind == "sync_conflict_resolved" ||
      event_kind == "sync_apply_completed") {
    return RuntimeEventChannel::kSyncEvent;
  }
  return std::nullopt;
}

std::string BuildSubscribeEventsRequestJson(uint64_t from_sequence,
                                            uint32_t max_events) {
  const uint32_t clamped_max_events = max_events == 0 ? 1 : max_events;
  // schema_version tracks compiler::product_api::PRODUCT_SCHEMA_VERSION (bumped
  // 2 -> 3 for first-class facets, ADR-0047); keep this literal in sync on the
  // next bump.
  std::string request = R"({"schema_version":3,"from_sequence":)";
  request.append(std::to_string(from_sequence));
  request.append(R"(,"max_events":)");
  request.append(std::to_string(clamped_max_events));
  request.append(R"(,"event_kinds":[]})");
  return request;
}

}  // namespace configflux::sdk::internal
