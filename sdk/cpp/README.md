# C++ SDK

This directory hosts the first-party ConfigFlux C++ SDK implementation.

Rules:

1. Modern C++ (C++20+) only.
2. Clang/clang++ toolchain only.
3. Google C++ style.
4. No first-party C sources.

See:

- `docs/cpp-engineering-standards.md`

## Core Targets

1. `//sdk/cpp:runtime_sdk_core`
2. `//sdk/cpp:runtime_sdk_core_minimal`
3. `//sdk/cpp:runtime_sdk_core_test`

Build with clang policy:

```bash
tools/install_clang_toolchain.sh  # needed only when host clang is unavailable
bazel build --config=clang_cpp //sdk/cpp:runtime_sdk_core
bazel test --config=clang_cpp //sdk/cpp:runtime_sdk_core_test
```

## Integration Example

The C++ SDK wraps the runtime C ABI with a thread-safe RAII session object.
`RuntimeSession` serializes all ABI calls for a handle and maps ABI/local failures to
deterministic `RuntimeSdkStatus` values.

```cpp
#include "configflux/sdk/runtime_linked_c_abi.h"
#include "configflux/sdk/runtime_session.h"

namespace cfg = configflux::sdk;

cfg::RuntimeSession session(cfg::MakeLinkedRuntimeCAbiApi());
cfg::RuntimeOpenResult open = session.Open(runtime_open_request_json);
if (!open.ok()) {
  // Handle deterministic SDK/ABI boundary failure.
}

cfg::RuntimeCallResult result = session.GetParameter(request_json);
```

## Event Callbacks

`RuntimeSession` can fan out `subscribe_events` responses to typed callback channels:

1. parameter changed
2. dirty state changed
3. auto-reset occurred
4. commit applied
5. sync events (`sync_state_changed`, conflicts, apply-completed)

```cpp
auto subscription = session.SubscribeEventChannel(
    cfg::RuntimeEventChannel::kParameterChanged,
    [](const cfg::RuntimeEventNotification& event) {
      // Process event.event_kind / event.event_json payload.
      return cfg::RuntimeCallbackAction::kKeepSubscription;
    });

cfg::RuntimeEventDispatchResult dispatch = session.PollAndDispatchEvents(
    /*from_sequence=*/0, /*max_events=*/256);
if (!dispatch.ok()) {
  // Handle transport/parse failure.
}
```

## Minimal Footprint Profile

Use `//sdk/cpp:runtime_sdk_core_minimal` when binary size is the priority:

1. Compiles with `CONFIGFLUX_SDK_MINIMAL_PROFILE=1`.
2. Keeps only generic `Execute`/`SnapshotJson` APIs.
3. Removes per-operation convenience wrappers to reduce symbol surface.
