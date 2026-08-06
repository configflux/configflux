// SPDX-License-Identifier: BUSL-1.1

#include "configflux/sdk/runtime_session.h"
#include "configflux/sdk/runtime_linked_c_abi.h"

#include <iostream>
#include <string>

namespace {

using configflux::sdk::RuntimeSdkStatus;
using configflux::sdk::RuntimeSession;

// A well-formed runtime-open request that carries NO usable `.ccm` (`ccm_ref`
// is absent). Under ADR-0030 D2 — uniform across the CLI and the C ABI per
// ADR-0030 Amendment 1 — this open MUST fail closed: the linked Rust ABI
// returns a `status=error` envelope with `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`
// and no session handle. This test pins that the SDK-facing C ABI surface
// enforces the precondition (the gap the move from the compiler crate to the
// runtime crate closes) while still exercising the full linked symbol path:
// the version handshake, open-request marshaling, the JSON envelope round-trip,
// boundary-status mapping, and ABI-owned string release.
constexpr char kRuntimeOpenRequestJson[] = R"({
  "schema_version": 4,
  "model_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "resolve_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "scope": "component:thermal_control",
  "resolved_output": {
    "thermal_control": {
      "package": "merged_root",
      "version": "0.0.0",
      "components": {
        "thermal_control": {
          "type": "controller",
          "params": {}
        }
      }
    }
  }
})";

#define CHECK_TRUE(expr)                                                        \
  do {                                                                          \
    if (!(expr)) {                                                              \
      std::cerr << "check failed at " << __FILE__ << ":" << __LINE__ << ": "   \
                << #expr << std::endl;                                          \
      return false;                                                             \
    }                                                                           \
  } while (false)

#define CHECK_EQ(lhs, rhs) CHECK_TRUE((lhs) == (rhs))

bool TestLinkedOpenFailsClosedWithoutUsableCcm() {
  RuntimeSession session(configflux::sdk::MakeLinkedRuntimeCAbiApi());

  // The open call itself is well-formed, so the boundary status is Ok and the
  // version handshake succeeds (an expected_minor=0 client stays compatible
  // with ABI 1.1: expected_minor <= ABI minor). The refusal is a runtime-domain
  // rejection carried in the response envelope, not a boundary error.
  auto open_result = session.Open(kRuntimeOpenRequestJson);
  CHECK_TRUE(open_result.ok());
  CHECK_EQ(open_result.abi_version.major, 1u);
  CHECK_EQ(open_result.abi_version.minor, 1u);

  // ADR-0030 D2 on the C ABI surface: fail closed with the model-unavailable
  // code and return no session handle.
  CHECK_TRUE(open_result.open_response_json.find("\"status\":\"error\"") !=
             std::string::npos);
  CHECK_TRUE(open_result.open_response_json.find(
                 "E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE") != std::string::npos);
  CHECK_TRUE(!session.is_open());

  // No handle means session operations are rejected locally and a close on an
  // unopened session is a no-op Ok — the ABI-owned response string from the
  // failed open was still released through the SDK (no leak on the linked path).
  CHECK_EQ(session.Close(), RuntimeSdkStatus::kOk);
  CHECK_TRUE(!session.is_open());
  return true;
}

}  // namespace

int main() {
  const struct {
    const char* name;
    bool (*fn)();
  } tests[] = {
      {"linked_open_fails_closed_without_usable_ccm",
       &TestLinkedOpenFailsClosedWithoutUsableCcm},
  };

  int failures = 0;
  for (const auto& test : tests) {
    if (!test.fn()) {
      std::cerr << "FAILED: " << test.name << std::endl;
      ++failures;
    }
  }

  if (failures == 0) {
    std::cout << "All linked ABI tests passed." << std::endl;
    return 0;
  }
  return 1;
}
