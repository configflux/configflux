// SPDX-License-Identifier: BUSL-1.1

#include <iostream>
#include <vector>

#include "support/test_harness.h"

int main() {
  using configflux::sdk::test_support::RegisterRuntimeSessionCoreTests;
  using configflux::sdk::test_support::RegisterRuntimeSessionEventTests;
  using configflux::sdk::test_support::RegisterRuntimeSessionThreadingTests;
  using configflux::sdk::test_support::TestCase;

  std::vector<TestCase> tests;
  RegisterRuntimeSessionCoreTests(&tests);
  RegisterRuntimeSessionThreadingTests(&tests);
  RegisterRuntimeSessionEventTests(&tests);

  int failures = 0;
  for (const TestCase& test : tests) {
    if (!test.fn()) {
      std::cerr << "FAILED: " << test.name << std::endl;
      ++failures;
    }
  }

  if (failures == 0) {
    std::cout << "All runtime_session tests passed (" << tests.size()
              << " cases)" << std::endl;
    return 0;
  }

  std::cerr << failures << " test(s) failed" << std::endl;
  return 1;
}
