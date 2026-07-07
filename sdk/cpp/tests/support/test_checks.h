// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <iostream>

#define CHECK_TRUE(expr)                                                        \
  do {                                                                          \
    if (!(expr)) {                                                              \
      std::cerr << "check failed at " << __FILE__ << ":" << __LINE__ << ": "   \
                << #expr << std::endl;                                          \
      return false;                                                             \
    }                                                                           \
  } while (false)

#define CHECK_EQ(lhs, rhs) CHECK_TRUE((lhs) == (rhs))
