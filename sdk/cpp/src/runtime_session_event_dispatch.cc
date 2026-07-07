// SPDX-License-Identifier: BUSL-1.1

#include "runtime_session_internal.h"

#include <algorithm>
#include <exception>
#include <utility>

namespace configflux::sdk {

RuntimeEventSubscription RuntimeSession::SubscribeEventChannel(
    RuntimeEventChannel channel, RuntimeEventCallback callback) {
  if (!callback) {
    return RuntimeEventSubscription{};
  }

  std::lock_guard<std::mutex> lock(callbacks_mutex_);
  uint64_t candidate = next_subscription_id_;
  while (candidate == 0 || event_subscriptions_.contains(candidate)) {
    ++candidate;
  }
  next_subscription_id_ = candidate + 1;
  if (next_subscription_id_ == 0) {
    next_subscription_id_ = 1;
  }

  event_subscriptions_.emplace(
      candidate, EventSubscriptionRecord{channel, std::move(callback)});
  RuntimeEventSubscription subscription;
  subscription.id = candidate;
  subscription.channel = channel;
  return subscription;
}

RuntimeSdkStatus RuntimeSession::UnsubscribeEventChannel(
    RuntimeEventSubscription subscription) {
  if (!subscription.valid()) {
    return RuntimeSdkStatus::kInvalidArgument;
  }

  std::lock_guard<std::mutex> lock(callbacks_mutex_);
  auto it = event_subscriptions_.find(subscription.id);
  if (it == event_subscriptions_.end() || it->second.channel != subscription.channel) {
    return RuntimeSdkStatus::kInvalidArgument;
  }
  event_subscriptions_.erase(it);
  return RuntimeSdkStatus::kOk;
}

RuntimeEventDispatchResult RuntimeSession::PollAndDispatchEvents(
    uint64_t from_sequence, uint32_t max_events) {
  if (max_events == 0) {
    RuntimeEventDispatchResult result = BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidArgument, "max_events must be greater than 0");
    result.from_sequence = from_sequence;
    return result;
  }

  RuntimeCallResult subscribe_result =
      SubscribeEvents(internal::BuildSubscribeEventsRequestJson(from_sequence, max_events));
  if (!subscribe_result.ok()) {
    RuntimeEventDispatchResult result = BuildLocalDispatchError(
        subscribe_result.status,
        subscribe_result.message.empty() ? "subscribe_events call failed"
                                         : subscribe_result.message);
    result.from_sequence = from_sequence;
    return result;
  }
  return DispatchEventsFromSubscribeResponse(subscribe_result.response_json);
}

RuntimeEventDispatchResult RuntimeSession::DispatchEventsFromSubscribeResponse(
    std::string_view subscribe_response_json) {
  if (HasEmbeddedNul(subscribe_response_json)) {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidArgument,
        "subscribe_response_json must not contain embedded NUL bytes");
  }

  RuntimeEventDispatchResult result;

  std::string status;
  if (!internal::ExtractJsonStringField(subscribe_response_json, "status", &status)) {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidJson,
        "subscribe_events response missing status field");
  }
  if (status != "ok") {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidArgument,
        "subscribe_events response returned non-ok status");
  }

  if (!internal::ExtractJsonUnsignedField(subscribe_response_json, "from_sequence",
                                          &result.from_sequence)) {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidJson,
        "subscribe_events response missing from_sequence");
  }
  if (!internal::ExtractJsonUnsignedField(subscribe_response_json, "next_sequence",
                                          &result.next_sequence)) {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidJson,
        "subscribe_events response missing next_sequence");
  }
  if (!internal::ExtractJsonUnsignedField(subscribe_response_json, "dropped_events",
                                          &result.dropped_events)) {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidJson,
        "subscribe_events response missing dropped_events");
  }

  std::string_view events_array_body;
  if (!internal::ExtractJsonArrayField(subscribe_response_json, "events",
                                       &events_array_body)) {
    return BuildLocalDispatchError(
        RuntimeSdkStatus::kInvalidJson,
        "subscribe_events response missing events array");
  }

  {
    std::lock_guard<std::mutex> lock(callbacks_mutex_);
    if (result.dropped_events >= last_observed_dropped_events_) {
      result.dropped_events_delta =
          result.dropped_events - last_observed_dropped_events_;
    } else {
      result.dropped_events_delta = result.dropped_events;
    }
    last_observed_dropped_events_ = result.dropped_events;
  }

  std::vector<std::string> event_json_objects;
  if (!internal::SplitTopLevelJsonObjects(events_array_body, &event_json_objects)) {
    return BuildLocalDispatchError(RuntimeSdkStatus::kInvalidJson,
                                   "subscribe_events response events array is malformed");
  }

  std::vector<uint64_t> auto_unsubscribe_ids;
  for (size_t event_index = 0; event_index < event_json_objects.size(); ++event_index) {
    const std::string& event_json = event_json_objects[event_index];
    RuntimeEventNotification notification;
    notification.event_json = event_json;
    if (!internal::ExtractJsonUnsignedField(event_json, "sequence", &notification.sequence) ||
        !internal::ExtractJsonStringField(event_json, "event_id", &notification.event_id) ||
        !internal::ExtractJsonStringField(event_json, "event_kind",
                                          &notification.event_kind) ||
        !internal::ExtractJsonStringField(event_json, "scope", &notification.scope)) {
      return BuildLocalDispatchError(
          RuntimeSdkStatus::kInvalidJson,
          "subscribe_events response contains malformed event envelope");
    }

    std::optional<RuntimeEventChannel> channel =
        internal::EventChannelForKind(notification.event_kind);
    if (!channel.has_value()) {
      continue;
    }

    std::vector<std::pair<uint64_t, RuntimeEventCallback>> callbacks;
    {
      std::lock_guard<std::mutex> lock(callbacks_mutex_);
      callbacks.reserve(event_subscriptions_.size());
      for (const auto& [id, record] : event_subscriptions_) {
        if (record.channel == channel.value() &&
            static_cast<bool>(record.callback)) {
          callbacks.emplace_back(id, record.callback);
        }
      }
    }
    std::sort(callbacks.begin(), callbacks.end(),
              [](const auto& lhs, const auto& rhs) { return lhs.first < rhs.first; });

    for (const auto& [id, callback] : callbacks) {
      ++result.callbacks_invoked;
      RuntimeCallbackAction action = RuntimeCallbackAction::kKeepSubscription;
      try {
        action = callback(notification);
      } catch (const std::exception& ex) {
        ++result.callback_failures;
        action = RuntimeCallbackAction::kUnsubscribe;
        if (result.message.empty()) {
          result.message = "event callback exception: ";
          result.message.append(ex.what());
        }
      } catch (...) {
        ++result.callback_failures;
        action = RuntimeCallbackAction::kUnsubscribe;
        if (result.message.empty()) {
          result.message = "event callback exception: unknown";
        }
      }
      if (action == RuntimeCallbackAction::kUnsubscribe) {
        auto_unsubscribe_ids.push_back(id);
      }
    }
  }

  if (!auto_unsubscribe_ids.empty()) {
    std::sort(auto_unsubscribe_ids.begin(), auto_unsubscribe_ids.end());
    auto_unsubscribe_ids.erase(
        std::unique(auto_unsubscribe_ids.begin(), auto_unsubscribe_ids.end()),
        auto_unsubscribe_ids.end());
    std::lock_guard<std::mutex> lock(callbacks_mutex_);
    for (uint64_t id : auto_unsubscribe_ids) {
      event_subscriptions_.erase(id);
    }
  }

  return result;
}

RuntimeEventDispatchResult RuntimeSession::BuildLocalDispatchError(
    RuntimeSdkStatus status, std::string message) const {
  RuntimeEventDispatchResult result;
  result.status = status;
  result.message = std::move(message);
  return result;
}

}  // namespace configflux::sdk
