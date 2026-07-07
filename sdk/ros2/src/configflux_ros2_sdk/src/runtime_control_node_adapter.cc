// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_control_node_adapter.h"

#include <cstdint>
#include <thread>
#include <utility>

namespace configflux::ros2 {
namespace {

constexpr uint32_t kDefaultMaxAuditEvents = 256;
constexpr uint32_t kDefaultMaxEvents = 256;

RuntimeControlResult BuildInvalidArgumentResult(std::string reason) {
  RuntimeControlResult result;
  result.status = sdk::RuntimeSdkStatus::kInvalidArgument;
  result.outcome = RuntimeControlOutcome::kTransportError;
  result.successful = false;
  result.reason = std::move(reason);
  return result;
}

}  // namespace

RuntimeControlNodeAdapter::RuntimeControlNodeAdapter(
    rclcpp::Node* node, RuntimeControlAdapter* runtime_adapter)
    : node_(node), runtime_adapter_(runtime_adapter) {}

bool RuntimeControlNodeAdapter::StartServers(std::string service_prefix) {
  if (node_ == nullptr || runtime_adapter_ == nullptr) {
    return false;
  }

  service_prefix_ =
      service_prefix.empty() ? "configflux/runtime_control" : std::move(service_prefix);

  commit_service_ = node_->create_service<CommitConfigurationSrv>(
      ComposeName(service_prefix_, "commit_configuration"),
      [this](const std::shared_ptr<CommitConfigurationSrv::Request> request,
             std::shared_ptr<CommitConfigurationSrv::Response> response) {
        HandleCommitConfiguration(request, response);
      });

  rollback_service_ = node_->create_service<RollbackDirtySrv>(
      ComposeName(service_prefix_, "rollback_dirty"),
      [this](const std::shared_ptr<RollbackDirtySrv::Request> request,
             std::shared_ptr<RollbackDirtySrv::Response> response) {
        HandleRollbackDirty(request, response);
      });

  check_updates_service_ = node_->create_service<CheckForUpdatesSrv>(
      ComposeName(service_prefix_, "check_for_updates"),
      [this](const std::shared_ptr<CheckForUpdatesSrv::Request> request,
             std::shared_ptr<CheckForUpdatesSrv::Response> response) {
        HandleCheckForUpdates(request, response);
      });

  get_sync_status_service_ = node_->create_service<GetSyncStatusSrv>(
      ComposeName(service_prefix_, "get_sync_status"),
      [this](const std::shared_ptr<GetSyncStatusSrv::Request> request,
             std::shared_ptr<GetSyncStatusSrv::Response> response) {
        HandleGetSyncStatus(request, response);
      });

  export_pending_sync_bundle_service_ =
      node_->create_service<ExportPendingSyncBundleSrv>(
          ComposeName(service_prefix_, "export_pending_sync_bundle"),
          [this](
              const std::shared_ptr<ExportPendingSyncBundleSrv::Request> request,
              std::shared_ptr<ExportPendingSyncBundleSrv::Response> response) {
            HandleExportPendingSyncBundle(request, response);
          });

  push_audit_events_service_ = node_->create_service<PushAuditEventsSrv>(
      ComposeName(service_prefix_, "push_audit_events"),
      [this](const std::shared_ptr<PushAuditEventsSrv::Request> request,
             std::shared_ptr<PushAuditEventsSrv::Response> response) {
        HandlePushAuditEvents(request, response);
      });

  pull_updates_action_server_ = rclcpp_action::create_server<PullUpdatesAction>(
      node_, ComposeName(service_prefix_, "pull_updates"),
      [this](const rclcpp_action::GoalUUID& goal_id,
             std::shared_ptr<const PullUpdatesAction::Goal> goal) {
        return HandlePullGoal(goal_id, std::move(goal));
      },
      [this](const std::shared_ptr<PullGoalHandle> goal_handle) {
        return HandlePullCancel(goal_handle);
      },
      [this](const std::shared_ptr<PullGoalHandle> goal_handle) {
        HandlePullAccepted(goal_handle);
      });

  return commit_service_ != nullptr && rollback_service_ != nullptr &&
         check_updates_service_ != nullptr && get_sync_status_service_ != nullptr &&
         export_pending_sync_bundle_service_ != nullptr &&
         push_audit_events_service_ != nullptr &&
         pull_updates_action_server_ != nullptr;
}

void RuntimeControlNodeAdapter::StopServers() {
  pull_updates_action_server_.reset();
  push_audit_events_service_.reset();
  export_pending_sync_bundle_service_.reset();
  get_sync_status_service_.reset();
  check_updates_service_.reset();
  rollback_service_.reset();
  commit_service_.reset();
}

void RuntimeControlNodeAdapter::SetLifecycleActive(bool active) {
  lifecycle_active_ = active;
}

bool RuntimeControlNodeAdapter::lifecycle_active() const {
  return lifecycle_active_;
}

void RuntimeControlNodeAdapter::SetLifecyclePolicy(
    RuntimeControlLifecyclePolicy policy) {
  lifecycle_policy_ = policy;
}

const RuntimeControlLifecyclePolicy&
RuntimeControlNodeAdapter::lifecycle_policy() const {
  return lifecycle_policy_;
}

void RuntimeControlNodeAdapter::HandleCommitConfiguration(
    const std::shared_ptr<CommitConfigurationSrv::Request> request,
    std::shared_ptr<CommitConfigurationSrv::Response> response) {
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &response->result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("commit_configuration")) {
    PopulateResult(
        BuildLifecycleInactiveResult("commit_configuration"),
        &response->result);
    return;
  }

  CommitConfigurationServiceRequest mapped_request;
  mapped_request.runtime_snapshot_json = request->runtime_snapshot_json;
  mapped_request.context.actor = request->actor;
  if (!request->reason.empty()) {
    mapped_request.context.reason = request->reason;
  }
  if (!request->expected_base_configuration_id.empty()) {
    mapped_request.expected_base_configuration_id =
        request->expected_base_configuration_id;
  }
  mapped_request.changed_ros_parameter_hints = request->changed_ros_parameter_hints;

  PopulateResult(runtime_adapter_->CommitConfiguration(mapped_request),
                 &response->result);
}

void RuntimeControlNodeAdapter::HandleRollbackDirty(
    const std::shared_ptr<RollbackDirtySrv::Request> request,
    std::shared_ptr<RollbackDirtySrv::Response> response) {
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &response->result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("rollback_dirty")) {
    PopulateResult(BuildLifecycleInactiveResult("rollback_dirty"),
                   &response->result);
    return;
  }

  RollbackDirtyServiceRequest mapped_request;
  mapped_request.runtime_snapshot_json = request->runtime_snapshot_json;
  mapped_request.context.actor = request->actor;
  if (!request->reason.empty()) {
    mapped_request.context.reason = request->reason;
  }
  mapped_request.rollback_ros_parameter_names =
      request->rollback_ros_parameter_names;

  PopulateResult(runtime_adapter_->RollbackDirty(mapped_request),
                 &response->result);
}

void RuntimeControlNodeAdapter::HandleCheckForUpdates(
    const std::shared_ptr<CheckForUpdatesSrv::Request> request,
    std::shared_ptr<CheckForUpdatesSrv::Response> response) {
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &response->result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("check_for_updates")) {
    PopulateResult(BuildLifecycleInactiveResult("check_for_updates"),
                   &response->result);
    return;
  }

  CheckForUpdatesServiceRequest mapped_request;
  mapped_request.runtime_snapshot_json = request->runtime_snapshot_json;
  mapped_request.backend_connected = request->backend_connected;
  if (!request->pending_update_summary.empty()) {
    mapped_request.pending_update_summary = request->pending_update_summary;
  }

  PopulateResult(runtime_adapter_->CheckForUpdates(mapped_request),
                 &response->result);
}

void RuntimeControlNodeAdapter::HandleGetSyncStatus(
    const std::shared_ptr<GetSyncStatusSrv::Request> request,
    std::shared_ptr<GetSyncStatusSrv::Response> response) {
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &response->result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("get_sync_status")) {
    PopulateResult(BuildLifecycleInactiveResult("get_sync_status"),
                   &response->result);
    return;
  }

  GetSyncStatusServiceRequest mapped_request;
  mapped_request.runtime_snapshot_json = request->runtime_snapshot_json;

  PopulateResult(runtime_adapter_->GetSyncStatus(mapped_request),
                 &response->result);
}

void RuntimeControlNodeAdapter::HandleExportPendingSyncBundle(
    const std::shared_ptr<ExportPendingSyncBundleSrv::Request> request,
    std::shared_ptr<ExportPendingSyncBundleSrv::Response> response) {
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &response->result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("export_pending_sync_bundle")) {
    PopulateResult(
        BuildLifecycleInactiveResult("export_pending_sync_bundle"),
        &response->result);
    return;
  }

  ExportPendingSyncBundleServiceRequest mapped_request;
  mapped_request.runtime_snapshot_json = request->runtime_snapshot_json;
  mapped_request.max_audit_events =
      request->max_audit_events == 0 ? kDefaultMaxAuditEvents
                                     : request->max_audit_events;

  PopulateResult(runtime_adapter_->ExportPendingSyncBundle(mapped_request),
                 &response->result);
}

void RuntimeControlNodeAdapter::HandlePushAuditEvents(
    const std::shared_ptr<PushAuditEventsSrv::Request> request,
    std::shared_ptr<PushAuditEventsSrv::Response> response) {
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &response->result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("push_audit_events")) {
    PopulateResult(BuildLifecycleInactiveResult("push_audit_events"),
                   &response->result);
    return;
  }

  PushAuditEventsServiceRequest mapped_request;
  mapped_request.runtime_snapshot_json = request->runtime_snapshot_json;
  mapped_request.backend_connected = request->backend_connected;
  mapped_request.max_events =
      request->max_events == 0 ? kDefaultMaxEvents : request->max_events;

  PopulateResult(runtime_adapter_->PushAuditEvents(mapped_request),
                 &response->result);
}

rclcpp_action::GoalResponse RuntimeControlNodeAdapter::HandlePullGoal(
    const rclcpp_action::GoalUUID& goal_id,
    std::shared_ptr<const PullUpdatesAction::Goal> goal) {
  (void)goal_id;
  if (runtime_adapter_ == nullptr || goal == nullptr) {
    return rclcpp_action::GoalResponse::REJECT;
  }
  if (!IsOperationAllowedWhileInactive("pull_updates")) {
    return rclcpp_action::GoalResponse::REJECT;
  }
  return rclcpp_action::GoalResponse::ACCEPT_AND_EXECUTE;
}

rclcpp_action::CancelResponse RuntimeControlNodeAdapter::HandlePullCancel(
    const std::shared_ptr<PullGoalHandle> goal_handle) {
  (void)goal_handle;
  return rclcpp_action::CancelResponse::ACCEPT;
}

void RuntimeControlNodeAdapter::HandlePullAccepted(
    const std::shared_ptr<PullGoalHandle> goal_handle) {
  std::thread([this, goal_handle]() { ExecutePullGoal(goal_handle); }).detach();
}

void RuntimeControlNodeAdapter::ExecutePullGoal(
    const std::shared_ptr<PullGoalHandle> goal_handle) {
  auto action_result = std::make_shared<PullUpdatesAction::Result>();
  if (runtime_adapter_ == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("runtime adapter is null"),
                   &action_result->result);
    goal_handle->abort(action_result);
    return;
  }
  if (!IsOperationAllowedWhileInactive("pull_updates")) {
    PopulateResult(BuildLifecycleInactiveResult("pull_updates"),
                   &action_result->result);
    goal_handle->abort(action_result);
    return;
  }

  const std::shared_ptr<const PullUpdatesAction::Goal> goal = goal_handle->get_goal();
  if (goal == nullptr) {
    PopulateResult(BuildInvalidArgumentResult("pull goal is null"),
                   &action_result->result);
    goal_handle->abort(action_result);
    return;
  }

  PullUpdatesActionRequest mapped_request;
  mapped_request.runtime_snapshot_json = goal->runtime_snapshot_json;
  mapped_request.context.actor = goal->actor;
  if (!goal->reason.empty()) {
    mapped_request.context.reason = goal->reason;
  }
  mapped_request.backend_connected = goal->backend_connected;
  if (!ToPullUpdatesSource(goal->source, &mapped_request.source)) {
    PopulateResult(BuildInvalidArgumentResult("pull_updates has unknown source"),
                   &action_result->result);
    goal_handle->abort(action_result);
    return;
  }
  mapped_request.full_snapshot = goal->full_snapshot;
  if (!goal->base_configuration_id.empty()) {
    mapped_request.base_configuration_id = goal->base_configuration_id;
  }
  if (!goal->pending_update_summary.empty()) {
    mapped_request.pending_update_summary = goal->pending_update_summary;
  }
  if (!goal->target_configuration_id.empty()) {
    mapped_request.target_configuration_id = goal->target_configuration_id;
  }

  mapped_request.writes.reserve(goal->writes.size());
  for (const auto& write_msg : goal->writes) {
    PullUpdateWrite write;
    std::string reason;
    if (!PopulatePullUpdateWrite(write_msg, &write, &reason)) {
      PopulateResult(BuildInvalidArgumentResult(reason), &action_result->result);
      goal_handle->abort(action_result);
      return;
    }
    mapped_request.writes.push_back(std::move(write));
  }

  auto feedback = std::make_shared<PullUpdatesAction::Feedback>();
  feedback->stage = "runtime_call_started";
  goal_handle->publish_feedback(feedback);

  const RuntimeControlResult result = runtime_adapter_->PullUpdatesAction(mapped_request);
  PopulateResult(result, &action_result->result);

  if (goal_handle->is_canceling()) {
    feedback->stage = "runtime_call_cancelled";
    goal_handle->publish_feedback(feedback);
    goal_handle->canceled(action_result);
    return;
  }

  feedback->stage = result.successful ? "runtime_call_completed"
                                      : "runtime_call_failed";
  goal_handle->publish_feedback(feedback);

  if (result.successful) {
    goal_handle->succeed(action_result);
    return;
  }
  goal_handle->abort(action_result);
}

bool RuntimeControlNodeAdapter::ToPullUpdatesSource(
    uint8_t source, PullUpdatesSource* out_source) {
  if (out_source == nullptr) {
    return false;
  }
  if (source == PullUpdatesAction::Goal::SOURCE_BACKEND) {
    *out_source = PullUpdatesSource::kBackend;
    return true;
  }
  if (source == PullUpdatesAction::Goal::SOURCE_DIRECT_PUSH) {
    *out_source = PullUpdatesSource::kDirectPush;
    return true;
  }
  return false;
}

bool RuntimeControlNodeAdapter::PopulatePullUpdateWrite(
    const configflux_ros2_sdk::msg::PullUpdateWrite& write_msg,
    PullUpdateWrite* out_write, std::string* out_reason) {
  if (out_write == nullptr || out_reason == nullptr) {
    return false;
  }

  if (write_msg.ros_parameter_name.empty()) {
    *out_reason = "pull write requires ros_parameter_name";
    return false;
  }

  out_write->ros_parameter_name = write_msg.ros_parameter_name;
  switch (write_msg.value_type) {
    case configflux_ros2_sdk::msg::PullUpdateWrite::VALUE_TYPE_BOOL:
      out_write->value = write_msg.bool_value;
      break;
    case configflux_ros2_sdk::msg::PullUpdateWrite::VALUE_TYPE_INT64:
      out_write->value = write_msg.int_value;
      break;
    case configflux_ros2_sdk::msg::PullUpdateWrite::VALUE_TYPE_DOUBLE:
      out_write->value = write_msg.double_value;
      break;
    case configflux_ros2_sdk::msg::PullUpdateWrite::VALUE_TYPE_STRING:
      out_write->value = write_msg.string_value;
      break;
    default:
      *out_reason = "pull write has unknown value_type";
      return false;
  }

  if (!write_msg.before_leaf_hash.empty()) {
    out_write->before_leaf_hash = write_msg.before_leaf_hash;
  }
  if (!write_msg.after_leaf_hash.empty()) {
    out_write->after_leaf_hash = write_msg.after_leaf_hash;
  }
  return true;
}

void RuntimeControlNodeAdapter::PopulateResult(
    const RuntimeControlResult& result, RuntimeControlResultMsg* out_result) {
  if (out_result == nullptr) {
    return;
  }
  out_result->sdk_status = static_cast<uint32_t>(result.status);
  out_result->outcome = static_cast<uint8_t>(result.outcome);
  out_result->successful = result.successful;
  out_result->runtime_status = result.runtime_status;
  out_result->runtime_code = result.runtime_code;
  out_result->response_json = result.response_json;
  out_result->reason = result.reason;
}

std::string RuntimeControlNodeAdapter::ComposeName(const std::string& prefix,
                                                   const std::string& suffix) {
  if (prefix.empty()) {
    return suffix;
  }
  if (!suffix.empty() && suffix.front() == '/') {
    return prefix + suffix;
  }
  if (prefix.back() == '/') {
    return prefix + suffix;
  }
  return prefix + "/" + suffix;
}

bool RuntimeControlNodeAdapter::IsOperationAllowedWhileInactive(
    std::string_view operation_name) const {
  if (lifecycle_active_) {
    return true;
  }
  if (operation_name == "commit_configuration") {
    return lifecycle_policy_.allow_commit_configuration_while_inactive;
  }
  if (operation_name == "rollback_dirty") {
    return lifecycle_policy_.allow_rollback_dirty_while_inactive;
  }
  if (operation_name == "check_for_updates") {
    return lifecycle_policy_.allow_check_for_updates_while_inactive;
  }
  if (operation_name == "get_sync_status") {
    return lifecycle_policy_.allow_get_sync_status_while_inactive;
  }
  if (operation_name == "export_pending_sync_bundle") {
    return lifecycle_policy_.allow_export_pending_sync_bundle_while_inactive;
  }
  if (operation_name == "push_audit_events") {
    return lifecycle_policy_.allow_push_audit_events_while_inactive;
  }
  if (operation_name == "pull_updates") {
    return lifecycle_policy_.allow_pull_updates_while_inactive;
  }
  return false;
}

RuntimeControlResult RuntimeControlNodeAdapter::BuildLifecycleInactiveResult(
    std::string operation_name) {
  RuntimeControlResult result;
  result.status = sdk::RuntimeSdkStatus::kInvalidArgument;
  result.outcome = RuntimeControlOutcome::kTransportError;
  result.successful = false;
  result.reason =
      "operation '" + std::move(operation_name) +
      "' is paused in lifecycle inactive state";
  return result;
}

}  // namespace configflux::ros2
