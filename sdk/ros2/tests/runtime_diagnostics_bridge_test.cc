// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_diagnostics_bridge.h"

#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

using configflux::ros2::DiagnosticSeverity;
using configflux::ros2::RuntimeDiagnosticStatus;
using configflux::ros2::RuntimeDiagnosticsBridge;
using configflux::ros2::RuntimeDiagnosticsSnapshot;
using configflux::sdk::RuntimeEventDispatchResult;
using configflux::sdk::RuntimeEventNotification;
using configflux::sdk::RuntimeSdkStatus;

RuntimeEventNotification MakeEvent(uint64_t sequence, std::string event_id,
                                   std::string event_kind,
                                   std::string event_json) {
  RuntimeEventNotification event;
  event.sequence = sequence;
  event.event_id = std::move(event_id);
  event.event_kind = std::move(event_kind);
  event.scope = "component:thermal_control";
  event.event_json = std::move(event_json);
  return event;
}

const RuntimeDiagnosticStatus* FindStatusByKey(
    const RuntimeDiagnosticsSnapshot& snapshot, std::string_view key) {
  for (const auto& status : snapshot.statuses) {
    if (status.status_key == key) {
      return &status;
    }
  }
  return nullptr;
}

#define CHECK_TRUE(expr)                                                        \
  do {                                                                          \
    if (!(expr)) {                                                              \
      std::cerr << "check failed at " << __FILE__ << ":" << __LINE__ << ": "   \
                << #expr << std::endl;                                          \
      return false;                                                             \
    }                                                                           \
  } while (false)

#define CHECK_EQ(lhs, rhs) CHECK_TRUE((lhs) == (rhs))

bool TestEventMappingAndCorrelationIds() {
  RuntimeDiagnosticsBridge bridge;
  RuntimeDiagnosticStatus mapped;

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(1, "evt-open-1", "runtime_opened",
                R"({"event_kind":"runtime_opened","payload":{"scope_root":"component:thermal_control"}})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.health");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_HEALTH_RUNTIME_OPENED");
  CHECK_EQ(mapped.severity, DiagnosticSeverity::kOk);
  CHECK_EQ(mapped.correlation_id, "evt-open-1");

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(2, "evt-dirty-2", "dirty_state_changed",
                R"({"event_kind":"dirty_state_changed","payload":{"dirty":true}})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.dirty");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_DIRTY_ACTIVE");
  CHECK_EQ(mapped.severity, DiagnosticSeverity::kWarn);
  CHECK_EQ(mapped.correlation_id, "evt-dirty-2");

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(3, "evt-commit-3", "commit_applied",
                R"({"event_kind":"commit_applied","payload":{"commit_id":"commit-3"}})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.commit");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_COMMIT_APPLIED");
  CHECK_EQ(mapped.severity, DiagnosticSeverity::kOk);
  CHECK_EQ(mapped.correlation_id, "evt-commit-3");

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(4, "evt-sync-4", "sync_state_changed",
                R"({"event_kind":"sync_state_changed","payload":{"state":"offline"}})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.sync");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_SYNC_STATE_OFFLINE");
  CHECK_EQ(mapped.severity, DiagnosticSeverity::kWarn);
  CHECK_EQ(mapped.correlation_id, "evt-sync-4");

  const RuntimeDiagnosticsSnapshot snapshot = bridge.Snapshot();
  CHECK_EQ(snapshot.last_sequence, 4u);
  CHECK_EQ(snapshot.statuses.size(), static_cast<size_t>(4));
  CHECK_EQ(snapshot.statuses[0].status_key, "runtime.commit");
  CHECK_EQ(snapshot.statuses[1].status_key, "runtime.dirty");
  CHECK_EQ(snapshot.statuses[2].status_key, "runtime.health");
  CHECK_EQ(snapshot.statuses[3].status_key, "runtime.sync");
  return true;
}

bool TestBurstEventsAndDroppedEventWarnings() {
  RuntimeDiagnosticsBridge bridge;
  RuntimeDiagnosticStatus mapped;

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(10, "evt-dirty-10", "dirty_state_changed",
                R"({"payload":{"dirty":true}})"),
      &mapped));
  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(11, "evt-dirty-11", "dirty_state_changed",
                R"({"payload":{"dirty":false}})"),
      &mapped));
  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(12, "evt-sync-12", "sync_state_changed",
                R"({"payload":{"state":"error"}})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.sync");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_SYNC_STATE_ERROR");
  CHECK_EQ(mapped.severity, DiagnosticSeverity::kError);

  RuntimeEventDispatchResult dispatch_result;
  dispatch_result.status = RuntimeSdkStatus::kOk;
  dispatch_result.from_sequence = 9;
  dispatch_result.next_sequence = 13;
  dispatch_result.dropped_events = 7;
  dispatch_result.dropped_events_delta = 3;
  dispatch_result.callbacks_invoked = 12;
  dispatch_result.callback_failures = 1;
  const auto dropped_status = bridge.ObserveDispatch(dispatch_result);
  CHECK_TRUE(dropped_status.has_value());
  CHECK_EQ(dropped_status->status_key, "runtime.events");
  CHECK_EQ(dropped_status->status_code, "CFGX_ROS2_EVENT_BACKPRESSURE_DROPPED");
  CHECK_EQ(dropped_status->severity, DiagnosticSeverity::kWarn);
  CHECK_EQ(dropped_status->correlation_id, "dispatch:9-13");

  dispatch_result.dropped_events_delta = 0;
  CHECK_TRUE(!bridge.ObserveDispatch(dispatch_result).has_value());

  const RuntimeDiagnosticsSnapshot snapshot = bridge.Snapshot();
  CHECK_EQ(snapshot.last_sequence, 13u);
  CHECK_EQ(snapshot.statuses.size(), static_cast<size_t>(3));

  const RuntimeDiagnosticStatus* dirty = FindStatusByKey(snapshot, "runtime.dirty");
  CHECK_TRUE(dirty != nullptr);
  CHECK_EQ(dirty->status_code, "CFGX_ROS2_DIRTY_CLEARED");

  const RuntimeDiagnosticStatus* events =
      FindStatusByKey(snapshot, "runtime.events");
  CHECK_TRUE(events != nullptr);
  CHECK_EQ(events->status_code, "CFGX_ROS2_EVENT_BACKPRESSURE_DROPPED");
  return true;
}

bool TestReconnectCycleResetsStateAndRejectsStaleEvents() {
  RuntimeDiagnosticsBridge bridge;
  RuntimeDiagnosticStatus mapped;

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(50, "evt-open-50", "runtime_opened",
                R"({"event_kind":"runtime_opened"})"),
      &mapped));
  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(51, "evt-sync-51", "sync_state_changed",
                R"({"payload":{"state":"idle"}})"),
      &mapped));

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(1, "evt-open-reconnect", "runtime_opened",
                R"({"event_kind":"runtime_opened"})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.health");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_HEALTH_RECONNECTED");
  CHECK_EQ(mapped.severity, DiagnosticSeverity::kWarn);
  CHECK_EQ(mapped.correlation_id, "evt-open-reconnect");

  CHECK_TRUE(!bridge.IngestEvent(
      MakeEvent(1, "evt-stale-1", "commit_applied",
                R"({"event_kind":"commit_applied"})"),
      &mapped));
  CHECK_TRUE(!bridge.IngestEvent(
      MakeEvent(0, "evt-stale-0", "sync_state_changed",
                R"({"payload":{"state":"offline"}})"),
      &mapped));

  CHECK_TRUE(bridge.IngestEvent(
      MakeEvent(2, "evt-sync-2", "sync_state_changed",
                R"({"payload":{"state":"offline"}})"),
      &mapped));
  CHECK_EQ(mapped.status_key, "runtime.sync");
  CHECK_EQ(mapped.status_code, "CFGX_ROS2_SYNC_STATE_OFFLINE");
  CHECK_EQ(mapped.correlation_id, "evt-sync-2");

  const RuntimeDiagnosticsSnapshot snapshot = bridge.Snapshot();
  CHECK_EQ(snapshot.last_sequence, 2u);
  CHECK_EQ(snapshot.statuses.size(), static_cast<size_t>(2));
  CHECK_TRUE(FindStatusByKey(snapshot, "runtime.health") != nullptr);
  CHECK_TRUE(FindStatusByKey(snapshot, "runtime.sync") != nullptr);
  CHECK_TRUE(FindStatusByKey(snapshot, "runtime.commit") == nullptr);
  return true;
}

}  // namespace

int main() {
  bool ok = true;
  ok = TestEventMappingAndCorrelationIds() && ok;
  ok = TestBurstEventsAndDroppedEventWarnings() && ok;
  ok = TestReconnectCycleResetsStateAndRejectsStaleEvents() && ok;

  if (!ok) {
    std::cerr << "runtime_diagnostics_bridge_test failed" << std::endl;
    return 1;
  }
  std::cout << "runtime_diagnostics_bridge_test passed" << std::endl;
  return 0;
}
