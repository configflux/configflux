// SPDX-License-Identifier: BUSL-1.1

#include "configflux/sdk/runtime_session.h"

#include <atomic>
#include <thread>
#include <vector>

#include "support/fake_runtime_c_abi.h"
#include "support/test_checks.h"
#include "support/test_harness.h"

namespace {

using configflux::sdk::RuntimeOperation;
using configflux::sdk::RuntimeSdkStatus;
using configflux::sdk::RuntimeSession;
using configflux::sdk::test_support::FakeApi;
using configflux::sdk::test_support::ResetFakeState;
using configflux::sdk::test_support::g_fake_state;

bool TestThreadSafeExecuteSerialization() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());
  CHECK_TRUE(session.is_open());

  std::atomic<bool> saw_error{false};
  std::vector<std::thread> threads;
  threads.reserve(8);
  for (int thread_index = 0; thread_index < 8; ++thread_index) {
    threads.emplace_back([&session, &saw_error]() {
      for (int i = 0; i < 16; ++i) {
        auto result =
            session.Execute(RuntimeOperation::kListParameters,
                            R"({"schema_version":2,"scope":"component:thermal_control"})");
        if (!result.ok()) {
          saw_error.store(true);
          return;
        }
      }
    });
  }
  for (auto& thread : threads) {
    thread.join();
  }

  CHECK_TRUE(!saw_error.load());
  CHECK_EQ(g_fake_state.max_execute_in_flight.load(), 1);
  CHECK_EQ(g_fake_state.execute_calls.load(), 8 * 16);
  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  return true;
}

}  // namespace

namespace configflux::sdk::test_support {

void RegisterRuntimeSessionThreadingTests(std::vector<TestCase>* tests) {
  tests->push_back(
      {"thread_safe_execute_serialization", &TestThreadSafeExecuteSerialization});
}

}  // namespace configflux::sdk::test_support
