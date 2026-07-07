// SPDX-License-Identifier: BUSL-1.1
//
// Serializable projection of `solver::Session` state. Per ADR-0003 Section
// 4, the snapshot is the in-memory struct that the v0.4.0 daemon transport
// will cross the wire as JSON / CBOR / MessagePack. In v0.3.0 it is
// internal to the crate; its externally-visible form is deferred to the
// daemon ADR.
//
// The concrete field set below carries `ccm_hash` for the CCM identity
// check a future daemon transport will perform, and `schema_version` for
// the forward-compatibility rule stated in ADR-0005 Section 6. The
// session `snapshot`/`restore` stubs that previously produced and
// consumed this struct were retired per ADR-0030 D6; the daemon milestone
// defines the fields it actually needs when it is scheduled. This scaffold
// pins the hash and version fields only.

use serde::{Deserialize, Serialize};

/// Initial value for `Snapshot::schema_version`. Bumped when the snapshot
/// struct gains or loses fields in a way that breaks on-wire compatibility.
pub const SNAPSHOT_SCHEMA_VERSION_V1: u32 = 1;

/// A byte-stable projection of session state.
///
/// `Snapshot` is an internal type retained for the future daemon
/// transport envelope. The session `snapshot`/`restore` stubs that
/// previously produced and consumed it were retired per ADR-0030 D6
/// (zero production callers; a shaped-but-fake transport API is
/// speculative surface). The daemon milestone — explicitly out of
/// v0.4.0 — designs session transport, and its wire format, against
/// real requirements when it is scheduled. The `Serialize` /
/// `Deserialize` derives are kept so that work starts from a
/// serializable shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Content-address of the `.ccm` this snapshot was taken against.
    /// A future daemon transport will cross-check this byte-for-byte
    /// against the `Ccm` handle it is given and reject mismatches with a
    /// stable error.
    pub ccm_hash: [u8; 32],
    /// Version marker for the snapshot schema itself. Starts at
    /// `SNAPSHOT_SCHEMA_VERSION_V1`. Loaders reject snapshots whose
    /// `schema_version` they do not recognize.
    pub schema_version: u32,
}

impl Snapshot {
    /// Construct a fresh snapshot with an empty hash and the current
    /// schema version. Used by unit tests that round-trip through serde
    /// and by the type-reachability guards in the backend parity tests.
    pub fn empty() -> Self {
        Self {
            ccm_hash: [0u8; 32],
            schema_version: SNAPSHOT_SCHEMA_VERSION_V1,
        }
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_uses_v1_schema() {
        let snap = Snapshot::empty();
        assert_eq!(snap.ccm_hash, [0u8; 32]);
        assert_eq!(snap.schema_version, SNAPSHOT_SCHEMA_VERSION_V1);
        assert_eq!(snap.schema_version, 1);
    }

    #[test]
    fn snapshot_equality_is_field_wise() {
        let a = Snapshot::empty();
        let b = Snapshot::empty();
        assert_eq!(a, b);
        let mut c = Snapshot::empty();
        c.ccm_hash[0] = 1;
        assert_ne!(a, c);
        let mut d = Snapshot::empty();
        d.schema_version = 2;
        assert_ne!(a, d);
    }

    #[test]
    fn snapshot_default_matches_empty() {
        assert_eq!(Snapshot::default(), Snapshot::empty());
    }
}
