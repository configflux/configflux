// SPDX-License-Identifier: BUSL-1.1

// Linked-C-ABI driver for the C++ SDK's documented integration example
// (sdk/cpp/README.md "Integration Example"), exercised end to end by
// sdk/cpp/tests/runtime_sdk_linked_abi_e2e_test.sh against a snapshot the real
// compiler and interpreter binaries produced (ADR-0061 D3, configflux-x5gb.8).
//
// It links //runtime:runtime_c_abi_static, exactly as
// runtime_session_linked_abi_test does, so every operation below crosses the
// real ABI: version handshake, open-request marshaling, operation dispatch,
// snapshot injection and update inside the session, envelope round-trip,
// boundary-status mapping and ABI-owned string release. Nothing here is
// simulated -- there is no test double on this path.
//
// The driver takes no product decisions: every response string is emitted
// verbatim (it is already JSON), and every assertion lives in the shell test.
//
// Usage:
//   runtime_session_e2e_driver <open-request.json> <param-path> <new-value-json>
//                              <immutable-path>
//
// Output: one JSON object per stdout line, in this order --
//   {"step":"open","ok":true,"response":<open response>}
//   {"step":"get_before","response":<get-parameter response>}
//   {"step":"set","response":<set-parameter response>}
//   {"step":"get_after","response":<get-parameter response>}
//   {"step":"set_immutable","status":<int>,"response":<set-parameter response>}
//   {"step":"snapshot","response":<session snapshot>}
//
// Exit codes: 0 all steps emitted; 2 usage/IO error; 3 the open failed at the
// ABI boundary (a single {"step":"open","ok":false,...} line is emitted).

#include <cstdint>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <string_view>

#include "configflux/sdk/runtime_linked_c_abi.h"
#include "configflux/sdk/runtime_session.h"

namespace {

using configflux::sdk::RuntimeCallResult;
using configflux::sdk::RuntimeSdkStatus;
using configflux::sdk::RuntimeSession;

bool ReadFile(const std::string& path, std::string* out) {
  std::ifstream stream(path, std::ios::binary);
  if (!stream) {
    return false;
  }
  std::ostringstream buffer;
  buffer << stream.rdbuf();
  *out = buffer.str();
  return true;
}

int StatusCode(RuntimeSdkStatus status) { return static_cast<int>(static_cast<uint32_t>(status)); }

// The response strings the ABI returns are already JSON documents, so they are
// embedded verbatim rather than re-encoded.
void EmitStep(std::string_view step, const std::string& response_json) {
  std::cout << R"({"step":")" << step << R"(","response":)" << response_json << "}\n";
}

// PRODUCT_SCHEMA_VERSION as of this test. The linked-ABI test beside this file
// pins the same literal for the same reason: a C++ client has no compiled
// access to the Rust constant, so a schema bump updates both by hand.
constexpr int kSchemaVersion = 5;

std::string GetRequest(const std::string& path) {
  std::ostringstream request;
  request << R"({"schema_version":)" << kSchemaVersion << R"(,"path":")" << path << R"("})";
  return request.str();
}

std::string SetRequest(const std::string& path, const std::string& value_json) {
  std::ostringstream request;
  request << R"({"schema_version":)" << kSchemaVersion << R"(,"path":")" << path << R"(","value":)"
          << value_json << "}";
  return request.str();
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 5) {
    std::cerr << "usage: " << argv[0]
              << " <open-request.json> <param-path> <new-value-json> <immutable-path>" << std::endl;
    return 2;
  }

  const std::string request_path = argv[1];
  const std::string param_path = argv[2];
  const std::string new_value_json = argv[3];
  const std::string immutable_path = argv[4];

  std::string open_request_json;
  if (!ReadFile(request_path, &open_request_json)) {
    std::cerr << "cannot read open request file: " << request_path << std::endl;
    return 2;
  }

  RuntimeSession session(configflux::sdk::MakeLinkedRuntimeCAbiApi());

  const auto open_result = session.Open(open_request_json);
  if (!open_result.ok()) {
    std::cout << R"({"step":"open","ok":false,"status":)" << StatusCode(open_result.status)
              << "}\n";
    return 3;
  }
  std::cout << R"({"step":"open","ok":true,"response":)" << open_result.open_response_json << "}\n";

  const RuntimeCallResult get_before = session.GetParameter(GetRequest(param_path));
  EmitStep("get_before", get_before.response_json);

  const RuntimeCallResult set = session.SetParameter(SetRequest(param_path, new_value_json));
  EmitStep("set", set.response_json);

  const RuntimeCallResult get_after = session.GetParameter(GetRequest(param_path));
  EmitStep("get_after", get_after.response_json);

  // A lifecycle rejection is a runtime-DOMAIN refusal carried in the response
  // envelope, so the boundary status stays Ok (docs/runtime-c-abi.md section 6).
  // The status is printed so the shell test can pin that mapping rather than
  // assume it.
  const RuntimeCallResult set_immutable =
      session.SetParameter(SetRequest(immutable_path, R"("foc")"));
  std::cout << R"({"step":"set_immutable","status":)" << StatusCode(set_immutable.status)
            << R"(,"response":)" << set_immutable.response_json << "}\n";

  const RuntimeCallResult snapshot = session.SnapshotJson();
  EmitStep("snapshot", snapshot.response_json);

  session.Close();
  return 0;
}
