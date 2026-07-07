# cudd-sys (configflux fork)

In-tree fork of upstream `cudd-sys` providing unsafe Rust FFI bindings
for the CUDD decision-diagram package. Consumed by the `solver/` crate
as a path dep; no other crate is permitted to link `cudd_sys`.

## Provenance

- **Upstream crate:** [`cudd-sys` 1.1.2-alpha.2](https://crates.io/crates/cudd-sys/1.1.2-alpha.2)
- **Upstream repository:** <https://github.com/pclewis/cudd-sys>
- **Upstream license:** CC0-1.0 (public-domain dedication)
- **Upstream `Cargo.toml` checksum (crates.io):**
  `4c6d85ca7d94bdbb2d62ba26cada6adf44c37594f24da20d061a528090306845`
  (sha256 of the published `.crate` tarball, verified at vendor time)

The `src/` directory and `convert-externs.pl` are copied verbatim from
the upstream 1.1.2-alpha.2 release.

## Relicensing rationale

This fork is licensed **MIT** (see `LICENSE` in this directory). CC0-1.0
is a public-domain dedication and explicitly permits any downstream
licence, including a permissive one such as MIT. Relicensing does not
strip credit from the upstream author — the `convert-externs.pl`
script and the manually-curated FFI surface remain credited to
[Philip Lewis](https://github.com/pclewis) and the upstream
contributor set; the MIT copyright line covers configflux's fork
maintenance only.

The relicence is the mechanism that lets the fork clear
`cargo deny check` against this repository's existing `[licenses]
allow` list without adding `CC0-1.0` to the allow list.

## Distribution mechanics

- **No `build.rs`.** The upstream `build.rs` autotools-builds CUDD at
  crate-build time. This fork strips it entirely — Bazel handles
  linking via `//third_party/cudd:cudd` (a hand-authored `cc_library`)
  in the `solver/` crate's `rust_library.deps`. No network or
  autotools step at build time.
- **No `build_cudd` feature.** Upstream's default feature toggled the
  autotools build; this fork has no `[features]` section. Linkage is
  the Bazel graph's responsibility.
- **`default-features = false`** at the consumer site
  (`solver/Cargo.toml`) is therefore a no-op for this fork but is set
  for hygiene.
- **`publish = false`.** This fork is internal to the workspace and
  must never reach a public registry.

## Boundary

The dependency boundary for this fork:

- Only `solver/src/backend_cudd.rs` is permitted to import
  `cudd_sys::*`. Every other consumer of
  decision-diagram functionality goes through the `SolverBackend`
  trait at `solver/src/backend.rs`.
- `compiler/`, `interpreter/`, and `runtime/` do not link
  `cudd-sys`. The fork's MIT licence and CUDD's BSD-3-Clause licence are
  both Apache-2.0-compatible and sit alongside the repository's
  Apache-2.0 licence without conflict.

## Maintenance

When bumping the upstream cudd-sys revision:

1. Re-download the published `.crate` tarball; verify its SHA-256
   against `https://crates.io/api/v1/crates/cudd-sys/<version>` first.
2. Replace `src/` and `convert-externs.pl` verbatim.
3. Re-confirm no new `build_*` feature flags or `build-dependencies`
   appear that would re-introduce a build-time autotools or network
   step. If they do, treat the bump as a new ADR amendment, not a
   maintenance update.
4. Update the upstream version, checksum, and link in this README.

When bumping CUDD itself, see `//third_party/cudd/BUILD.bazel` for
the cc_library wiring and `//third_party/cudd/config.h` for the
hand-authored autotools substitute.
