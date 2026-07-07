# ConfigFlux toolchain container image

The official toolchain image packages the three ConfigFlux command-line tools —
`compiler`, `interpreter`, and `runtime` — together with the exact pinned `cue`
evaluator the project builds against. It lets you run the toolchain **anywhere a
container runtime runs**, on any host where installing native binaries is
inconvenient or unavailable. The image is a run-and-exit toolchain: it reads
your model from a mounted directory, does its work, writes its outputs back, and
exits. It is not a server and does not stay resident.

## Image reference

```
ghcr.io/configflux/toolchain:vX.Y.Z
```

The version tag tracks the public release tag. There is **no `latest` tag** — a
floating tag would let a deploy-time run silently pick up a different toolchain
(and therefore a different `model_hash`) from one day to the next. Use a
versioned tag, and for anything that must resolve identically across machines
and over time, pin by **immutable digest**:

```
ghcr.io/configflux/toolchain@sha256:<digest>
```

A `vX.Y.Z` tag is conventionally stable; `@sha256:<digest>` is the guarantee.
Prefer the digest form for reproducible runs.

## Invocation contract

Pick the tool you want as the **first argument** (all three are on `PATH`
inside the image), mount your working directory at `/work`, and run. On a
native Linux Docker daemon, pass `-u` so emitted files are owned by you:

```bash
docker run --rm \
  -u "$(id -u):$(id -g)" \
  -v "$PWD:/work" \
  -w /work \
  ghcr.io/configflux/toolchain:vX.Y.Z \
  compiler <args...>
```

The same shape runs `interpreter` and `runtime` — just change the first
argument. A bare run with no command prints a usage banner, so the image is
self-documenting:

```bash
docker run --rm ghcr.io/configflux/toolchain:vX.Y.Z
```

Any OCI-compatible runtime works; substitute `podman run` for `docker run`.

### Files are owned by the invoking user — but the flags depend on your runtime

The contract is that **emitted files are owned by you, the invoking user, not
by root** — never a tree of root-owned files you cannot delete without
elevation. *How* you achieve that depends on the container runtime, and this is
the one place a wrong flag turns into a confusing *compile/emit* error rather
than an obvious one: the toolchain simply cannot write under the mount.

| Runtime | What to pass | Why |
|---------|--------------|-----|
| **Native Linux Docker** (rootful) | `-u "$(id -u):$(id -g)"` | In-container root ≠ your host user, so `-u` is what maps emitted files back to you. |
| **Docker Desktop** (macOS/Windows) | **omit `-u`** | The file-sharing layer maps writes from the container's default user back to you; passing `-u <hostuid>` instead makes the mount unwritable. |
| **Rootless Docker / Podman** | **omit `-u`** | A user namespace already maps in-container root to your host user; `-u <hostuid>` lands a uid the mount is not owned by. |
| **SELinux-enforcing host** (any of the above) | add `:z` to the mount — `-v "$PWD:/work:z"` | An unlabeled bind mount is denied to the container; `:z` relabels it as shared. |

The image itself is built to run correctly as an **arbitrary, unprivileged user
id**: it assumes no named account, its bundled tools are world-readable and
executable, and it writes only under `/work` (plus a writable `/tmp`). The
variation above is entirely in the `docker`/`podman` flags, not the image.

> **Trial users:** the `cfx` wrapper shipped in the dogfood package detects your
> runtime and selects these flags automatically (override with `CFX_USER_MODE`
> and `CFX_MOUNT_OPT`). Prefer it over a hand-written `docker run`.

### Everything happens under `/work`

The image is treated as read-only. All inputs are read from, and all outputs
are written under, the bind-mounted `/work`. Mount the directory that holds
your model (and that should receive the results) at `/work`, set `-w /work`, and
pass paths relative to it.

## What the image contains

| Component | Notes |
|-----------|-------|
| `compiler`, `interpreter`, `runtime` | The three release binaries, identical to the bytes shipped in the release tarballs for the same version. |
| `cue` | The exact pinned CUE evaluator the project builds against. Because the CUE version is load-bearing for model semantics, bundling the pinned evaluator means a compile/resolve run inside the image reproduces the same `model_hash` as the host build. |
| `LICENSE`, `NOTICE` | Bundled under `/usr/local/share/configflux/`. |

There is no shell and no package manager — the image is intentionally minimal.
It is built on a `distroless`-class base that carries CA certificates and a
writable temp directory and nothing else.

## Reproducibility

Because the image carries the same pinned `cue` as the rest of the toolchain, a
full compile → resolve run performed inside the container produces output that
is **byte-identical** to the same run performed with the native binaries. The
packaging boundary does not change the result. This is what makes the image
safe for reproducible, deploy-time configuration generation: pin the image by
digest and the toolchain — binaries and evaluator alike — is fixed.

## Verifying authenticity

Published images are signed with cosign keyless signing, the same mechanism the
binary release uses. Verify a digest-pinned image with the project's signing
identity:

```bash
cosign verify \
  --certificate-identity-regexp 'https://github.com/configflux/configflux/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  ghcr.io/configflux/toolchain@sha256:<digest>
```
