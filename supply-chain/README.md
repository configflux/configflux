# supply-chain/

This directory is the home for `cargo-vet` state — the attestation trail
that records which reviewer at which organisation has vetted each crate
version that ships in this workspace.

## Status

**Initialised.** `cargo vet init` has been run and the attestation trail
is live. The 10 direct workspace dependencies have locally-authored
`safe-to-deploy` audit entries in `audits.toml`. Remaining transitive
dependencies are covered by exemptions in `config.toml` until third-party
audit imports or future local audits replace them.

`cargo vet check` is enforced in CI as part of the supply-chain audit
step. A failure blocks the build.

## Do not hand-edit

Once `cargo vet init` has run, the three files below are managed by
`cargo vet` itself. Manual edits should be restricted to
`config.toml` `[imports.*]` tables; `audits.toml` rows should be added
via `cargo vet certify`.

- `config.toml` — audit imports, policy, and per-crate criteria.
- `audits.toml` — locally-authored certifications.
- `imports.lock` — pinned hash of each imported third-party audit set.

## References

- Policy: `SECURITY.md` (Dependency Auditing section)
