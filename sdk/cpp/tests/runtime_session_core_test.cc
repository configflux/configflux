// SPDX-License-Identifier: BUSL-1.1

#include "configflux/sdk/runtime_session.h"

#include <string>

#include "support/fake_runtime_c_abi.h"
#include "support/test_checks.h"
#include "support/test_harness.h"

namespace {

using configflux::sdk::RuntimeCAbiApi;
using configflux::sdk::RuntimeOperation;
using configflux::sdk::RuntimeSdkStatus;
using configflux::sdk::RuntimeSession;
using configflux::sdk::test_support::FakeApi;
using configflux::sdk::test_support::ResetFakeState;

bool TestOpenExecuteSnapshotCloseRoundTrip() {
  ResetFakeState();
  RuntimeSession session(FakeApi());

  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());
  CHECK_TRUE(open_result.message.empty());
  CHECK_TRUE(session.is_open());

  auto sync_result = session.GetSyncStatus(R"({"schema_version":2})");
  CHECK_TRUE(sync_result.ok());
  CHECK_TRUE(sync_result.response_json.find("\"operation\":15") !=
             std::string::npos);

  auto snapshot_result = session.SnapshotJson();
  CHECK_TRUE(snapshot_result.ok());
  CHECK_EQ(snapshot_result.response_json, R"({"snapshot":"ok"})");

  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  CHECK_TRUE(!session.is_open());
  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  return true;
}

bool TestVersionMismatchAndOpenDomainError() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto mismatch_result = session.Open(R"({"open":"ok"})", 2, 0);
  CHECK_EQ(mismatch_result.status, RuntimeSdkStatus::kVersionMismatch);
  CHECK_TRUE(!session.is_open());

  auto domain_error_result = session.Open(R"({"domain_error":true})");
  CHECK_TRUE(domain_error_result.ok());
  CHECK_EQ(domain_error_result.open_response_json,
           R"({"status":"error","code":"E_RUNTIME_OPEN_INVALID"})");
  CHECK_TRUE(!session.is_open());
  return true;
}

bool TestDeterministicLocalStatusMapping() {
  ResetFakeState();
  RuntimeSession unbound_session(RuntimeCAbiApi{});
  auto unbound_open = unbound_session.Open(R"({"open":"ok"})");
  CHECK_EQ(unbound_open.status, RuntimeSdkStatus::kApiNotBound);

  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  std::string invalid_request = "{\"schema_version\":1}";
  invalid_request.push_back('\0');
  invalid_request.push_back('x');
  auto invalid_result =
      session.Execute(RuntimeOperation::kGetSyncStatus, invalid_request);
  CHECK_EQ(invalid_result.status, RuntimeSdkStatus::kInvalidArgument);

  auto runtime_error_result =
      session.Execute(RuntimeOperation::kGetSyncStatus,
                      R"({"return_invalid_json":true})");
  CHECK_EQ(runtime_error_result.status, RuntimeSdkStatus::kInvalidJson);

  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  auto closed_result =
      session.Execute(RuntimeOperation::kGetSyncStatus, R"({"schema_version":2})");
  CHECK_EQ(closed_result.status, RuntimeSdkStatus::kSessionClosed);
  return true;
}

}  // namespace

namespace configflux::sdk::test_support {

void RegisterRuntimeSessionCoreTests(std::vector<TestCase>* tests) {
  tests->push_back({"open_execute_snapshot_close_round_trip",
                    &TestOpenExecuteSnapshotCloseRoundTrip});
  tests->push_back({"version_mismatch_and_open_domain_error",
                    &TestVersionMismatchAndOpenDomainError});
  tests->push_back({"deterministic_local_status_mapping",
                    &TestDeterministicLocalStatusMapping});
}

}  // namespace configflux::sdk::test_support
