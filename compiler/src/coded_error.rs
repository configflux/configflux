// SPDX-License-Identifier: BUSL-1.1

//! The diagnostic code a refusal carries, as DATA on the error.
//!
//! configflux-py7w. The product mappers used to recover a code from the error's
//! PROSE — `message.contains("dependency cycle detected")` and eight siblings —
//! while nearly every validator message interpolates an authored id. Naming a
//! definition `closed facet` therefore handed the caller the closed-facet rule's
//! code for a non-finite-float refusal: four chunks carrying ONE fault came back
//! with four different codes, chosen by the author. The code is a frozen
//! contract callers branch on (`docs/interface-contracts.md` §3.4), so that is a
//! broken contract even though every one of those models was still refused.
//!
//! The fix is this type. A rule that has a code attaches it at the point of
//! refusal, the mappers read it back off the error chain, and no amount of
//! authored text can move it. [`fmt::Display`] prints the message and nothing
//! else, so attaching a code leaves the text a caller sees byte-identical —
//! which is what lets every existing message stay exactly as it is.
//!
//! A refusal with NO `CodedError` in its chain is not an error in this scheme:
//! it is how a rule says it has no dedicated code, and the mappers land it on
//! `E_COMPILE_INPUT_INVALID`. There is deliberately no substring fallback — a
//! prose sniff is the hole itself, and one left anywhere would keep every
//! neighbouring arm steerable.

use std::error::Error;
use std::fmt;

/// A refusal that names its own diagnostic code.
#[derive(Debug)]
pub(crate) struct CodedError {
    /// The frozen `E_*` code this rule reports (`product_api`'s constants).
    pub(crate) code: &'static str,
    /// The author-facing message, printed verbatim by [`fmt::Display`].
    pub(crate) message: String,
    /// The remedy, for a rule whose remedy is not its code's default
    /// (`product_api::hint_for`). Two codes are shared by two rules each, and
    /// the halves deliberately differ in remedy: `E_FACET_VALUE_UNDECLARED`
    /// covers both an undeclared facet and a value outside a closed domain, and
    /// `E_INGEST_DUPLICATE_FACET` covers a duplicate facet and a duplicate
    /// binding (a binding IS a facet, ADR-0057 §D3). `None` takes the default.
    pub(crate) hint: Option<&'static str>,
}

impl fmt::Display for CodedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CodedError {}

/// A refusal carrying `code`, reported with that code's default remedy.
pub(crate) fn coded(code: &'static str, message: String) -> anyhow::Error {
    anyhow::Error::new(CodedError {
        code,
        message,
        hint: None,
    })
}

/// A refusal carrying `code` with a remedy of its own — for a rule that shares
/// its code with another rule whose remedy would be wrong here.
pub(crate) fn coded_with_hint(
    code: &'static str,
    hint: &'static str,
    message: String,
) -> anyhow::Error {
    anyhow::Error::new(CodedError {
        code,
        message,
        hint: Some(hint),
    })
}

/// The FIRST coded refusal in `err`'s chain, or `None` when nothing in it named
/// a code.
///
/// First rather than innermost: a wrapper that adds a code is stating the
/// diagnostic the caller should see, and the message the caller is shown comes
/// from the same end of the chain (`anyhow::Error::to_string` prints the
/// outermost context), so the two always agree about which refusal is being
/// reported.
pub(crate) fn coded_of(err: &anyhow::Error) -> Option<&CodedError> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<CodedError>())
}

/// `anyhow::bail!` for a rule that has a diagnostic code.
macro_rules! coded_bail {
    ($code:expr, $($arg:tt)*) => {
        return ::core::result::Result::Err($crate::coded_error::coded($code, format!($($arg)*)))
    };
}

/// `coded_bail!` for a rule whose remedy is not its code's default.
macro_rules! coded_bail_hint {
    ($code:expr, $hint:expr, $($arg:tt)*) => {
        return ::core::result::Result::Err($crate::coded_error::coded_with_hint(
            $code,
            $hint,
            format!($($arg)*),
        ))
    };
}

pub(crate) use coded_bail;
pub(crate) use coded_bail_hint;
