# ConfigFlux Service Integration Guide

This guide explains how a service — in **any** language, with no ConfigFlux
SDK embedded in it — consumes a resolved ConfigFlux configuration at runtime.
It is organized around the **three consumption patterns** that exist today,
from simplest to most advanced. The patterns are language- and
topology-agnostic; **Python** and **C#** appear as worked reference languages,
and a reader writing Go, Java, Node, or Rust can transpose the snippets
directly. The patterns hold whether your software is a single monolith or a
stack of independently deployed services.

The three patterns:

1. **Read-only startup config** — the service reads its resolved snapshot
   (plain JSON) at startup. No ConfigFlux binary runs inside the service.
2. **Validated session via the runtime CLI** — the service drives the
   `configflux-runtime` executable as a subprocess, exchanging JSON over
   stdin/stdout (or files), to read and to mutate runtime-lifecycle
   parameters with full domain validation.
3. **C ABI (advanced)** — direct in-process FFI. Documented for completeness;
   native bindings are still under evaluation (see
   [Pattern 3](#pattern-3--c-abi-advanced)).

Most services need only Pattern 1 or Pattern 2.

## Prerequisites

This guide assumes a **compiled and resolved bundle already exists** —
produced by `configflux-compiler compile` and then `cfx resolve` (the one-shot
resolver). If you have not yet produced one, work through the
[canonical worked example](canonical-worked-example.md) first; it walks the
full compile → resolve → runtime-handoff pipeline and the runnable
[`examples/`](../examples/) build the same artifacts you will consume here.

You need:

- A resolved snapshot bundle (see [The deployment bundle](#the-deployment-bundle)).
- For Pattern 2 only: the `configflux-runtime` executable shipped with your
  deployment, reachable on a known path.
- A JSON library in your service language (every language has one).

## The deployment bundle

A resolved configuration is **not a single file**. Whatever transport
delivers configuration to a service — a container image layer, a mounted
volume, a file synced to a disk, an artifact produced in a pipeline — **must
deliver the bundle as one unit**. The bundle is:

| Artifact | Purpose |
|----------|---------|
| `resolve_result.<root>.<selection>.json` | The resolved snapshot: every parameter's final value and metadata for one scope. This is what a Pattern 1 consumer reads directly. |
| `ccm/` (a directory) | The solver model: `ccm.manifest.json`, `ccm.symbols.json`, and one or more `partition-*/ccm.bdd.bin`. Required by Pattern 2's validated session. |
| Hash lineage | The `model_hash` and `resolve_hash` carried inside the snapshot. They identify exactly which compiled model and which resolution this bundle represents. |

This layout — the per-scope snapshot at the bundle root plus a sibling `ccm/`
directory, with the hash lineage carried inside the snapshot — is **the
standard delivery unit**. Whatever moves configuration to a target (a container
image layer, a mounted volume, a synced directory, a pipeline artifact) moves
exactly this layout, as one unit. There is no separate bundle descriptor file:
the identities that bind the unit together already live in the files
themselves — `model_hash` and `resolve_hash` inside the snapshot, and
`bound_model_hash` inside `ccm/ccm.manifest.json`.

### Verifying a bundle before you ship it

Because the snapshot and the `ccm/` are bound only by their hashes, an
assembly mistake — pairing a snapshot with the wrong `ccm/`, or shipping a
snapshot with no `ccm/` at all — is possible. The reference script
[`examples/verify_bundle.sh`](../examples/verify_bundle.sh) is the maintained
way to cross-check that a bundle is a matched, complete unit before it is
shipped, baked into an image, or mounted. Point it at a bundle directory:

```bash
examples/verify_bundle.sh <bundle-dir> [expected-snapshot-content-hash]
```

It verifies that the snapshot's `model_hash` equals the `ccm/`'s
`bound_model_hash` (a mismatch means the snapshot and the solver model do not
belong together — a mis-assembled bundle), and it recomputes and prints the
snapshot's content hash (asserting it against an expected value if you supply
one). It exits `0` on a matched bundle and non-zero, with a diagnostic, when
the `ccm/` is missing, the hashes disagree, or the snapshot or `ccm/` manifest
cannot be read. It reads only the two already-produced JSON files and needs no
ConfigFlux binary.

This cross-check is an **optional pre-assembly convenience**. It lets a
pipeline catch a mis-assembled bundle *earlier* — before delivery — but it is
not what makes a bundle safe on the target. The on-target guarantee is the
fail-closed runtime open described next: a validated session refuses to start
without a usable `ccm/`, and that holds whether or not this script was ever
run.

### Fail-closed rule (the headline operational fact)

> **A validated runtime session requires a usable `ccm/` solver model
> alongside the snapshot. If it is missing, unloadable, or a symbol-less
> stub, opening the session fails closed — it does not silently degrade.**

When you open a runtime session (Pattern 2), the open request carries a
`ccm_ref` pointing at the bundle's `ccm/` directory. If that reference is
empty or does not resolve to a loadable solver model, the open is **rejected**
with status `error`, exit code `2`, and diagnostic code
`E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`. There is no degraded mode that
accepts a snapshot without its solver model. This is why the bundle must ship
together: a snapshot delivered without its `ccm/` directory cannot start a
validated session.

(Pattern 1, the read-only path, does not open a session and so does not
require `ccm/` at read time — but you should still distribute the bundle whole
so the same artifacts support both patterns and so the hash lineage stays
intact.)

### One scope per service

A resolved snapshot is **per-scope**. The `scope` field tells you what it
covers:

- `component:<name>` — a single component's resolved configuration. A service
  in a multi-service stack consumes **only its own component scope**.
- `all` — the entire model. A monolith consumes `scope = all`.

The **same mechanism** serves both. A multi-service stack simply produces one
per-component snapshot per service; a monolith produces one `scope = all`
snapshot. Nothing about the consumption code changes between the two — only
which file the service reads.

### Multiple resolve targets from one model

A single compiled model can produce **several** resolved snapshots — for
different contexts or selections (for example a developer-local target and a
production target). They are distinguished by the `<selection>` suffix in the
file name. A service started standalone for debugging just points at whichever
bundle it wants; there is no special "debug mode" — you select a target by
choosing which bundle the service reads.

#### Naming targets: the environment-manifest convention

A resolution target is fully described by three values: a **scope**
(`component:<name>` or `all`), a set of **context tags**, and a set of
selection **choices**. Typing those into the resolve pipeline by hand for every
target is error-prone and leaves nothing under version control. The
**environment-manifest convention** gives each target a name, so "resolve the
production target" means the same resolution every time.

An environment manifest is a small JSON file **you own and version-control** in
your own repository. The convention fixes the field names; the shape is:

```json
{
  "schema_version": 1,
  "environments": {
    "local": {
      "scope": "component:runtime_tuner",
      "context_tags": { "region": "eu", "site": "lab" },
      "choices": { "device_class": "gateway", "update_channel": "canary" }
    },
    "production": {
      "scope": "component:runtime_tuner",
      "context_tags": { "site": "prod" },
      "choices": { "device_class": "gateway", "update_channel": "canary", "region": "eu" }
    }
  }
}
```

Each environment names a target: the `scope` it resolves, the `context_tags`
that seed the resolution, and the `choices` (one per selectable facet) that
drive it. A worked example manifest ships at
[`examples/environments.example.json`](../examples/environments.example.json).

> **This is a documented convention, not a product input format.** The
> ConfigFlux binaries do **not** read the manifest file. There is no new CLI
> verb and no new input parser. The manifest exists only to give targets names
> under version control; each named entry is exactly the three inputs
> `cfx resolve` already takes (scope + context tags + choices).

**Produce one target with `cfx resolve`.** `cfx resolve` is the one-shot
resolver: it opens the compiled model, applies a selection, resolves, and writes
the snapshot in a single command — no request envelopes to thread by hand. A
target is described by a small **selection file** (its scope, context tags, and
choices):

```bash
cat > production.selection.json <<'JSON'
{
  "schema_version": 2,
  "model_hash": "",
  "scope": "component:runtime_tuner",
  "context_tags": { "site": "prod" },
  "choices": { "device_class": "gateway", "update_channel": "canary", "region": "eu" },
  "selection_state_hash": ""
}
JSON

cfx resolve \
  --model <cmp.manifest.json> \
  --selection-file production.selection.json \
  --out ./targets/production
```

`cfx` re-derives `model_hash` and `selection_state_hash` itself, so those two
fields are left empty in the file. Choices can also be given as repeatable
`--select facet=option` flags instead of (or on top of) the file. Resolution is
deterministic: resolving the same target twice yields a byte-identical snapshot
and an identical `resolve_hash`. `cfx` exits `0` on success, `2` on a usage/IO
error, and `3` on an unsatisfiable selection (run `cfx explain` to see why).

**Reference resolver (the envelope appendix).** When you prefer to drive
resolution from a manifest of named environments — or you are a machine
integrator wiring the raw request/response **envelope** protocol (the interpreter
`open → init-selection-state → select → resolve` chain, the agent/Depth-2 seam
per ADR-0042) — the reference resolver
[`examples/resolve_environment.sh`](../examples/resolve_environment.sh) reads a
manifest and unfolds one named environment into that envelope chain:

```bash
examples/resolve_environment.sh \
  --cmp <cmp.manifest.json> \
  --manifest environments.example.json \
  --environment production \
  --out ./targets/production
```

It locates the interpreter via `CONFIGFLUX_INTERPRETER` (falling back to the
local build), reads `scope`, `context_tags`, and `choices` for the named
environment, drives the envelope chain, and writes one
`resolve_result.<root>.<selection>.json` snapshot that is **bit-for-bit
identical** to the `cfx resolve` snapshot for the same target. The script exits
`0` on success, `1` if a resolution is rejected or the manifest is malformed,
and `2` on a usage error.

#### Resolving a matrix of targets in CI

A release pipeline usually resolves **many** targets from one model — several
environments, and within each, one or more service scopes. This is a
**matrix**: N environments × M scopes, where every cell is an independent,
deterministic resolution producing one snapshot. Because the cells are
independent, the matrix is simply a **loop** of `cfx resolve` calls — one per
(environment, scope) cell — with no batch verb and no shared cross-cell state.
[`examples/05-compose-fleet`](../examples/05-compose-fleet) shows exactly this
loop end to end.

The reference resolver also runs the matrix directly through the envelope chain
(the appendix path), which is handy when you already drive it from a manifest:

```bash
# Resolve every environment in the manifest, each across two service scopes.
# Produces one snapshot per (environment, scope) cell under ./targets/.
examples/resolve_environment.sh \
  --cmp <cmp.manifest.json> \
  --manifest environments.example.json \
  --matrix \
  --scopes "component:runtime_tuner,component:update_agent" \
  --out ./targets
```

Each cell is written under `./targets/<environment>/<scope-root>/` as its own
`resolve_result.<root>.<selection>.json`, so the cells never collide. With
`--scopes` omitted, each environment is resolved at its own declared `scope`.
Because every cell is deterministic, the whole matrix is reproducible: a CI
stage can re-run it and compare against committed snapshots, and a per-target
delivery bundle (snapshot + `ccm/`) is assembled from each cell and verified
with [`examples/verify_bundle.sh`](../examples/verify_bundle.sh) before it is
shipped. This is the CI-suitable pattern for building every target's
configuration in one pass — a documented loop over the existing pipeline, not a
new product feature.

## Pattern 1 — Read-only startup config

This is the simplest pattern and needs **no ConfigFlux binary** in the service
runtime. The service reads its resolved snapshot once at startup and treats it
as immutable configuration.

### The resolved snapshot shape

The snapshot has these top-level keys:

| Key | Meaning |
|-----|---------|
| `schema_version` | Envelope version. |
| `status` | `ok` on a clean resolve. |
| `scope` | What this snapshot covers (`component:<name>` or `all`). |
| `model_hash` | Identity of the compiled model (invariant). |
| `resolve_hash` | Identity of this resolution (model + selection + output). |
| `resolved_output` | The resolved parameter tree (see below). |
| `selection_state_hash`, `choices`, `resolved_artifacts`, `resolved_component_dependencies` | Selection and dependency provenance. |
| `error_count`, `warning_count`, `diagnostics` | Resolution diagnostics. |

`resolved_output` is a **nested** map, not a flat list. Its shape is:

```
resolved_output
└── <scope_root>
    ├── package, version
    └── components
        └── <component>
            ├── type
            └── params
                └── <param> → { value, type, unit, safety, lifecycle, access, req_id, doc, limits }
```

A real excerpt:

```json
{
  "runtime_tuner": {
    "package": "merged_root",
    "version": "0.0.0",
    "components": {
      "runtime_tuner": {
        "type": "controller",
        "params": {
          "log_level": {
            "value": "info",
            "type": "string",
            "unit": null,
            "safety": "q_m",
            "lifecycle": "runtime",
            "access": "technician",
            "req_id": null,
            "doc": "Runtime log verbosity (info, debug, trace)",
            "limits": null
          }
        }
      }
    }
  }
}
```

Each parameter leaf carries its own `lifecycle`. A read-only consumer reads
values directly and ignores `lifecycle` if it only reads at startup.

### Verify the lineage and treat the file as immutable

A Pattern 1 consumer should:

1. Read `model_hash` and `resolve_hash` and **pin or verify** them against the
   configuration it expects. These two hashes uniquely identify the
   configuration; if they change, the configuration changed.
2. Treat the file as **immutable**. Do not write back to it. Runtime mutation
   is Pattern 2's job.

### Python

```python
import json

def load_config(path: str) -> dict:
    with open(path, "r", encoding="utf-8") as f:
        snapshot = json.load(f)
    if snapshot["status"] != "ok":
        raise RuntimeError(f"resolve status is {snapshot['status']}")
    return snapshot

def get_value(snapshot: dict, component: str, param: str):
    # scope is "component:<root>" or "all"; the root key of resolved_output
    scope = snapshot["scope"]
    root = scope.split(":", 1)[1] if ":" in scope else scope
    tree = snapshot["resolved_output"][root]
    return tree["components"][component]["params"][param]["value"]

snapshot = load_config("resolve_result.runtime_tuner.canary.json")

# Pin the lineage you were deployed with.
EXPECTED_RESOLVE_HASH = "d3d9ad268b4f3184c10a02f4125a6c94dc5043957b291626a9af1c99665c05db"
assert snapshot["resolve_hash"] == EXPECTED_RESOLVE_HASH, "unexpected configuration"

log_level = get_value(snapshot, "runtime_tuner", "log_level")
print("log level:", log_level)  # "info"
```

### C#

```csharp
using System;
using System.IO;
using System.Text.Json;

string path = "resolve_result.runtime_tuner.canary.json";
using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(path));
JsonElement root = doc.RootElement;

if (root.GetProperty("status").GetString() != "ok")
    throw new InvalidOperationException("resolve status is not ok");

// Pin the lineage you were deployed with.
const string ExpectedResolveHash =
    "d3d9ad268b4f3184c10a02f4125a6c94dc5043957b291626a9af1c99665c05db";
if (root.GetProperty("resolve_hash").GetString() != ExpectedResolveHash)
    throw new InvalidOperationException("unexpected configuration");

string scope = root.GetProperty("scope").GetString()!;
string scopeRoot = scope.Contains(':') ? scope.Split(':', 2)[1] : scope;

JsonElement logLevel = root
    .GetProperty("resolved_output")
    .GetProperty(scopeRoot)
    .GetProperty("components")
    .GetProperty("runtime_tuner")
    .GetProperty("params")
    .GetProperty("log_level")
    .GetProperty("value");

Console.WriteLine($"log level: {logLevel.GetString()}");  // "info"
```

A Go, Java, or Node service does the same: parse the JSON, check `status`,
verify `resolve_hash`, and read `resolved_output[<root>].components.<c>.params.<p>.value`.

## Pattern 2 — Validated session via the runtime CLI

When a service needs to **read live parameter metadata** or **mutate
runtime-lifecycle parameters** with full domain validation, it drives the
`configflux-runtime` executable as a subprocess. This pattern requires the
`ccm/` solver model in the bundle (see the [fail-closed rule](#fail-closed-rule-the-headline-operational-fact)).

### Transport contract

- **Default mode**: one JSON request object on **stdin**, one JSON response
  object (plus a trailing newline) on **stdout**.
- **File mode**: pass `--request-file <path>` and `--response-file <path>`
  instead. Useful when wiring the binary into a script or when stdin/stdout
  are otherwise occupied.
- **Request size** is bounded to **8 MiB**. A larger request is a transport
  failure.
- Every request and response carries `schema_version`, set to `1`.

**Exit codes** (the same for every command):

| Exit | Meaning |
|------|---------|
| `0` | Response `status` is `ok`. |
| `2` | Response `status` is `error` — a domain or runtime rejection (the response JSON still parses and carries diagnostics). |
| `1` | Transport / CLI failure — bad arguments, an I/O error, malformed JSON, or an oversized request. Read stderr, not stdout. |

Treat exit `2` as "the request was understood and refused" (inspect
`diagnostics`) and exit `1` as "the request never ran" (the binary or the
envelope was wrong).

### The session flow

1. **`runtime-open`** — hand the binary the resolved snapshot plus a `ccm_ref`
   pointing at the bundle's `ccm/` directory. The response returns a
   `runtime_snapshot` object. **Thread that object into every subsequent
   request** — it carries the open session's state (including pending dirty
   changes).
2. **Read** — `get-scope-metadata` (component/parameter/artifact counts),
   `list-parameters` (sorted parameter paths), `get-parameter` (one
   parameter's value and metadata).
3. **Write** — `set-parameter` mutates a single runtime-lifecycle parameter,
   producing a *pending dirty change*. Then either `commit-configuration`
   (persist) or `rollback-dirty` (discard). `list-dirty-parameters` and
   `get-dirty-metadata` inspect what is pending.

The required response envelope fields are `schema_version`, `status`,
`model_hash`, `resolve_hash`, `error_count`, `warning_count`, and
`diagnostics`. The `model_hash` and `resolve_hash` are **preserved unchanged**
across open, every read, and every write — a service can audit that its
runtime state is still bound to the configuration it opened.

### The open request

The open request projects the resolved snapshot and adds the `ccm_ref`:

```json
{
  "schema_version": 1,
  "model_hash": "<from snapshot>",
  "resolve_hash": "<from snapshot>",
  "ccm_ref": "<path to the bundle's ccm/ directory>",
  "scope": "component:runtime_tuner",
  "resolved_output": { "...": "from snapshot" },
  "resolved_component_dependencies": {},
  "resolved_artifacts": {},
  "context_tags": {},
  "choices": {}
}
```

A successful open returns (abbreviated):

```json
{
  "schema_version": 1,
  "status": "ok",
  "scope": "component:runtime_tuner",
  "model_hash": "efab1360380efc62...",
  "resolve_hash": "d3d9ad268b4f3184...",
  "runtime_snapshot": { "ccm_ref": "<path>/ccm", "...": "..." },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": { "...": "..." }
}
```

### Dirty / commit / rollback

A `set-parameter` on a runtime-lifecycle parameter creates a **pending dirty
change** in the returned `runtime_snapshot`; it is **not yet committed**. The
service must then:

- **`commit-configuration`** — persist the pending change(s) as a new
  configuration version. The response returns a `commit_id`, a
  `base_configuration_id`, a `target_configuration_id`, and the
  `changed_paths` delta.
- **`rollback-dirty`** — discard pending changes. Use `"mode": "all"` to
  discard everything, or `"mode": "subset"` with a `paths` list to discard
  specific ones.

There is an **auto-reset policy** (enabled by default, with a default timeout
on the order of tens of seconds) that reverts *uncommitted* dirty changes when
the timer expires. A service that wants a change to stick must
**`commit-configuration` within that window**, or the change is automatically
reset. `list-dirty-parameters` shows what is currently pending.

### Python

```python
import json
import subprocess

RUNTIME = "/opt/configflux/configflux-runtime"  # the shipped executable

def call(command: str, request: dict) -> dict:
    """Drive one configflux-runtime command over stdin/stdout."""
    proc = subprocess.run(
        [RUNTIME, command],
        input=json.dumps(request).encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if proc.returncode == 1:
        # Transport failure: the request never ran. Detail is on stderr.
        raise RuntimeError(f"{command} transport failure: {proc.stderr.decode().strip()}")
    response = json.loads(proc.stdout)  # parses on exit 0 and exit 2
    return response  # caller inspects status / diagnostics

# 1. Open the session from the bundle.
open_request = {
    "schema_version": 1,
    "model_hash": snapshot["model_hash"],
    "resolve_hash": snapshot["resolve_hash"],
    "ccm_ref": "/opt/configflux/bundle/ccm",
    "scope": snapshot["scope"],
    "resolved_output": snapshot["resolved_output"],
    "resolved_component_dependencies": snapshot.get("resolved_component_dependencies", {}),
    "resolved_artifacts": snapshot.get("resolved_artifacts", {}),
    "context_tags": snapshot.get("context_tags", {}),
    "choices": snapshot.get("choices", {}),
}
opened = call("runtime-open", open_request)
if opened["status"] != "ok":
    code = opened["diagnostics"]["diagnostics"][0]["code"]
    raise RuntimeError(f"runtime-open rejected: {code}")
session = opened["runtime_snapshot"]  # thread this into every later call

# 2. Read parameter paths.
listed = call("list-parameters", {
    "schema_version": 1, "runtime_snapshot": session, "scope_root": "runtime_tuner",
})
print("parameters:", listed["parameter_paths"])

# 3. Mutate a runtime-lifecycle parameter -> pending dirty change.
result = call("set-parameter", {
    "schema_version": 1, "runtime_snapshot": session,
    "path": "component.runtime_tuner.param.log_level", "value": "debug",
})
if result["status"] != "ok":
    code = result["diagnostics"]["diagnostics"][0]["code"]
    raise RuntimeError(f"set-parameter rejected: {code}")
session = result["runtime_snapshot"]  # carries the pending change

# 4. Commit within the auto-reset window, or the change is reverted.
committed = call("commit-configuration", {
    "schema_version": 1, "runtime_snapshot": session,
    "actor": "inventory-service", "reason": "persist log level",
})
print("commit_id:", committed["commit_id"])
```

### C#

```csharp
using System;
using System.Diagnostics;
using System.Text;
using System.Text.Json;

const string Runtime = "/opt/configflux/configflux-runtime";

static JsonElement Call(string command, string requestJson)
{
    var psi = new ProcessStartInfo(Runtime, command)
    {
        RedirectStandardInput = true,
        RedirectStandardOutput = true,
        RedirectStandardError = true,
    };
    using var proc = Process.Start(psi)!;
    proc.StandardInput.Write(requestJson);
    proc.StandardInput.Close();

    string stdout = proc.StandardOutput.ReadToEnd();
    string stderr = proc.StandardError.ReadToEnd();
    proc.WaitForExit();

    if (proc.ExitCode == 1)  // transport failure: the request never ran
        throw new InvalidOperationException($"{command} transport failure: {stderr.Trim()}");

    return JsonDocument.Parse(stdout).RootElement;  // parses on exit 0 and exit 2
}

static string FirstDiagnosticCode(JsonElement response) =>
    response.GetProperty("diagnostics").GetProperty("diagnostics")[0]
            .GetProperty("code").GetString()!;

// 1. Open the session (openRequestJson built from the bundle snapshot + ccm_ref).
JsonElement opened = Call("runtime-open", openRequestJson);
if (opened.GetProperty("status").GetString() != "ok")
    throw new InvalidOperationException($"runtime-open rejected: {FirstDiagnosticCode(opened)}");
string session = opened.GetProperty("runtime_snapshot").GetRawText();  // thread into later calls

// 2. Mutate a runtime-lifecycle parameter -> pending dirty change.
string setReq = $$"""
{
  "schema_version": 1,
  "runtime_snapshot": {{session}},
  "path": "component.runtime_tuner.param.log_level",
  "value": "debug"
}
""";
JsonElement setResult = Call("set-parameter", setReq);
if (setResult.GetProperty("status").GetString() != "ok")
    throw new InvalidOperationException($"set-parameter rejected: {FirstDiagnosticCode(setResult)}");
session = setResult.GetProperty("runtime_snapshot").GetRawText();

// 3. Commit within the auto-reset window.
string commitReq = $$"""
{ "schema_version": 1, "runtime_snapshot": {{session}},
  "actor": "inventory-service", "reason": "persist log level" }
""";
JsonElement committed = Call("commit-configuration", commitReq);
Console.WriteLine($"commit_id: {committed.GetProperty("commit_id").GetString()}");
```

Both snippets transpose directly to Go (`os/exec`), Java
(`ProcessBuilder`), or Node (`child_process.spawn`): spawn the executable with
one subcommand argument, write one JSON object to stdin, read one JSON object
from stdout, branch on the exit code, and thread `runtime_snapshot` from each
response into the next request.

## Lifecycle semantics for consumers

Every parameter has a `lifecycle` class. There are three, serialized
lowercase:

| `lifecycle` | Meaning for the running service | Writable at runtime? |
|-------------|---------------------------------|----------------------|
| `construction` | Baked in at build/compile time; fixed for the life of this configuration. | No |
| `startup` | Read once at process start; fixed thereafter while the process runs. | No |
| `runtime` | Mutable live, via `set-parameter`. | Yes |

**Read paths expose all parameters** regardless of lifecycle —
`get-parameter`, `list-parameters`, and the Pattern 1 snapshot all show
`construction`, `startup`, and `runtime` parameters alike.

**`set-parameter` mutates only `runtime` parameters.** Attempting to write a
`construction` or `startup` parameter is rejected with status `error`, exit
code `2`, and diagnostic code `E_RUNTIME_LIFECYCLE_IMMUTABLE`. The message
names the path and its lifecycle, with the hint *"Only lifecycle=runtime
parameters are writable"*. A service should read `lifecycle` from the snapshot
and only offer live editing for `runtime` parameters.

## Pattern 3 — C ABI (advanced)

A service can also call ConfigFlux runtime-core in-process over a C ABI,
avoiding the subprocess round-trip. The ABI surface — version handshake,
session open/execute/close, ownership rules, and operation codes — is
documented in [the runtime C ABI reference](runtime-c-abi.md).

**Native bindings are under evaluation and are not shipped today.** Using the
ABI from a non-C/C++ language requires a dynamically linkable build of the
runtime that is not part of the current distribution, so this guide does
**not** provide ready-to-run `ctypes` or P/Invoke clients. The same open-time
fail-closed solver-model precondition described above applies to the ABI open
entrypoint. **Prefer Pattern 1 or Pattern 2 today**; revisit Pattern 3 once
native bindings ship.

## Worked scenarios (coverage)

The same two patterns cover very different deployment shapes — only the
*delivery* of the bundle changes, never the consumption code:

- **Containerized multi-environment stacks.** Bake the bundle into the image
  at build time, or mount it alongside the container at run time. Each service
  reads its own `component:<name>` snapshot; a per-environment build simply
  selects a different resolve target (a different `<selection>` bundle).
  Pattern 1 covers startup config; Pattern 2 covers live tuning. The `ccm/`
  directory must travel inside the image layer or the mounted volume.

- **Bare-metal and VM services.** Place the bundle on disk next to the service
  binary. A read-only service points Pattern 1 at the snapshot file; a service
  that tunes parameters live points Pattern 2 at the same bundle and at the
  local `configflux-runtime` executable.

- **CI pipelines.** A pipeline stage produces the bundle (compile → resolve)
  and a later stage consumes it — for example asserting a `resolve_hash`
  matches an expected value, or reading `resolved_output` to drive a
  deployment step. This is Pattern 1 reading a freshly produced bundle; no
  long-running process is involved.

In every case it is the *same* mechanism: a per-scope resolved snapshot, its
`ccm/` solver model, and the hash lineage, delivered as one unit.

### Materializing an env-file or compose override (user-side)

Some orchestrators want configuration in a shape other than the snapshot JSON —
an env-file of `KEY=value` lines, a `docker-compose` override fragment, an
orchestrator-specific config file. **The product does not emit those shapes.**
It emits the resolved snapshot (the richer artifact: it carries *every*
parameter and its full metadata, across all lifecycle classes); shaping that
snapshot into whatever an orchestrator consumes is a **thin, user-side
transform** that lives in your repository, not a product output. Generating a
compose override or an orchestrator config is therefore your code's job — a few
lines that read `resolved_output` and write the shape you need.

A snapshot-to-env-file transform is just a walk over `resolved_output`,
emitting one `KEY=value` line per parameter. In Python:

```python
import json

def env_lines(snapshot: dict) -> list[str]:
    # scope is "component:<root>" or "all"; the root key of resolved_output
    scope = snapshot["scope"]
    root = scope.split(":", 1)[1] if ":" in scope else scope
    lines = []
    for component, cdef in snapshot["resolved_output"][root]["components"].items():
        for param, leaf in cdef["params"].items():
            key = f"{component}_{param}".upper()
            lines.append(f"{key}={leaf['value']}")
    return sorted(lines)

with open("resolve_result.runtime_tuner.canary.json", encoding="utf-8") as f:
    snapshot = json.load(f)

with open(".env", "w", encoding="utf-8") as out:
    out.write("\n".join(env_lines(snapshot)) + "\n")
# RUNTIME_TUNER_LOG_LEVEL=info
```

The same walk is a one-liner with `jq` (handy in a shell pipeline):

```bash
root=$(jq -r '.scope | sub("^component:";"")' resolve_result.runtime_tuner.canary.json)
jq -r --arg root "$root" '
  .resolved_output[$root].components
  | to_entries[] as $c
  | $c.value.params | to_entries[]
  | "\($c.key)_\(.key | ascii_upcase)=\(.value.value)"
' resolve_result.runtime_tuner.canary.json | sort > .env
```

To produce a `docker-compose` override instead, the same loop writes the
values under a service's `environment:` (or `env_file:`) key rather than to a
flat file — again entirely in your transform. The product's contribution stops
at the snapshot; where the override or orchestrator config is generated, and in
what dialect, stays with the consumer who owns the orchestrator. Pin or verify
`model_hash` / `resolve_hash` first (as in Pattern 1) so the file you generate
is traceable to the exact resolution it came from.

## Troubleshooting

### "runtime-open requires a usable .ccm solver model; the snapshot's ccm_ref is empty, unloadable, or carries no symbol table"

Diagnostic code `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`, exit code `2`. The
`ccm_ref` in your open request did not resolve to a usable solver model. The
`ccm/` directory is missing from the bundle, the path is wrong, or the
artifact is a stub without a symbol table. **Ship the bundle whole** — the
snapshot JSON *and* the `ccm/` directory — and point `ccm_ref` at that
directory. There is no degraded open path.

### "Parameter '<path>' is not writable at runtime (lifecycle=...)"

Diagnostic code `E_RUNTIME_LIFECYCLE_IMMUTABLE`, exit code `2`. You called
`set-parameter` on a `construction` or `startup` parameter. Only `runtime`
parameters are writable live. Check the parameter's `lifecycle` in the
snapshot before offering it for editing; `construction` and `startup` values
are fixed for the running service.

### "Request payload from 'stdin' exceeds 8388608 bytes"

Diagnostic code `E_RUNTIME_CLI_REQUEST_TOO_LARGE`, exit code `1`. The request
exceeded the 8 MiB bound. This is a transport failure (the request never ran).
Most often the cause is an unexpectedly large `runtime_snapshot` or
`resolved_output` being threaded into a request; confirm you are passing the
session snapshot, not a duplicated or accidentally nested payload.

### "Malformed JSON request envelope for '<command>' command"

Diagnostic code `E_RUNTIME_CLI_REQUEST_INVALID`, exit code `1`. The bytes on
stdin (or in `--request-file`) were not a valid request envelope. Verify you
serialized a single JSON object, that `schema_version` is `1`, and that field
names match the command (for example the list/metadata commands
take `scope_root`, not `scope`).

### A command exited `2` but I expected `0`

Exit `2` means the request was understood and **refused** — the response JSON
on stdout still parses and carries the reason under `diagnostics`. Read
`diagnostics.diagnostics[].code` and `.message`. This is distinct from exit
`1`, which means the request never ran and the detail is on stderr.

## Related

- [Canonical worked example](canonical-worked-example.md) — the full
  compile → resolve → runtime pipeline (compiler + `cfx resolve`) that produces
  the bundle this guide consumes.
- [Runtime CLI product contract](runtime-cli-contract.md) — the frozen
  request/response envelopes, transport rules, and exit-code behavior for
  `configflux-runtime`.
- [Runtime C ABI reference](runtime-c-abi.md) — the in-process FFI surface
  referenced by Pattern 3.
- [`examples/`](../examples/) — runnable progressive examples, including a
  full runtime-handoff pipeline.
