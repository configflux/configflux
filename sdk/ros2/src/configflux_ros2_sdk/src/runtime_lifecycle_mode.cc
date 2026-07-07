// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_lifecycle_mode.h"

#include <utility>

namespace configflux::ros2 {
namespace {

AdapterResult BuildLifecycleError(std::string reason) {
  AdapterResult result;
  result.status = sdk::RuntimeSdkStatus::kInvalidArgument;
  result.successful = false;
  result.reason = std::move(reason);
  return result;
}

}  // namespace

RuntimeLifecycleMode::RuntimeLifecycleMode(
    RuntimeParameterNodeAdapter* parameter_node_adapter,
    RuntimeControlNodeAdapter* control_node_adapter)
    : parameter_node_adapter_(parameter_node_adapter),
      control_node_adapter_(control_node_adapter) {}

AdapterResult RuntimeLifecycleMode::OnConfigure(
    const RuntimeLifecycleConfigureRequest& request) {
  if (parameter_node_adapter_ == nullptr || control_node_adapter_ == nullptr) {
    return BuildLifecycleError("lifecycle mode requires non-null adapters");
  }
  if (state_ != RuntimeLifecycleState::kUnconfigured) {
    return BuildLifecycleError("lifecycle mode is already configured");
  }
  if (request.scope_root.empty()) {
    return BuildLifecycleError("configure requires non-empty scope_root");
  }

  const AdapterResult declare_result =
      parameter_node_adapter_->DeclareParameters(request.scope_root);
  if (declare_result.status != sdk::RuntimeSdkStatus::kOk ||
      !declare_result.successful) {
    return declare_result;
  }
  if (!control_node_adapter_->StartServers(request.service_prefix)) {
    return BuildLifecycleError("failed to start runtime control servers");
  }

  callback_actor_ = request.callback_actor.empty()
                        ? "ros2.lifecycle.set_callback"
                        : request.callback_actor;
  callback_reason_ = request.callback_reason;
  if (callback_reason_.has_value() && callback_reason_->empty()) {
    callback_reason_.reset();
  }

  control_node_adapter_->SetLifecyclePolicy(request.inactive_policy);
  control_node_adapter_->SetLifecycleActive(false);
  parameter_node_adapter_->SetMutationsEnabled(false);
  state_ = RuntimeLifecycleState::kInactive;
  return AdapterResult{};
}

AdapterResult RuntimeLifecycleMode::OnActivate() {
  if (parameter_node_adapter_ == nullptr || control_node_adapter_ == nullptr) {
    return BuildLifecycleError("lifecycle mode requires non-null adapters");
  }
  if (state_ == RuntimeLifecycleState::kUnconfigured) {
    return BuildLifecycleError("activate requires configured lifecycle mode");
  }
  if (state_ == RuntimeLifecycleState::kActive) {
    return AdapterResult{};
  }
  if (parameter_node_adapter_->RegisterSetParametersCallback(callback_actor_,
                                                             callback_reason_) ==
      nullptr) {
    return BuildLifecycleError("failed to register parameter set callback");
  }
  parameter_node_adapter_->SetMutationsEnabled(true);
  control_node_adapter_->SetLifecycleActive(true);
  state_ = RuntimeLifecycleState::kActive;
  return AdapterResult{};
}

AdapterResult RuntimeLifecycleMode::OnDeactivate() {
  if (parameter_node_adapter_ == nullptr || control_node_adapter_ == nullptr) {
    return BuildLifecycleError("lifecycle mode requires non-null adapters");
  }
  if (state_ == RuntimeLifecycleState::kUnconfigured) {
    return BuildLifecycleError("deactivate requires configured lifecycle mode");
  }
  if (state_ == RuntimeLifecycleState::kInactive) {
    return AdapterResult{};
  }
  parameter_node_adapter_->SetMutationsEnabled(false);
  control_node_adapter_->SetLifecycleActive(false);
  state_ = RuntimeLifecycleState::kInactive;
  return AdapterResult{};
}

AdapterResult RuntimeLifecycleMode::OnCleanup() {
  if (parameter_node_adapter_ == nullptr || control_node_adapter_ == nullptr) {
    return BuildLifecycleError("lifecycle mode requires non-null adapters");
  }
  if (state_ == RuntimeLifecycleState::kUnconfigured) {
    return AdapterResult{};
  }
  parameter_node_adapter_->SetMutationsEnabled(false);
  parameter_node_adapter_->ClearSetParametersCallback();
  control_node_adapter_->SetLifecycleActive(false);
  control_node_adapter_->StopServers();
  state_ = RuntimeLifecycleState::kUnconfigured;
  return AdapterResult{};
}

AdapterResult RuntimeLifecycleMode::OnShutdown() { return OnCleanup(); }

RuntimeLifecycleState RuntimeLifecycleMode::state() const { return state_; }

bool RuntimeLifecycleMode::configured() const {
  return state_ != RuntimeLifecycleState::kUnconfigured;
}

bool RuntimeLifecycleMode::active() const {
  return state_ == RuntimeLifecycleState::kActive;
}

}  // namespace configflux::ros2
