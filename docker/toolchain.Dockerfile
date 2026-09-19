# docker/toolchain.Dockerfile
#
# Official ConfigFlux toolchain image (ADR-0033). ONE image carrying all four
# release CLIs (compiler, interpreter, runtime, cfx) + the repo-pinned `cue` binary +
# LICENSE/NOTICE. No shell, no package manager, no interactive userland.
#
# Build context: a staging directory assembled by tools/build_image.sh, NOT the
# repo root. tools/build_image.sh:
#   * resolves the already-built static-musl payload
#       dist/configflux-v<VERSION>-x86_64-linux/{compiler,interpreter,runtime,cfx,
#       LICENSE,NOTICE}
#     (the exact layout tools/build_release.sh emits — no second compile here,
#      per ADR-0033 §4), and
#   * fetches + sha256-verifies the bzl-pinned `cue` (tools/cue_toolchain.bzl,
#     via tools/lib/cue_pin.sh) into the staging dir as `cue`.
# So every COPY below resolves against that staging dir.
#
# Base: gcr.io/distroless/static-debian12, pinned by digest. The four CLIs are
# static-musl and `cue` is a self-contained Go binary, so the runtime layer
# needs no libc, no dynamic loader, and no OS packages (ADR-0033 §1). The
# distroless-static base still provides CA certificates (/etc/ssl/certs) and a
# writable, sticky /tmp — the two things a run-and-exit resolve needs beyond the
# binaries themselves.
#
# Arbitrary-UID contract (ADR-0033 §2, load-bearing): the documented invocation
# ALWAYS passes `-u "$(id -u):$(id -g)"`, so emitted files are owned by the
# invoking user, not root. Nothing here assumes a named user in /etc/passwd; the
# bundled assets are world-readable/executable; /tmp is world-writable+sticky;
# and all IO is under the bind-mounted /work. No `USER` directive ties the image
# to one uid — the caller's `-u` decides.
#
# Pinned by index digest so `docker build` selects linux/amd64 reproducibly.
# Resolved 2026-09-10 from gcr.io/distroless/static-debian12:latest.
FROM gcr.io/distroless/static-debian12@sha256:d75cdd72874d4790092fcb1b058493ecf6bb5bf2b2b897045b00ff01d91843f2

# Provenance labels (OCI). The image tag tracks the public release tag; consumers
# should still pin by @sha256 digest for reproducible deploy-time runs
# (ADR-0033 §3).
ARG CONFIGFLUX_VERSION=0.0.0-dev
ARG CUE_VERSION=unknown
# Declared so `--build-arg SOURCE_DATE_EPOCH=…` is consumed without a warning;
# buildx uses it to make layer timestamps reproducible.
ARG SOURCE_DATE_EPOCH=0
LABEL org.opencontainers.image.title="configflux-toolchain" \
      org.opencontainers.image.description="ConfigFlux toolchain (compiler, interpreter, runtime, cfx) + pinned cue. Run-and-exit against a mounted /work." \
      org.opencontainers.image.source="https://github.com/configflux/configflux" \
      org.opencontainers.image.version="${CONFIGFLUX_VERSION}" \
      org.opencontainers.image.licenses="BUSL-1.1" \
      io.configflux.cue.version="${CUE_VERSION}"

# All five binaries land on PATH (/usr/local/bin is on the distroless default
# PATH) so the user selects the tool as the FIRST argument (the container's
# command): `... <image> compiler <args>`. --chmod=0755 makes them world
# readable+executable so an arbitrary, passwd-less UID can exec them.
COPY --chmod=0755 compiler     /usr/local/bin/compiler
COPY --chmod=0755 interpreter  /usr/local/bin/interpreter
COPY --chmod=0755 runtime      /usr/local/bin/runtime
COPY --chmod=0755 cfx          /usr/local/bin/cfx
COPY --chmod=0755 cue          /usr/local/bin/cue

# Bundled license + notice (world-readable).
COPY --chmod=0644 LICENSE  /usr/local/share/configflux/LICENSE
COPY --chmod=0644 NOTICE   /usr/local/share/configflux/NOTICE

# The contract: mount your model at /work and run. All inputs are read from and
# all outputs written under the bind-mounted /work (ADR-0033 §2).
WORKDIR /work

# ENTRYPOINT is NOT hard-bound to one CLI. CMD defaults to a self-documenting
# usage banner so a bare `docker run <image>` explains itself rather than
# failing. Because distroless has no shell, the banner is produced by one of the
# bundled binaries' own `--help` (compiler), which exec's nothing else and exits
# cleanly. Supplying a command (e.g. `compiler compile ...`) overrides CMD and
# runs the selected tool.
CMD ["compiler", "--help"]
