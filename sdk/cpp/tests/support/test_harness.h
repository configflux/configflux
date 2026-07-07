// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <vector>

namespace configflux::sdk::test_support {

struct TestCase {
  const char* name;
  bool (*fn)();
};

void RegisterRuntimeSessionCoreTests(std::vector<TestCase>* tests);
void RegisterRuntimeSessionThreadingTests(std::vector<TestCase>* tests);
void RegisterRuntimeSessionEventTests(std::vector<TestCase>* tests);

}  // namespace configflux::sdk::test_support
