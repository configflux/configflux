// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>
#include <optional>
#include <string>

#include "configflux/ros2/runtime_control_node_adapter.h"
#include "configflux/ros2/runtime_parameter_node_adapter.h"

namespace configflux::ros2 {

enum class RuntimeLifecycleState : uint8_t {
  kUnconfigured = 0,
  kInactive = 1,
  kActive = 2,
};

struct RuntimeLifecycleConfigureRequest {
  std::string scope_root;
  std::string service_prefix = "configflux/runtime_control";
  std::string callback_actor = "ros2.lifecycle.set_callback";
  std::optional<std::string> callback_reason;
  RuntimeControlLifecyclePolicy inactive_policy;
};

class RuntimeLifecycleMode {
 public:
  RuntimeLifecycleMode(RuntimeParameterNodeAdapter* parameter_node_adapter,
                       RuntimeControlNodeAdapter* control_node_adapter);

  AdapterResult OnConfigure(const RuntimeLifecycleConfigureRequest& request);
  AdapterResult OnActivate();
  AdapterResult OnDeactivate();
  AdapterResult OnCleanup();
  AdapterResult OnShutdown();

  RuntimeLifecycleState state() const;
  bool configured() const;
  bool active() const;

 private:
  RuntimeParameterNodeAdapter* parameter_node_adapter_ = nullptr;
  RuntimeControlNodeAdapter* control_node_adapter_ = nullptr;
  RuntimeLifecycleState state_ = RuntimeLifecycleState::kUnconfigured;
  std::string callback_actor_ = "ros2.lifecycle.set_callback";
  std::optional<std::string> callback_reason_;
};

}  // namespace configflux::ros2
