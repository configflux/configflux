// SPDX-License-Identifier: BUSL-1.1

#include "fake_runtime_c_abi.h"

#include <chrono>
#include <cstdlib>
#include <cstring>
#include <string>
#include <thread>

namespace configflux::sdk::test_support {

FakeRuntimeState g_fake_state;

namespace {

struct FakeSession {
  int id;
};

void UpdateMax(std::atomic<int>* target_max, int candidate) {
  int observed = target_max->load(std::memory_order_relaxed);
  while (candidate > observed &&
         !target_max->compare_exchange_weak(
             observed, candidate, std::memory_order_relaxed,
             std::memory_order_relaxed)) {
  }
}

char* AllocateAbiString(const std::string& value) {
  char* buffer = static_cast<char*>(std::malloc(value.size() + 1));
  if (buffer == nullptr) {
    return nullptr;
  }
  std::memcpy(buffer, value.c_str(), value.size() + 1);
  return buffer;
}

RuntimeAbiVersion FakeAbiVersion() { return RuntimeAbiVersion{1, 0, 0}; }

RuntimeAbiStatus FakeAbiHandshake(uint32_t expected_major, uint32_t expected_minor,
                                  RuntimeAbiVersion* out_version) {
  if (out_version == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_version = FakeAbiVersion();
  if (expected_major != 1 || expected_minor > 0) {
    return RuntimeAbiStatus::kVersionMismatch;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionOpen(const char* runtime_open_request_json,
                                 RuntimeSessionHandle** out_handle,
                                 char** out_response_json) {
  if (out_handle == nullptr || out_response_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_handle = nullptr;
  *out_response_json = nullptr;
  if (runtime_open_request_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }

  const std::string request(runtime_open_request_json);
  if (request.find("\"invalid_json\":true") != std::string::npos) {
    return RuntimeAbiStatus::kInvalidJson;
  }

  if (request.find("\"domain_error\":true") != std::string::npos) {
    *out_response_json =
        AllocateAbiString(R"({"status":"error","code":"E_RUNTIME_OPEN_INVALID"})");
    return RuntimeAbiStatus::kOk;
  }

  auto* session = new FakeSession;
  session->id = g_fake_state.next_session_id.fetch_add(1);
  *out_handle = reinterpret_cast<RuntimeSessionHandle*>(session);
  *out_response_json = AllocateAbiString(R"({"status":"ok"})");
  if (*out_response_json == nullptr) {
    delete session;
    *out_handle = nullptr;
    return RuntimeAbiStatus::kInternalError;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionExecuteJson(RuntimeSessionHandle* handle,
                                        uint32_t operation,
                                        const char* request_json,
                                        char** out_response_json) {
  if (out_response_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_response_json = nullptr;
  if (handle == nullptr) {
    return RuntimeAbiStatus::kInvalidHandle;
  }
  if (request_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }

  int in_flight = g_fake_state.execute_in_flight.fetch_add(1) + 1;
  UpdateMax(&g_fake_state.max_execute_in_flight, in_flight);
  std::this_thread::sleep_for(std::chrono::milliseconds(2));
  g_fake_state.execute_calls.fetch_add(1);

  const std::string request(request_json);
  if (request.find("\"return_invalid_json\":true") != std::string::npos) {
    g_fake_state.execute_in_flight.fetch_sub(1);
    return RuntimeAbiStatus::kInvalidJson;
  }

  if (operation == static_cast<uint32_t>(RuntimeOperation::kSubscribeEvents)) {
    int subscribe_call = g_fake_state.subscribe_calls.fetch_add(1);
    const char* response = nullptr;
    if (subscribe_call == 0) {
      response =
          R"({"status":"ok","from_sequence":0,"next_sequence":7,"dropped_events":2,"events":[{"event_id":"evt-01","sequence":1,"event_kind":"runtime_opened","scope":"component:thermal_control"},{"event_id":"evt-02","sequence":2,"event_kind":"parameter_changed","scope":"component:thermal_control"},{"event_id":"evt-03","sequence":3,"event_kind":"dirty_state_changed","scope":"component:thermal_control"},{"event_id":"evt-04","sequence":4,"event_kind":"reset_applied","scope":"component:thermal_control"},{"event_id":"evt-05","sequence":5,"event_kind":"commit_applied","scope":"component:thermal_control"},{"event_id":"evt-06","sequence":6,"event_kind":"sync_apply_completed","scope":"component:thermal_control"}]})";
    } else if (subscribe_call == 1) {
      response =
          R"({"status":"ok","from_sequence":6,"next_sequence":9,"dropped_events":5,"events":[{"event_id":"evt-07","sequence":7,"event_kind":"parameter_changed","scope":"component:thermal_control"},{"event_id":"evt-08","sequence":8,"event_kind":"sync_state_changed","scope":"component:thermal_control"}]})";
    } else {
      response =
          R"({"status":"ok","from_sequence":8,"next_sequence":9,"dropped_events":5,"events":[]})";
    }
    *out_response_json = AllocateAbiString(response);
    g_fake_state.execute_in_flight.fetch_sub(1);
    if (*out_response_json == nullptr) {
      return RuntimeAbiStatus::kInternalError;
    }
    return RuntimeAbiStatus::kOk;
  }

  std::string response = "{\"status\":\"ok\",\"operation\":";
  response += std::to_string(operation);
  response += "}";
  *out_response_json = AllocateAbiString(response);
  g_fake_state.execute_in_flight.fetch_sub(1);
  if (*out_response_json == nullptr) {
    return RuntimeAbiStatus::kInternalError;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionSnapshotJson(const RuntimeSessionHandle* handle,
                                         char** out_snapshot_json) {
  if (out_snapshot_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_snapshot_json = nullptr;
  if (handle == nullptr) {
    return RuntimeAbiStatus::kInvalidHandle;
  }
  *out_snapshot_json = AllocateAbiString(R"({"snapshot":"ok"})");
  if (*out_snapshot_json == nullptr) {
    return RuntimeAbiStatus::kInternalError;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionClose(RuntimeSessionHandle* handle) {
  if (handle == nullptr) {
    return RuntimeAbiStatus::kInvalidHandle;
  }
  delete reinterpret_cast<FakeSession*>(handle);
  return RuntimeAbiStatus::kOk;
}

void FakeStringFree(char* value) { std::free(value); }

}  // namespace

RuntimeCAbiApi FakeApi() {
  return MakeRuntimeCAbiApi(
      &FakeAbiVersion, &FakeAbiHandshake, &FakeSessionOpen,
      &FakeSessionExecuteJson, &FakeSessionSnapshotJson, &FakeSessionClose,
      &FakeStringFree);
}

void ResetFakeState() {
  g_fake_state.next_session_id.store(1);
  g_fake_state.execute_in_flight.store(0);
  g_fake_state.max_execute_in_flight.store(0);
  g_fake_state.execute_calls.store(0);
  g_fake_state.subscribe_calls.store(0);
}

}  // namespace configflux::sdk::test_support
