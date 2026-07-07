// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_parameter_path.h"

#include <iostream>
#include <string>

namespace {

using configflux::ros2::parameter_path::NormalizeRuntimePath;
using configflux::ros2::parameter_path::RosNameToRuntimePath;
using configflux::ros2::parameter_path::RuntimePathToRosName;

#define CHECK_TRUE(expr)                                                        \
  do {                                                                          \
    if (!(expr)) {                                                              \
      std::cerr << "check failed at " << __FILE__ << ":" << __LINE__ << ": "   \
                << #expr << std::endl;                                          \
      return false;                                                             \
    }                                                                           \
  } while (false)

#define CHECK_EQ(lhs, rhs) CHECK_TRUE((lhs) == (rhs))

bool TestRosAndRuntimePathRoundTrip() {
  std::string runtime_path;
  CHECK_TRUE(RosNameToRuntimePath("thermal_control.max_rpm", &runtime_path));
  CHECK_EQ(runtime_path, "component.thermal_control.param.max_rpm");

  std::string ros_name;
  CHECK_TRUE(RuntimePathToRosName(runtime_path, &ros_name));
  CHECK_EQ(ros_name, "thermal_control.max_rpm");
  return true;
}

bool TestNormalizeRuntimePathAcceptsCanonicalAndRosForms() {
  std::string runtime_path;
  CHECK_TRUE(
      NormalizeRuntimePath("component.thermal_control.param.max_rpm", &runtime_path));
  CHECK_EQ(runtime_path, "component.thermal_control.param.max_rpm");

  CHECK_TRUE(NormalizeRuntimePath("thermal_control.max_rpm", &runtime_path));
  CHECK_EQ(runtime_path, "component.thermal_control.param.max_rpm");
  return true;
}

bool TestInvalidPathInputsAreRejected() {
  std::string output;
  CHECK_TRUE(!RuntimePathToRosName("component.thermal_control.max_rpm", &output));
  CHECK_TRUE(!RuntimePathToRosName("component..param.max_rpm", &output));
  CHECK_TRUE(!RosNameToRuntimePath("max_rpm", &output));
  CHECK_TRUE(!NormalizeRuntimePath("", &output));
  CHECK_TRUE(!NormalizeRuntimePath("component.only_prefix", &output));
  return true;
}

}  // namespace

int main() {
  bool ok = true;
  ok = TestRosAndRuntimePathRoundTrip() && ok;
  ok = TestNormalizeRuntimePathAcceptsCanonicalAndRosForms() && ok;
  ok = TestInvalidPathInputsAreRejected() && ok;

  if (!ok) {
    std::cerr << "runtime_parameter_path_test failed" << std::endl;
    return 1;
  }
  std::cout << "runtime_parameter_path_test passed" << std::endl;
  return 0;
}
