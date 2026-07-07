// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "configflux/sdk/runtime_session.h"

namespace configflux::sdk::internal {

bool ExtractJsonStringField(std::string_view json, std::string_view field_name,
                            std::string* out);
bool ExtractJsonUnsignedField(std::string_view json, std::string_view field_name,
                              uint64_t* out);
bool ExtractJsonArrayField(std::string_view json, std::string_view field_name,
                           std::string_view* out_array_body);
bool SplitTopLevelJsonObjects(std::string_view array_body,
                              std::vector<std::string>* out_objects);
std::optional<RuntimeEventChannel> EventChannelForKind(
    std::string_view event_kind);
std::string BuildSubscribeEventsRequestJson(uint64_t from_sequence,
                                            uint32_t max_events);

}  // namespace configflux::sdk::internal
