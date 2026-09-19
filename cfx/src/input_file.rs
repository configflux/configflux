// SPDX-License-Identifier: BUSL-1.1
//
// Bounded reads for the two operator-supplied input files — `--selection-file`
// and `--manifest` (configflux-c16x).
//
// Both flags name a path the operator hands in, and both were read with a bare
// `std::fs::read`: no file type was settled and no bound applied, so either
// flag pointed at a character device read until the process address space was
// exhausted, and either pointed at a FIFO blocked in `open(2)` and then
// buffered whatever arrived.
//
// The shape is the interpreter's `read_request_file` (configflux-mtmi), and the
// bound is the SAME 8 MiB the interpreter and the runtime apply to a request
// payload, so the three surfaces agree about how large an input file may be.
// A `stat()` size bounds what a read returns for a regular file and for nothing
// else, so the file type is settled BEFORE the path is opened; the read is
// capped as well, so neither check stands alone.
//
// `cfx` adds no diagnostic namespace of its own (ADR-0042 §3), so a refusal is
// the exit-2 usage error these paths already reported, and the io wording is
// the one they already printed.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::pipeline::PipelineError;

/// The largest input file either flag accepts, matching the interpreter and
/// runtime request bound. Not configurable: an operator-supplied selection or
/// manifest is a small hand-written document, and a bound that can be raised
/// per invocation is not a bound.
pub(crate) const INPUT_FILE_SIZE_LIMIT_BYTES: usize = 8 * 1024 * 1024;

/// Read `path` for `flag` — the flag's own spelling, which every message names.
pub(crate) fn read_input_file(path: &Path, flag: &str) -> Result<Vec<u8>, PipelineError> {
    let metadata = fs::metadata(path).map_err(|err| io_failure(path, flag, &err))?;

    if !metadata.file_type().is_file() {
        return Err(PipelineError::usage(format!(
            "unable to read {flag} '{}' (not a regular file)",
            path.display()
        )));
    }

    if metadata.len() > INPUT_FILE_SIZE_LIMIT_BYTES as u64 {
        return Err(too_large(path, flag));
    }

    let payload = fs::File::open(path)
        .and_then(read_capped)
        .map_err(|err| io_failure(path, flag, &err))?;

    if payload.len() > INPUT_FILE_SIZE_LIMIT_BYTES {
        return Err(too_large(path, flag));
    }

    Ok(payload)
}

/// Read at most one byte past the bound. Stopping one byte PAST it is what
/// lets the caller tell a file sitting exactly on the bound from one over it.
fn read_capped(file: fs::File) -> std::io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    file.take((INPUT_FILE_SIZE_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut payload)?;
    Ok(payload)
}

/// The wording both call sites already printed for an unreadable path, kept
/// verbatim so an existing missing-file or unreadable-file message is unchanged.
fn io_failure(path: &Path, flag: &str, err: &std::io::Error) -> PipelineError {
    PipelineError::usage(format!(
        "unable to read {flag} '{}' ({err})",
        path.display()
    ))
}

fn too_large(path: &Path, flag: &str) -> PipelineError {
    PipelineError::usage(format!(
        "{flag} '{}' exceeds {} bytes",
        path.display(),
        INPUT_FILE_SIZE_LIMIT_BYTES
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{compile_fixture, run_args, ModelFixture, HERO_COMPONENTS, HERO_DEFS};
    use crate::EXIT_USAGE;

    use std::sync::mpsc;
    use std::time::Duration;

    // These cases live beside the reader rather than in `tests.rs` or
    // `selection_input_tests.rs`: the first is at its grandfathered line cap
    // and the second is 16 lines under the default, so either would have to buy
    // a cap increase to hold them. They still drive the REAL argument surface
    // through `run_args` — a bound that holds in this module and never reaches
    // a verb would be no bound at all.

    /// A path whose `stat()` size says nothing about what a read delivers.
    const CHARACTER_DEVICE: &str = "/dev/zero";

    /// A regular file of `len` bytes that occupies no disk. The bound is about
    /// what a read would deliver, not about what was written, so a sparse file
    /// exercises it without moving 8 MiB.
    fn sparse_file(fixture: &ModelFixture, name: &str, len: u64) -> String {
        let path = fixture.dir.join(name);
        let file = std::fs::File::create(&path).expect("create sparse file");
        file.set_len(len).expect("set sparse length");
        path.to_string_lossy().into_owned()
    }

    /// Run `cfx` on another thread and FAIL if it has not returned in 5 s.
    ///
    /// A refusal that arrives only after the whole stream has been read is
    /// indistinguishable from an absent one on a path that never ends, so the
    /// deadline is part of the assertion rather than a convenience: a timeout
    /// here is a failure, not a skip.
    fn cfx_within_5s(args: &[&str]) -> (u8, String, String) {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
            let _ = tx.send(run_args(&borrowed));
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("cfx did not return within 5s: the input file was read unbounded")
    }

    /// `mkfifo(1)` or `None` when the platform has no such command.
    fn make_fifo(fixture: &ModelFixture, name: &str) -> Option<String> {
        let path = fixture.dir.join(name);
        let status = std::process::Command::new("mkfifo").arg(&path).status().ok()?;
        status
            .success()
            .then(|| path.to_string_lossy().into_owned())
    }

    #[test]
    fn selection_file_that_is_a_character_device_is_refused() {
        // The reproduction from the report: `/dev/zero` reports a size of 0 and
        // delivers bytes forever, so nothing but the file type can refuse it.
        if !std::path::Path::new(CHARACTER_DEVICE).exists() {
            return;
        }
        let fixture = compile_fixture("c16x-devzero-selection", HERO_DEFS, HERO_COMPONENTS);
        let (code, _stdout, err) = cfx_within_5s(&[
            "cfx",
            "options",
            "--model",
            &fixture.manifest,
            "--selection-file",
            CHARACTER_DEVICE,
        ]);

        assert_eq!(code, EXIT_USAGE, "a character device must exit 2: {err}");
        assert!(
            err.contains("not a regular file"),
            "stderr must name the cause: {err}"
        );
    }

    #[test]
    fn manifest_that_is_a_character_device_is_refused() {
        // Same path, other flag. Both are operator-supplied, so a bound on one
        // of them is a bound on neither.
        if !std::path::Path::new(CHARACTER_DEVICE).exists() {
            return;
        }
        let fixture = compile_fixture("c16x-devzero-manifest", HERO_DEFS, HERO_COMPONENTS);
        let out = fixture.out("devzero_out").to_string_lossy().into_owned();
        let (code, _stdout, err) = cfx_within_5s(&[
            "cfx",
            "resolve",
            "--model",
            &fixture.manifest,
            "--manifest",
            CHARACTER_DEVICE,
            "--all",
            "--out",
            &out,
        ]);

        assert_eq!(code, EXIT_USAGE, "a character device must exit 2: {err}");
        assert!(
            err.contains("not a regular file"),
            "stderr must name the cause: {err}"
        );
    }

    #[test]
    fn selection_file_over_the_bound_is_refused() {
        let fixture = compile_fixture("c16x-oversized-selection", HERO_DEFS, HERO_COMPONENTS);
        let path = sparse_file(
            &fixture,
            "oversized.selection.json",
            INPUT_FILE_SIZE_LIMIT_BYTES as u64 + 1,
        );

        let (code, _stdout, err) = run_args(&[
            "cfx",
            "options",
            "--model",
            &fixture.manifest,
            "--selection-file",
            &path,
        ]);

        assert_eq!(code, EXIT_USAGE, "an oversized file must exit 2: {err}");
        assert!(
            err.contains(&format!(
                "--selection-file '{path}' exceeds {INPUT_FILE_SIZE_LIMIT_BYTES} bytes"
            )),
            "stderr must name the file and the bound: {err}"
        );
    }

    #[test]
    fn manifest_over_the_bound_is_refused() {
        let fixture = compile_fixture("c16x-oversized-manifest", HERO_DEFS, HERO_COMPONENTS);
        let path = sparse_file(
            &fixture,
            "oversized.environments.json",
            INPUT_FILE_SIZE_LIMIT_BYTES as u64 + 1,
        );
        let out = fixture.out("oversized_out").to_string_lossy().into_owned();

        let (code, _stdout, err) = run_args(&[
            "cfx",
            "resolve",
            "--model",
            &fixture.manifest,
            "--manifest",
            &path,
            "--all",
            "--out",
            &out,
        ]);

        assert_eq!(code, EXIT_USAGE, "an oversized manifest must exit 2: {err}");
        assert!(
            err.contains(&format!(
                "--manifest '{path}' exceeds {INPUT_FILE_SIZE_LIMIT_BYTES} bytes"
            )),
            "stderr must name the file and the bound: {err}"
        );
    }

    #[test]
    fn selection_file_exactly_on_the_bound_is_still_read() {
        // The bound is inclusive, and the capped reader stops one byte past it
        // so the two cases stay distinguishable. A file sitting exactly on the
        // bound is read whole and fails as the malformed document it is — a
        // reader that refused it would be a silent behaviour change for every
        // input that is merely large.
        let fixture = compile_fixture("c16x-onbound-selection", HERO_DEFS, HERO_COMPONENTS);
        let path = sparse_file(
            &fixture,
            "onbound.selection.json",
            INPUT_FILE_SIZE_LIMIT_BYTES as u64,
        );

        let (code, _stdout, err) = run_args(&[
            "cfx",
            "options",
            "--model",
            &fixture.manifest,
            "--selection-file",
            &path,
        ]);

        assert_eq!(code, EXIT_USAGE, "a malformed document still exits 2: {err}");
        assert!(
            !err.contains("exceeds"),
            "a file ON the bound must not be refused for its size: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn selection_file_that_is_a_fifo_is_refused_without_blocking() {
        // A FIFO with no writer blocks in `open(2)` forever. Settling the file
        // type from `stat()` — which does not block — is what keeps `cfx` from
        // hanging on a path an operator mistyped.
        let fixture = compile_fixture("c16x-fifo-selection", HERO_DEFS, HERO_COMPONENTS);
        let Some(path) = make_fifo(&fixture, "selection.fifo") else {
            return;
        };

        let (code, _stdout, err) = cfx_within_5s(&[
            "cfx",
            "options",
            "--model",
            &fixture.manifest,
            "--selection-file",
            &path,
        ]);

        assert_eq!(code, EXIT_USAGE, "a FIFO must exit 2: {err}");
        assert!(
            err.contains("not a regular file"),
            "stderr must name the cause: {err}"
        );
    }
}
