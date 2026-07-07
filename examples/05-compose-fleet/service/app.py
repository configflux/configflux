#!/usr/bin/env python3
"""Containerized service that consumes a resolved ConfigFlux snapshot.

This is *Pattern 1* from docs/service-integration-guide.md: a language-agnostic
consumer that reads the resolved snapshot JSON directly at startup. No ConfigFlux
binary, no C ABI, no runtime subprocess — just JSON. The same file runs

  * inside a docker-compose stack (the snapshot is mounted/copied into the image
    and pointed at via CONFIGFLUX_SNAPSHOT), and
  * standalone on a developer machine for debugging (run_standalone.sh points
    CONFIGFLUX_SNAPSHOT at the local-environment bundle's snapshot).

Startup contract (the headline operational facts):

  * The snapshot is IMMUTABLE. We read it once, never write back. Runtime
    mutation is a different pattern (the runtime CLI); a read-only consumer
    ignores it.
  * We FAIL CLOSED. If the snapshot is missing, unparseable, reports a non-ok
    resolve status, or its lineage hash does not match the one we were deployed
    with, we exit non-zero and refuse to serve a configuration we cannot trust.
  * We PIN the lineage. model_hash identifies the compiled model; resolve_hash
    identifies this one resolution. If CONFIGFLUX_EXPECT_RESOLVE_HASH is set we
    verify the snapshot matches it (the deploy pipeline knows which resolution
    it shipped). This is what makes a configuration traceable end to end.

Environment:

  CONFIGFLUX_SNAPSHOT             path to resolve_result.<root>.<selection>.json
                                 (required)
  CONFIGFLUX_EXPECT_RESOLVE_HASH expected resolve_hash to pin against (optional
                                 but recommended; mismatch fails closed)

Exit codes: 0 ok; 1 fail-closed (missing / unparseable / not-ok / hash mismatch).
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any


def _fail(message: str) -> "NoReturn":  # type: ignore[name-defined]
    """Print a fail-closed diagnostic to stderr and exit non-zero."""
    print(f"FATAL: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_snapshot(path: str) -> dict[str, Any]:
    """Load a resolved snapshot, failing closed on anything untrustworthy."""
    if not path:
        _fail("CONFIGFLUX_SNAPSHOT is not set; nothing to load")
    if not os.path.isfile(path):
        _fail(f"snapshot not found at {path!r}")
    try:
        with open(path, "r", encoding="utf-8") as handle:
            snapshot = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        _fail(f"snapshot at {path!r} is unreadable or not valid JSON: {exc}")
    if snapshot.get("status") != "ok":
        _fail(f"resolve status is {snapshot.get('status')!r}, expected 'ok'")
    return snapshot


def verify_lineage(snapshot: dict[str, Any]) -> None:
    """Pin/verify the resolution lineage; fail closed on a mismatch."""
    expected = os.environ.get("CONFIGFLUX_EXPECT_RESOLVE_HASH", "").strip()
    actual = snapshot.get("resolve_hash", "")
    if not actual:
        _fail("snapshot carries no resolve_hash; refusing to trust it")
    if expected and actual != expected:
        _fail(
            "resolve_hash mismatch — the snapshot is not the configuration this "
            f"service was deployed with (expected {expected}, got {actual})"
        )


def scope_root(snapshot: dict[str, Any]) -> str:
    """Return the resolved_output root key for the snapshot's scope."""
    scope = snapshot.get("scope", "")
    return scope.split(":", 1)[1] if ":" in scope else scope


def get_value(snapshot: dict[str, Any], component: str, param: str) -> Any:
    """Read one resolved leaf value: resolved_output[root].components[c].params[p].value."""
    root = scope_root(snapshot)
    try:
        return snapshot["resolved_output"][root]["components"][component]["params"][param][
            "value"
        ]
    except (KeyError, TypeError) as exc:
        _fail(f"resolved value {component}.{param} not present in snapshot: {exc}")


def iter_params(snapshot: dict[str, Any]):
    """Yield (component, param, value) for every resolved leaf under the scope root."""
    root = scope_root(snapshot)
    components = snapshot["resolved_output"][root]["components"]
    for component in sorted(components):
        params = components[component].get("params", {})
        for param in sorted(params):
            yield component, param, params[param].get("value")


def main() -> int:
    snapshot_path = os.environ.get("CONFIGFLUX_SNAPSHOT", "")
    snapshot = load_snapshot(snapshot_path)
    verify_lineage(snapshot)

    root = scope_root(snapshot)
    env_label = os.environ.get("CONFIGFLUX_ENV_LABEL", "(unnamed)")

    print(f"service: loaded scoped configuration from {snapshot_path}")
    print(f"  environment : {env_label}")
    print(f"  scope       : {snapshot.get('scope')}")
    print(f"  model_hash  : {snapshot.get('model_hash')}")
    print(f"  resolve_hash: {snapshot.get('resolve_hash')}")
    print("  resolved values:")
    for component, param, value in iter_params(snapshot):
        print(f"    {component}.{param} = {value}")

    # A real long-lived service would now enter its work loop using these
    # values (connect the broker_endpoint, tick every tick_interval ms, log at
    # log_level). For a deterministic demo we read one representative value to
    # prove the service is driving off the resolved config, then exit cleanly.
    broker = get_value(snapshot, root, "broker_endpoint")
    print(f"  -> {root} would connect its broker at {broker}")
    print("service: startup OK (read-only Pattern 1 consumer); exiting 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
