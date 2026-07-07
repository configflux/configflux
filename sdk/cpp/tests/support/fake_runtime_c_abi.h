// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <atomic>

#include "configflux/sdk/runtime_session.h"

namespace configflux::sdk::test_support {

struct FakeRuntimeState {
  std::atomic<int> next_session_id{1};
  std::atomic<int> execute_in_flight{0};
  std::atomic<int> max_execute_in_flight{0};
  std::atomic<int> execute_calls{0};
  std::atomic<int> subscribe_calls{0};
};

extern FakeRuntimeState g_fake_state;

RuntimeCAbiApi FakeApi();
void ResetFakeState();

}  // namespace configflux::sdk::test_support
