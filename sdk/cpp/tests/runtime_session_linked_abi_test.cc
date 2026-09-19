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
//
// `resolve_hash` below is the REAL hash of this payload, not a stand-in. The
// compiler's resolve-hash cross-validation is unconditional, so it now runs
// even though this request carries no provenance at all — and it runs BEFORE
// the `.ccm` precondition, because that precondition is layered on the snapshot
// a successful open returns. A made-up hash would therefore be rejected as
// `E_RUNTIME_HASH_MISMATCH` and this test would never reach the behaviour it
// exists to pin.
//
// It is a constant because a C++ test cannot compute the canonical pre-image.
// To refresh it after changing the payload below (or after a
// PRODUCT_SCHEMA_VERSION bump, which also moves the `schema_version` literal
// here): run the request through `runtime-open` and copy the value the
// rejection names in `resolve_hash mismatch (expected '<hash>', got ...)`. The
// stale-constant check in the test body prints that envelope for you.
constexpr char kRuntimeOpenRequestJson[] = R"({
  "schema_version": 5,
  "model_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "resolve_hash": "93549a22442617b51d8347e4be14aa9f19c4007de1d75cde56d9a1a68f659c34",
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
  // with ABI 1.3: expected_minor <= ABI minor — which is the whole point of
  // moving the minor rather than the major, and this assertion is what proves
  // the linked client still handshakes across the bump). The refusal is a
  // runtime-domain rejection carried in the response envelope, not a boundary
  // error.
  auto open_result = session.Open(kRuntimeOpenRequestJson);
  CHECK_TRUE(open_result.ok());
  CHECK_EQ(open_result.abi_version.major, 1u);
  CHECK_EQ(open_result.abi_version.minor, 3u);

  // Stale-constant guard, checked before the assertion it would otherwise
  // sabotage: a hash mismatch here means `resolve_hash` in the payload above no
  // longer matches it, so the open is rejected on the hash and never reaches
  // the `.ccm` precondition. The printed envelope names the value to paste back.
  if (open_result.open_response_json.find("E_RUNTIME_HASH_MISMATCH") !=
      std::string::npos) {
    std::cerr << "stale resolve_hash constant in kRuntimeOpenRequestJson; "
              << "runtime replied: " << open_result.open_response_json
              << std::endl;
    return false;
  }

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
