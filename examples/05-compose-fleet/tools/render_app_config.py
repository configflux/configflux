#!/usr/bin/env python3
"""Render a resolved ConfigFlux snapshot into a NESTED app-config document.

>>> THIS IS USER-SIDE GLUE, NOT A PRODUCT FEATURE. <<<

ConfigFlux emits the resolved snapshot — the rich artifact that carries *every*
parameter and its full metadata across all lifecycle classes. It deliberately
does NOT emit nested app-config files, env-files, or any framework-specific
shape (ADR-0032 D3/D5, reaffirmed by ADR-0035 C3; docs/service-integration-
guide.md). Shaping the snapshot into whatever your service framework mounts is a
thin transform that lives in YOUR repository.

This script is that transform, shown in the place a user would keep it: next to
the compose-override transform it sits beside (gen_compose_override.py). Where
gen_compose_override.py FLATTENS the resolved leaves into uppercased
COMPONENT_PARAM=value environment entries, this one renders a NESTED document —
the shape layered app-config files take in common service frameworks: a
hierarchical settings file with sections, mounted unchanged by every service
that reads it. That nested shape is the whole point: it demonstrates the
"services keep mounting the same layered app-config file" consumption pattern.

The rendered document is portable, dependency-free JSON (python stdlib only, no
PyYAML), emitted with sorted keys and stable indentation so it is byte-stable —
a nested JSON document is exactly the shape many frameworks accept for a layered
config file, and it keeps the example hermetic. If your framework wants YAML, a
TOML file, or a different nesting, you change THIS file — the product surface
does not move.

The document has two top-level parts:

  * "lineage" — a block recording model_hash, resolve_hash, and scope, so the
    rendered file is traceable to the exact resolution it came from (the same
    lineage gen_compose_override.py records as comments).
  * one section per scope root, nested component -> param -> value, mirroring the
    snapshot's resolved_output[<root>].components.<c>.params.<p>.value leaves.

Lineage first: like any Pattern 1 consumer we verify the resolve status is ok
and that a resolve_hash is present, and refuse to render otherwise.

Usage:
  render_app_config.py --snapshot <resolve_result.json> [--out <app-config.json>]

Output goes to --out, or stdout if --out is omitted.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any


def scope_root(snapshot: dict[str, Any]) -> str:
    """Return the resolved_output root key for the snapshot's scope."""
    scope = snapshot.get("scope", "")
    return scope.split(":", 1)[1] if ":" in scope else scope


def nested_config(snapshot: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Build a nested component -> {param -> value} mapping from resolved_output.

    Mirrors the leaf path the compose-override transform and the Pattern 1
    service read (resolved_output[root].components.<c>.params.<p>.value), but
    preserves the component/param nesting instead of flattening it.
    """
    root = scope_root(snapshot)
    components = snapshot["resolved_output"][root]["components"]
    config: dict[str, dict[str, Any]] = {}
    for component, cdef in components.items():
        params = cdef.get("params", {})
        config[component] = {param: leaf.get("value") for param, leaf in params.items()}
    return config


def render_document(snapshot: dict[str, Any]) -> dict[str, Any]:
    """Assemble the nested app-config document (a JSON-serializable dict)."""
    root = scope_root(snapshot)
    return {
        "lineage": {
            "model_hash": snapshot.get("model_hash"),
            "resolve_hash": snapshot.get("resolve_hash"),
            "scope": snapshot.get("scope"),
        },
        # The config section is keyed by scope root so multiple scoped renders
        # could be merged into one file by a consumer without key collisions.
        root: nested_config(snapshot),
    }


def render_text(snapshot: dict[str, Any]) -> str:
    """Serialize the nested document deterministically (sorted keys, indented)."""
    document = render_document(snapshot)
    return json.dumps(document, sort_keys=True, indent=2) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshot", required=True, help="path to a resolve_result.*.json")
    parser.add_argument("--out", default="", help="output path (default: stdout)")
    args = parser.parse_args(argv)

    try:
        with open(args.snapshot, "r", encoding="utf-8") as handle:
            snapshot = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"ERROR: cannot read snapshot {args.snapshot!r}: {exc}", file=sys.stderr)
        return 1

    if snapshot.get("status") != "ok":
        print(
            f"ERROR: refusing to render from a non-ok snapshot (status="
            f"{snapshot.get('status')!r})",
            file=sys.stderr,
        )
        return 1
    if not snapshot.get("resolve_hash"):
        print("ERROR: snapshot carries no resolve_hash; refusing to render", file=sys.stderr)
        return 1

    rendered = render_text(snapshot)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as out:
            out.write(rendered)
        print(f"wrote nested app-config document -> {args.out}")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
