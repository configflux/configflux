// SPDX-License-Identifier: BUSL-1.1

#include "configflux/sdk/runtime_session.h"

#include <stdexcept>
#include <string>
#include <vector>

#include "support/fake_runtime_c_abi.h"
#include "support/test_checks.h"
#include "support/test_harness.h"

namespace {

using configflux::sdk::RuntimeCallbackAction;
using configflux::sdk::RuntimeEventChannel;
using configflux::sdk::RuntimeEventNotification;
using configflux::sdk::RuntimeSdkStatus;
using configflux::sdk::RuntimeSession;
using configflux::sdk::test_support::FakeApi;
using configflux::sdk::test_support::ResetFakeState;

bool TestEventCallbackDispatchOrderingAndBackpressure() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  int parameter_calls = 0;
  int dirty_calls = 0;
  int reset_calls = 0;
  int commit_calls = 0;
  int sync_calls = 0;
  std::vector<uint64_t> callback_sequence;

  auto parameter_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kParameterChanged,
      [&parameter_calls, &callback_sequence](
          const RuntimeEventNotification& event) {
        ++parameter_calls;
        callback_sequence.push_back(event.sequence);
        return RuntimeCallbackAction::kUnsubscribe;
      });
  CHECK_TRUE(parameter_subscription.valid());

  auto dirty_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kDirtyStateChanged,
      [&dirty_calls, &callback_sequence](const RuntimeEventNotification& event) {
        ++dirty_calls;
        callback_sequence.push_back(event.sequence);
        return RuntimeCallbackAction::kKeepSubscription;
      });
  CHECK_TRUE(dirty_subscription.valid());

  auto reset_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kAutoResetOccurred,
      [&reset_calls, &callback_sequence](const RuntimeEventNotification& event) {
        ++reset_calls;
        callback_sequence.push_back(event.sequence);
        return RuntimeCallbackAction::kKeepSubscription;
      });
  CHECK_TRUE(reset_subscription.valid());

  auto commit_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kCommitApplied,
      [&commit_calls, &callback_sequence](const RuntimeEventNotification& event) {
        ++commit_calls;
        callback_sequence.push_back(event.sequence);
        return RuntimeCallbackAction::kKeepSubscription;
      });
  CHECK_TRUE(commit_subscription.valid());

  auto sync_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kSyncEvent,
      [&sync_calls, &callback_sequence](const RuntimeEventNotification& event) {
        ++sync_calls;
        callback_sequence.push_back(event.sequence);
        return RuntimeCallbackAction::kKeepSubscription;
      });
  CHECK_TRUE(sync_subscription.valid());

  auto first_dispatch = session.PollAndDispatchEvents(0, 64);
  CHECK_TRUE(first_dispatch.ok());
  CHECK_EQ(first_dispatch.from_sequence, 0u);
  CHECK_EQ(first_dispatch.next_sequence, 7u);
  CHECK_EQ(first_dispatch.dropped_events, 2u);
  CHECK_EQ(first_dispatch.dropped_events_delta, 2u);
  CHECK_EQ(first_dispatch.callbacks_invoked, 5u);
  CHECK_EQ(first_dispatch.callback_failures, 0u);
  CHECK_TRUE(first_dispatch.message.empty());

  CHECK_EQ(parameter_calls, 1);
  CHECK_EQ(dirty_calls, 1);
  CHECK_EQ(reset_calls, 1);
  CHECK_EQ(commit_calls, 1);
  CHECK_EQ(sync_calls, 1);
  CHECK_EQ(callback_sequence.size(), static_cast<size_t>(5));
  CHECK_EQ(callback_sequence[0], 2u);
  CHECK_EQ(callback_sequence[1], 3u);
  CHECK_EQ(callback_sequence[2], 4u);
  CHECK_EQ(callback_sequence[3], 5u);
  CHECK_EQ(callback_sequence[4], 6u);

  auto second_dispatch = session.PollAndDispatchEvents(6, 64);
  CHECK_TRUE(second_dispatch.ok());
  CHECK_EQ(second_dispatch.from_sequence, 6u);
  CHECK_EQ(second_dispatch.next_sequence, 9u);
  CHECK_EQ(second_dispatch.dropped_events, 5u);
  CHECK_EQ(second_dispatch.dropped_events_delta, 3u);
  CHECK_EQ(second_dispatch.callbacks_invoked, 1u);
  CHECK_EQ(second_dispatch.callback_failures, 0u);
  CHECK_EQ(parameter_calls, 1);
  CHECK_EQ(sync_calls, 2);
  CHECK_EQ(callback_sequence.size(), static_cast<size_t>(6));
  CHECK_EQ(callback_sequence[5], 8u);

  CHECK_EQ(session.UnsubscribeEventChannel(dirty_subscription), RuntimeSdkStatus::kOk);
  CHECK_EQ(session.UnsubscribeEventChannel(reset_subscription), RuntimeSdkStatus::kOk);
  CHECK_EQ(session.UnsubscribeEventChannel(commit_subscription), RuntimeSdkStatus::kOk);
  CHECK_EQ(session.UnsubscribeEventChannel(sync_subscription), RuntimeSdkStatus::kOk);
  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  return true;
}

bool TestEventCallbackFailureHandlingAndUnsubscribe() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  int stable_sync_calls = 0;
  auto throwing_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kSyncEvent,
      [](const RuntimeEventNotification&) -> RuntimeCallbackAction {
        throw std::runtime_error("sync callback failure");
      });
  CHECK_TRUE(throwing_subscription.valid());

  auto stable_subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kSyncEvent,
      [&stable_sync_calls](const RuntimeEventNotification&) {
        ++stable_sync_calls;
        return RuntimeCallbackAction::kKeepSubscription;
      });
  CHECK_TRUE(stable_subscription.valid());

  auto first_dispatch = session.PollAndDispatchEvents(0, 64);
  CHECK_TRUE(first_dispatch.ok());
  CHECK_EQ(first_dispatch.callbacks_invoked, 2u);
  CHECK_EQ(first_dispatch.callback_failures, 1u);
  CHECK_TRUE(!first_dispatch.message.empty());
  CHECK_EQ(stable_sync_calls, 1);

  auto second_dispatch = session.PollAndDispatchEvents(6, 64);
  CHECK_TRUE(second_dispatch.ok());
  CHECK_EQ(second_dispatch.callbacks_invoked, 1u);
  CHECK_EQ(second_dispatch.callback_failures, 0u);
  CHECK_EQ(stable_sync_calls, 2);

  CHECK_EQ(session.UnsubscribeEventChannel(stable_subscription),
           RuntimeSdkStatus::kOk);
  CHECK_EQ(session.UnsubscribeEventChannel(stable_subscription),
           RuntimeSdkStatus::kInvalidArgument);

  auto third_dispatch = session.PollAndDispatchEvents(8, 64);
  CHECK_TRUE(third_dispatch.ok());
  CHECK_EQ(third_dispatch.callbacks_invoked, 0u);
  CHECK_EQ(third_dispatch.callback_failures, 0u);
  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  return true;
}

bool TestCloseClearsEventSubscriptions() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  int parameter_calls = 0;
  auto subscription = session.SubscribeEventChannel(
      RuntimeEventChannel::kParameterChanged,
      [&parameter_calls](const RuntimeEventNotification&) {
        ++parameter_calls;
        return RuntimeCallbackAction::kKeepSubscription;
      });
  CHECK_TRUE(subscription.valid());

  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  CHECK_EQ(session.UnsubscribeEventChannel(subscription),
           RuntimeSdkStatus::kInvalidArgument);

  auto reopen_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(reopen_result.ok());
  auto dispatch = session.PollAndDispatchEvents(0, 64);
  CHECK_TRUE(dispatch.ok());
  CHECK_EQ(dispatch.callbacks_invoked, 0u);
  CHECK_EQ(parameter_calls, 0);
  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  return true;
}

}  // namespace

namespace configflux::sdk::test_support {

void RegisterRuntimeSessionEventTests(std::vector<TestCase>* tests) {
  tests->push_back({"event_callback_dispatch_ordering_and_backpressure",
                    &TestEventCallbackDispatchOrderingAndBackpressure});
  tests->push_back({"event_callback_failure_handling_and_unsubscribe",
                    &TestEventCallbackFailureHandlingAndUnsubscribe});
  tests->push_back({"close_clears_event_subscriptions",
                    &TestCloseClearsEventSubscriptions});
}

}  // namespace configflux::sdk::test_support
