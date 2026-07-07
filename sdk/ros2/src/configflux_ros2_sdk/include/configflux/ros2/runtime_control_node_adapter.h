// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <memory>
#include <string>
#include <string_view>

#include "rclcpp/rclcpp.hpp"
#include "rclcpp_action/rclcpp_action.hpp"

#include "configflux/ros2/runtime_control_adapter.h"
#include "configflux_ros2_sdk/action/pull_updates.hpp"
#include "configflux_ros2_sdk/msg/runtime_control_result.hpp"
#include "configflux_ros2_sdk/srv/check_for_updates.hpp"
#include "configflux_ros2_sdk/srv/commit_configuration.hpp"
#include "configflux_ros2_sdk/srv/export_pending_sync_bundle.hpp"
#include "configflux_ros2_sdk/srv/get_sync_status.hpp"
#include "configflux_ros2_sdk/srv/push_audit_events.hpp"
#include "configflux_ros2_sdk/srv/rollback_dirty.hpp"

namespace configflux::ros2 {

struct RuntimeControlLifecyclePolicy {
  bool allow_commit_configuration_while_inactive = false;
  bool allow_rollback_dirty_while_inactive = false;
  bool allow_check_for_updates_while_inactive = false;
  bool allow_get_sync_status_while_inactive = true;
  bool allow_export_pending_sync_bundle_while_inactive = false;
  bool allow_push_audit_events_while_inactive = false;
  bool allow_pull_updates_while_inactive = false;
};

class RuntimeControlNodeAdapter {
 public:
  using CommitConfigurationSrv = configflux_ros2_sdk::srv::CommitConfiguration;
  using RollbackDirtySrv = configflux_ros2_sdk::srv::RollbackDirty;
  using CheckForUpdatesSrv = configflux_ros2_sdk::srv::CheckForUpdates;
  using GetSyncStatusSrv = configflux_ros2_sdk::srv::GetSyncStatus;
  using ExportPendingSyncBundleSrv =
      configflux_ros2_sdk::srv::ExportPendingSyncBundle;
  using PushAuditEventsSrv = configflux_ros2_sdk::srv::PushAuditEvents;
  using PullUpdatesAction = configflux_ros2_sdk::action::PullUpdates;
  using PullGoalHandle = rclcpp_action::ServerGoalHandle<PullUpdatesAction>;

  RuntimeControlNodeAdapter(rclcpp::Node* node,
                            RuntimeControlAdapter* runtime_adapter);

  bool StartServers(std::string service_prefix = "configflux/runtime_control");
  void StopServers();

  void SetLifecycleActive(bool active);
  bool lifecycle_active() const;

  void SetLifecyclePolicy(RuntimeControlLifecyclePolicy policy);
  const RuntimeControlLifecyclePolicy& lifecycle_policy() const;

 private:
  using RuntimeControlResultMsg =
      configflux_ros2_sdk::msg::RuntimeControlResult;

  void HandleCommitConfiguration(
      const std::shared_ptr<CommitConfigurationSrv::Request> request,
      std::shared_ptr<CommitConfigurationSrv::Response> response);
  void HandleRollbackDirty(
      const std::shared_ptr<RollbackDirtySrv::Request> request,
      std::shared_ptr<RollbackDirtySrv::Response> response);
  void HandleCheckForUpdates(
      const std::shared_ptr<CheckForUpdatesSrv::Request> request,
      std::shared_ptr<CheckForUpdatesSrv::Response> response);
  void HandleGetSyncStatus(
      const std::shared_ptr<GetSyncStatusSrv::Request> request,
      std::shared_ptr<GetSyncStatusSrv::Response> response);
  void HandleExportPendingSyncBundle(
      const std::shared_ptr<ExportPendingSyncBundleSrv::Request> request,
      std::shared_ptr<ExportPendingSyncBundleSrv::Response> response);
  void HandlePushAuditEvents(
      const std::shared_ptr<PushAuditEventsSrv::Request> request,
      std::shared_ptr<PushAuditEventsSrv::Response> response);

  rclcpp_action::GoalResponse HandlePullGoal(
      const rclcpp_action::GoalUUID& goal_id,
      std::shared_ptr<const PullUpdatesAction::Goal> goal);
  rclcpp_action::CancelResponse HandlePullCancel(
      const std::shared_ptr<PullGoalHandle> goal_handle);
  void HandlePullAccepted(const std::shared_ptr<PullGoalHandle> goal_handle);
  void ExecutePullGoal(const std::shared_ptr<PullGoalHandle> goal_handle);

  static bool ToPullUpdatesSource(uint8_t source,
                                  PullUpdatesSource* out_source);
  static bool PopulatePullUpdateWrite(
      const configflux_ros2_sdk::msg::PullUpdateWrite& write_msg,
      PullUpdateWrite* out_write, std::string* out_reason);
  static void PopulateResult(const RuntimeControlResult& result,
                             RuntimeControlResultMsg* out_result);
  static std::string ComposeName(const std::string& prefix,
                                 const std::string& suffix);
  bool IsOperationAllowedWhileInactive(
      std::string_view operation_name) const;
  static RuntimeControlResult BuildLifecycleInactiveResult(
      std::string operation_name);

  rclcpp::Node* node_ = nullptr;
  RuntimeControlAdapter* runtime_adapter_ = nullptr;
  std::string service_prefix_;
  bool lifecycle_active_ = true;
  RuntimeControlLifecyclePolicy lifecycle_policy_;

  rclcpp::Service<CommitConfigurationSrv>::SharedPtr commit_service_;
  rclcpp::Service<RollbackDirtySrv>::SharedPtr rollback_service_;
  rclcpp::Service<CheckForUpdatesSrv>::SharedPtr check_updates_service_;
  rclcpp::Service<GetSyncStatusSrv>::SharedPtr get_sync_status_service_;
  rclcpp::Service<ExportPendingSyncBundleSrv>::SharedPtr
      export_pending_sync_bundle_service_;
  rclcpp::Service<PushAuditEventsSrv>::SharedPtr push_audit_events_service_;

  rclcpp_action::Server<PullUpdatesAction>::SharedPtr pull_updates_action_server_;
};

}  // namespace configflux::ros2
