// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>

#include "configflux/sdk/runtime_c_abi.h"

extern "C" configflux::sdk::RuntimeAbiVersion configflux_runtime_abi_version();
extern "C" configflux::sdk::RuntimeAbiStatus configflux_runtime_abi_handshake(
    uint32_t expected_major, uint32_t expected_minor,
    configflux::sdk::RuntimeAbiVersion* out_version);
extern "C" configflux::sdk::RuntimeAbiStatus configflux_runtime_session_open(
    const char* runtime_open_request_json,
    configflux::sdk::RuntimeSessionHandle** out_handle,
    char** out_response_json);
extern "C" configflux::sdk::RuntimeAbiStatus
configflux_runtime_session_execute_json(
    configflux::sdk::RuntimeSessionHandle* handle, uint32_t operation,
    const char* request_json, char** out_response_json);
extern "C" configflux::sdk::RuntimeAbiStatus
configflux_runtime_session_snapshot_json(
    const configflux::sdk::RuntimeSessionHandle* handle,
    char** out_snapshot_json);
extern "C" configflux::sdk::RuntimeAbiStatus configflux_runtime_session_close(
    configflux::sdk::RuntimeSessionHandle* handle);
extern "C" void configflux_runtime_string_free(char* value);

namespace configflux::sdk {

inline RuntimeCAbiApi MakeLinkedRuntimeCAbiApi() {
  return MakeRuntimeCAbiApi(
      &::configflux_runtime_abi_version, &::configflux_runtime_abi_handshake,
      &::configflux_runtime_session_open,
      &::configflux_runtime_session_execute_json,
      &::configflux_runtime_session_snapshot_json,
      &::configflux_runtime_session_close, &::configflux_runtime_string_free);
}

}  // namespace configflux::sdk
