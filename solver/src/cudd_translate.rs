// SPDX-License-Identifier: BUSL-1.1
//
// Internal CCM bytestream <-> CUDD translation pass — configflux-fvew.
//
// This module owns the internal mapping between the on-disk
// `ccm.bdd.bin` byte layout (ADR-0005 §4 canonical post-order node
// table) and CUDD's in-memory ROBDD representation (`*mut DdNode` rooted
// at a `*mut DdManager`). It exists so `backend_cudd.rs` can stay
// focused on the `SolverBackend` trait surface and so the byte<->CUDD
// translation is exercised independently by a round-trip integration
// test.
//
// # Scope of configflux-fvew
//
// - **Reverse direction (`cudd_to_canonical`)**: walk a CUDD-rooted
//   sub-DAG and emit the ADR-0005 §4 canonical node table (post-order,
//   flags=0). This is new capability — `backend_cudd.rs` does not need
//   it for the M1 surface, but the round-trip test in
//   `tests/cudd_round_trip_bdd_bin.rs` requires it to prove the swap is
//   byte-deterministic.
//
// - **Forward direction (`canonical_to_cudd`)**: rebuild a CUDD BDD
//   from a `&[BddNode]` table. This is a refactor of the body of
//   `CuddBackend::deserialize_bdd` so the same translation is reachable
//   from the round-trip test without going through the trait. The
//   trait method becomes a thin wrapper that calls this function and
//   pushes the result into the backend's handle table.
//
// - **Bytes <-> canonical (`encode_canonical_to_bytes`)**: a tiny
//   reverse of `parse_bdd_bin` (which lives in `ccm_format.rs`). Lives
//   here because it is only used by the round-trip path; the solver
//   itself never writes a `.ccm` (the compiler does).
//
// # Complement-edge handling (the only real subtlety)
//
// CUDD uses complemented edges internally: every non-constant node may
// be reached through a pointer with bit 0 set, and `Cudd_ReadLogicZero`
// is just `Cudd_Not(Cudd_ReadOne)`. ADR-0005 §4 reserves `flags` bit 0
// for the complement bit but says CUDD's default mode "MUST" emit
// `flags = 0`. To honour that, `cudd_to_canonical` **erases** complement
// edges by treating each CUDD pointer as a `(regular_ptr, phase)` pair
// and synthesising a separate canonical node id for the negated phase
// of any node it walks. The resulting table is complement-free and
// reduced (no `low_id == high_id`, no duplicate `(var, low, high)` triples
// because the synthesis uses a memo keyed by `(regular_ptr, phase)`).
//
// This is the standard "complement-edge erasure" walk; it costs at most
// 2x the regular DAG node count (most fixtures hit far less because
// only one phase is ever observed for each regular pointer).
//
// # Memory discipline
//
// `canonical_to_cudd` allocates fresh CUDD variables and intermediate
// nodes; the caller (`CuddBackend::deserialize_bdd`) is responsible for
// `Cudd_Ref` / `Cudd_RecursiveDeref` accounting. This module does
// reference the root and intermediates exactly the same way the previous
// in-place implementation did (ref every materialised node; deref every
// non-root intermediate after root resolution); the algorithm is moved,
// not weakened.
//
// `cudd_to_canonical` does NOT deref or modify the input DAG; it is a
// pure read-only walk that returns Rust-owned `BddNode` records.

use core::ffi::c_int;
use std::collections::HashMap;
use std::ptr::NonNull;

use cudd_sys::cudd::{
    Cudd_bddIte, Cudd_bddIthVar, Cudd_E, Cudd_IsComplement, Cudd_IsConstant, Cudd_NodeReadIndex,
    Cudd_ReadLogicZero, Cudd_ReadOne, Cudd_Ref, Cudd_RecursiveDeref, Cudd_Regular, Cudd_T,
};
use cudd_sys::{DdManager, DdNode};

use crate::backend::BackendError;
use crate::ccm_format::{
    BddNode, BDD_BIN_HEADER_BYTES, CCM_BDD_BIN_MAGIC, CCM_BDD_BIN_VERSION, NODE_RECORD_BYTES,
    TERMINAL_FALSE, TERMINAL_TRUE, TERMINAL_VAR_INDEX,
};

// ---------------------------------------------------------------------
// Forward direction: canonical node table -> CUDD root pointer.
// ---------------------------------------------------------------------

/// Result of `canonical_to_cudd`: the CUDD-side root pointer plus the
/// per-variable literal nodes the caller must keep alive (`Cudd_Ref`'d)
/// for the lifetime of the session because `is_var_sat_under` and
/// `apply_and` index into it. Both vectors are caller-owned; this module
/// has done the `Cudd_Ref` calls (one per variable, one for the root)
/// and made no other mutation to the manager.
pub(crate) struct CanonicalToCuddOutput {
    /// Root CUDD pointer for the rebuilt BDD. Already `Cudd_Ref`'d; the
    /// caller is responsible for the eventual `Cudd_RecursiveDeref`.
    pub root: NonNull<DdNode>,
    /// `var_functions[i]` is the literal `x_i = 1` node. Each entry is
    /// `Cudd_Ref`'d; the caller is responsible for the eventual
    /// `Cudd_RecursiveDeref`. Vector length equals the input
    /// `var_count`.
    pub var_functions: Vec<NonNull<DdNode>>,
}

/// Rebuild a CUDD BDD from the ADR-0005 §4 canonical node table.
/// Mirrors the walk previously inlined in `CuddBackend::deserialize_bdd`;
/// extracted here so the round-trip test can reach it without touching
/// the backend trait. See the module-level doc for the algorithm and
/// memory-discipline contract.
///
/// # Safety
///
/// The caller must pass a valid, live `*mut DdManager`. The returned
/// CUDD pointers are valid for the lifetime of that manager and become
/// invalid the instant `Cudd_Quit` is called on it. The caller owns the
/// `Cudd_Ref` accounting for the returned `root` and every entry of
/// `var_functions`.
pub(crate) unsafe fn canonical_to_cudd(
    manager: NonNull<DdManager>,
    var_count: u32,
    nodes: &[BddNode],
    root: u32,
) -> Result<CanonicalToCuddOutput, BackendError> {
    // Step 1: allocate `var_count` fresh variables. CUDD's
    // `Cudd_bddIthVar` is idempotent on a given index, so this is safe
    // even if the caller invokes the translation twice on the same
    // manager (which the backend wrapper does not, but tests may).
    let mut var_functions: Vec<NonNull<DdNode>> = Vec::with_capacity(var_count as usize);
    for i in 0..var_count {
        // SAFETY: manager is live; Cudd_bddIthVar returns NULL only on
        // allocation failure, which we surface as OutOfCapacity.
        let var_ptr = Cudd_bddIthVar(manager.as_ptr(), i as c_int);
        let var = NonNull::new(var_ptr).ok_or(BackendError::OutOfCapacity)?;
        // SAFETY: `var` is a valid CUDD-managed node.
        Cudd_Ref(var.as_ptr());
        var_functions.push(var);
    }

    // Step 2: snapshot terminal sentinels.
    // SAFETY: manager is live.
    let false_ptr = Cudd_ReadLogicZero(manager.as_ptr());
    let true_ptr = Cudd_ReadOne(manager.as_ptr());
    let false_node = NonNull::new(false_ptr).ok_or(BackendError::Invariant(
        "canonical_to_cudd: Cudd_ReadLogicZero returned NULL",
    ))?;
    let true_node = NonNull::new(true_ptr).ok_or(BackendError::Invariant(
        "canonical_to_cudd: Cudd_ReadOne returned NULL",
    ))?;

    // Trivial-terminal fast paths so the empty-CCM and constant-formula
    // cases do not allocate a `rebuilt` table.
    if root == TERMINAL_TRUE {
        // SAFETY: terminals do not need a Cudd_Ref to stay alive in
        // CUDD's bookkeeping, but reffing keeps the discipline uniform
        // with the non-terminal path.
        Cudd_Ref(true_node.as_ptr());
        return Ok(CanonicalToCuddOutput {
            root: true_node,
            var_functions,
        });
    }
    if root == TERMINAL_FALSE {
        // SAFETY: as above.
        Cudd_Ref(false_node.as_ptr());
        return Ok(CanonicalToCuddOutput {
            root: false_node,
            var_functions,
        });
    }

    // Step 3: walk `nodes` in post-order. `rebuilt[i]` is the CUDD node
    // for on-disk index `i`; populated only as we cross the slot, which
    // is what makes the forward references (low_id < i, high_id < i)
    // valid. Every non-terminal materialised here is `Cudd_Ref`'d so it
    // cannot be GC'd by CUDD's unique table mid-walk.
    let mut rebuilt: Vec<Option<NonNull<DdNode>>> = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        if node.var_index == TERMINAL_VAR_INDEX {
            // ADR-0005 §4: index 0 = FALSE, index 1 = TRUE.
            if i == 0 {
                rebuilt[i] = Some(false_node);
            } else if i == 1 {
                rebuilt[i] = Some(true_node);
            } else {
                unwind_rebuilt(manager, &mut rebuilt, nodes);
                drain_var_functions(manager, var_functions);
                return Err(BackendError::Serialization(
                    "canonical_to_cudd: terminal-tagged node at unexpected index",
                ));
            }
            continue;
        }
        if node.var_index as usize >= var_functions.len() {
            unwind_rebuilt(manager, &mut rebuilt, nodes);
            drain_var_functions(manager, var_functions);
            return Err(BackendError::Serialization(
                "canonical_to_cudd: node var_index out of range",
            ));
        }
        let low = match lookup_child(node.low_id, &rebuilt, false_node, true_node) {
            Ok(p) => p,
            Err(e) => {
                unwind_rebuilt(manager, &mut rebuilt, nodes);
                drain_var_functions(manager, var_functions);
                return Err(e);
            }
        };
        let high = match lookup_child(node.high_id, &rebuilt, false_node, true_node) {
            Ok(p) => p,
            Err(e) => {
                unwind_rebuilt(manager, &mut rebuilt, nodes);
                drain_var_functions(manager, var_functions);
                return Err(e);
            }
        };
        // Shannon expansion: node = ite(var, high, low). CUDD's
        // `Cudd_bddIte` implements this directly.
        let var_ptr = var_functions[node.var_index as usize].as_ptr();
        // SAFETY: every input pointer is live and CUDD-managed; manager
        // is live.
        let built_ptr = Cudd_bddIte(manager.as_ptr(), var_ptr, high.as_ptr(), low.as_ptr());
        let built = match NonNull::new(built_ptr) {
            Some(p) => p,
            None => {
                unwind_rebuilt(manager, &mut rebuilt, nodes);
                drain_var_functions(manager, var_functions);
                return Err(BackendError::OutOfCapacity);
            }
        };
        // SAFETY: `built` is a valid CUDD-managed node.
        Cudd_Ref(built.as_ptr());
        rebuilt[i] = Some(built);
    }

    // Step 4: resolve root.
    let root_idx = root as usize;
    if root_idx >= rebuilt.len() {
        unwind_rebuilt(manager, &mut rebuilt, nodes);
        drain_var_functions(manager, var_functions);
        return Err(BackendError::Serialization(
            "canonical_to_cudd: root index out of range",
        ));
    }
    let root_node = match rebuilt[root_idx].take() {
        Some(p) => p,
        None => {
            unwind_rebuilt(manager, &mut rebuilt, nodes);
            drain_var_functions(manager, var_functions);
            return Err(BackendError::Serialization(
                "canonical_to_cudd: root references an unprocessed node slot",
            ));
        }
    };

    // The root keeps its ref. Deref every non-terminal intermediate
    // we still hold; terminals were never reffed in this loop and must
    // NOT be dereffed here.
    for (i, slot) in rebuilt.iter_mut().enumerate() {
        if let Some(p) = slot.take() {
            if nodes[i].var_index == TERMINAL_VAR_INDEX {
                continue;
            }
            // SAFETY: we held a ref on `p` from the build loop.
            Cudd_RecursiveDeref(manager.as_ptr(), p.as_ptr());
        }
    }

    Ok(CanonicalToCuddOutput {
        root: root_node,
        var_functions,
    })
}

/// Helper used by error paths in `canonical_to_cudd` to deref every
/// intermediate we materialised before bailing.
unsafe fn unwind_rebuilt(
    manager: NonNull<DdManager>,
    rebuilt: &mut [Option<NonNull<DdNode>>],
    nodes: &[BddNode],
) {
    for (i, slot) in rebuilt.iter_mut().enumerate() {
        if let Some(p) = slot.take() {
            if i < nodes.len() && nodes[i].var_index == TERMINAL_VAR_INDEX {
                continue;
            }
            // SAFETY: we held a ref on `p` from the build loop.
            Cudd_RecursiveDeref(manager.as_ptr(), p.as_ptr());
        }
    }
}

/// Helper used by error paths in `canonical_to_cudd` to deref every
/// per-variable literal we allocated before bailing.
unsafe fn drain_var_functions(manager: NonNull<DdManager>, vars: Vec<NonNull<DdNode>>) {
    for v in vars {
        // SAFETY: we held a ref on `v` from the var allocation loop.
        Cudd_RecursiveDeref(manager.as_ptr(), v.as_ptr());
    }
}

/// Resolve a `(low_id, high_id)`-style child reference: a sentinel
/// points at the appropriate terminal directly; anything else indexes
/// into the in-progress `rebuilt` vector. The return is non-owning.
fn lookup_child(
    id: u32,
    rebuilt: &[Option<NonNull<DdNode>>],
    false_node: NonNull<DdNode>,
    true_node: NonNull<DdNode>,
) -> Result<NonNull<DdNode>, BackendError> {
    if id == TERMINAL_FALSE {
        return Ok(false_node);
    }
    if id == TERMINAL_TRUE {
        return Ok(true_node);
    }
    let idx = id as usize;
    if idx >= rebuilt.len() {
        return Err(BackendError::Serialization(
            "canonical_to_cudd: child index out of range",
        ));
    }
    rebuilt[idx].ok_or(BackendError::Serialization(
        "canonical_to_cudd: child references an unprocessed node slot",
    ))
}

// ---------------------------------------------------------------------
// Reverse direction: CUDD root pointer -> canonical node table.
// ---------------------------------------------------------------------

/// Walk a CUDD-rooted sub-DAG and emit the ADR-0005 §4 canonical node
/// table for the same Boolean function. Complement edges are erased
/// (see module-level doc) so every emitted record has `flags = 0`.
///
/// Returns `(nodes, root_id)` where `nodes` already includes the two
/// mandatory terminal records at indices 0 (FALSE) and 1 (TRUE), and
/// `root_id` is either a sentinel (`TERMINAL_FALSE` / `TERMINAL_TRUE`)
/// for a constant formula or a zero-based index into `nodes` for any
/// non-trivial root.
///
/// # Safety
///
/// `manager` must be a live CUDD manager and `root` must be a valid
/// node pointer in that manager. The walk is read-only; this function
/// does NOT take or release any CUDD references.
//
// `#[allow(dead_code)]` because the only in-tree caller is the
// crate-internal round-trip test below; the production solver path
// (interpreter, runtime) never calls the reverse direction. Lifting
// this when a future task wires CUDD-side .ccm emission into the
// compiler is a deliberate change that comes with its own tests.
#[allow(dead_code)]
pub(crate) unsafe fn cudd_to_canonical(
    manager: NonNull<DdManager>,
    root: NonNull<DdNode>,
) -> (Vec<BddNode>, u32) {
    // Always emit the two mandatory terminal records first so callers
    // can pass the result straight into `encode_canonical_to_bytes`.
    let mut out: Vec<BddNode> = vec![
        BddNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_FALSE,
            high_id: TERMINAL_FALSE,
            flags: 0,
        },
        BddNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_TRUE,
            high_id: TERMINAL_TRUE,
            flags: 0,
        },
    ];

    // Memo keyed by `(regular_ptr_addr, phase)`. `phase = 1` means "the
    // canonical node is the negation of `regular_ptr`'s function". We
    // store the assigned canonical node-table index (0/1 for terminals,
    // >= 2 for non-terminals).
    let mut memo: HashMap<(usize, u8), u32> = HashMap::new();
    let root_index = visit(manager, root, &mut out, &mut memo);
    // ADR-0005 §4: `root_table[0]` is a node-table index for any
    // non-trivial formula, but for a trivially-constant formula it
    // must be the terminal sentinel (0xFFFFFFFE for TRUE, 0xFFFFFFFF
    // for FALSE). Translate back here so the test fixture round-trips
    // byte-identically.
    let root_id = match root_index {
        FALSE_NODE_INDEX => TERMINAL_FALSE,
        TRUE_NODE_INDEX => TERMINAL_TRUE,
        idx => idx,
    };
    (out, root_id)
}

/// Index of the FALSE terminal record in the node table per ADR-0005
/// §4 ("index 0 is FALSE"). This is the integer the canonical encoder
/// stores in a non-terminal's `low_id`/`high_id` when the child is the
/// FALSE constant, matching the compiler-side emitter convention; the
/// terminal sentinel `TERMINAL_FALSE` (`0xFFFFFFFF`) is reserved for
/// the `root_table` slot when the entire formula reduces to FALSE.
const FALSE_NODE_INDEX: u32 = 0;

/// Index of the TRUE terminal record in the node table per ADR-0005
/// §4 ("index 1 is TRUE"). Same rationale as `FALSE_NODE_INDEX`.
const TRUE_NODE_INDEX: u32 = 1;

/// Recursive worker for `cudd_to_canonical`. Returns the **node-table
/// index** for the function reached through `node` (which may be a
/// complemented pointer). Terminals collapse to indices 0/1 (the
/// compiler-side emitter convention) so a parent record can use the
/// returned id directly in its `low_id`/`high_id` field; the caller
/// (`cudd_to_canonical`) is responsible for converting an index of 0
/// or 1 into a terminal sentinel only when that index ends up in
/// `root_table[0]` for a trivially-constant formula.
//
// `#[allow(dead_code)]` for the same reason as `cudd_to_canonical`
// above — only reached from the crate-internal round-trip test until
// a future task wires CUDD-side emission.
#[allow(dead_code)]
unsafe fn visit(
    manager: NonNull<DdManager>,
    node: NonNull<DdNode>,
    out: &mut Vec<BddNode>,
    memo: &mut HashMap<(usize, u8), u32>,
) -> u32 {
    // SAFETY: caller guarantees `node` is a live CUDD pointer.
    let regular_ptr = Cudd_Regular(node.as_ptr());
    let phase: u8 = if Cudd_IsComplement(node.as_ptr()) != 0 { 1 } else { 0 };

    // Constant case: `Cudd_ReadOne` is the canonical TRUE; complementing
    // it gives FALSE. Return the node-table index of the appropriate
    // terminal (0 for FALSE, 1 for TRUE), NOT the sentinel — sentinels
    // are only used in `root_table[]`, never as a child reference, per
    // the compiler-side emitter convention.
    if Cudd_IsConstant(regular_ptr) != 0 {
        if phase == 0 {
            return TRUE_NODE_INDEX;
        } else {
            return FALSE_NODE_INDEX;
        }
    }

    // Non-constant: memoise by `(regular_ptr, phase)`.
    let key = (regular_ptr as usize, phase);
    if let Some(&id) = memo.get(&key) {
        return id;
    }

    // Recurse on regular children. ADR-0005 §4 stores `low` (else)
    // and `high` (then) in that order; CUDD exposes them as `Cudd_E`
    // and `Cudd_T`. Apply the phase BEFORE recursing so the children
    // we see live in the same Boolean-function space as the parent —
    // i.e. when `phase == 1` we recurse on the negation of each child.
    let then_child_raw = NonNull::new(Cudd_T(regular_ptr))
        .expect("Cudd_T returned NULL on non-constant node");
    let else_child_raw = NonNull::new(Cudd_E(regular_ptr))
        .expect("Cudd_E returned NULL on non-constant node");
    let then_child = if phase == 1 {
        NonNull::new(((then_child_raw.as_ptr() as usize) ^ 1) as *mut DdNode)
            .expect("phase-flipped pointer is non-null")
    } else {
        then_child_raw
    };
    let else_child = if phase == 1 {
        NonNull::new(((else_child_raw.as_ptr() as usize) ^ 1) as *mut DdNode)
            .expect("phase-flipped pointer is non-null")
    } else {
        else_child_raw
    };

    let high_id = visit(manager, then_child, out, memo);
    let low_id = visit(manager, else_child, out, memo);

    // ROBDD reduction rule 1: collapse identical-children nodes. CUDD
    // already enforces this internally for its own representation, but
    // after phase erasure the two children may have collapsed to the
    // same canonical id, so re-check here.
    if low_id == high_id {
        memo.insert(key, low_id);
        return low_id;
    }

    let var_index = Cudd_NodeReadIndex(regular_ptr);
    let id = out.len() as u32;
    out.push(BddNode {
        var_index,
        low_id,
        high_id,
        flags: 0,
    });
    memo.insert(key, id);
    id
}

// ---------------------------------------------------------------------
// Bytes <-> canonical (write side; read side lives in ccm_format.rs).
// ---------------------------------------------------------------------

/// Encode a canonical `(nodes, root)` pair to the ADR-0005 §4 byte
/// layout. The output is byte-identical to what
/// `compiler/src/ccm_emitter/bdd.rs::serialize` would emit for the same
/// reduced ROBDD; this is the contract the round-trip test pins.
///
/// `var_count` is encoded into the header; `nodes.len()` becomes
/// `node_count`; `root` is written as the sole entry of `root_table`
/// (`root_count = 1`, matching ADR-0005 §4 "the primary root is index 0").
//
// `#[allow(dead_code)]` for the same reason as `cudd_to_canonical`
// above — the reverse-emit path is exercised by the crate-internal
// round-trip test; the production code path (compiler-side emission)
// has its own canonical encoder in `compiler/src/ccm_emitter/bdd.rs`.
#[allow(dead_code)]
pub(crate) fn encode_canonical_to_bytes(var_count: u32, nodes: &[BddNode], root: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(BDD_BIN_HEADER_BYTES + 4 + nodes.len() * NODE_RECORD_BYTES);
    bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
    bytes.push(CCM_BDD_BIN_VERSION);
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&var_count.to_le_bytes());
    bytes.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count = 1
    bytes.extend_from_slice(&root.to_le_bytes());
    for node in nodes {
        bytes.extend_from_slice(&node.var_index.to_le_bytes());
        bytes.extend_from_slice(&node.low_id.to_le_bytes());
        bytes.extend_from_slice(&node.high_id.to_le_bytes());
        bytes.push(node.flags);
        bytes.extend_from_slice(&[0, 0, 0]);
    }
    bytes
}

#[cfg(test)]
mod tests {
    //! Crate-internal round-trip tests for the CCM <-> CUDD translation
    //! pass — configflux-fvew acceptance criterion 1.
    //!
    //! These live inside the module so the tests can reach the
    //! `pub(crate)` translation entry points and the `pub(crate)` parser
    //! in `ccm_format` without going through the trait surface or
    //! re-exporting internals. The integration-level CUDD tests in
    //! `solver/tests/cudd_backend_parity.rs` cover the trait surface;
    //! this module covers the byte-stream <-> CUDD pass directly.
    //!
    //! Each test uses an isolated CUDD manager (its own `Cudd_Init` /
    //! `Cudd_Quit` pair) so refcount mistakes surface as either a
    //! `Cudd_Quit` assert in a debug build of CUDD or as a leak in
    //! ASan; the production build tolerates leaks but the CI gate
    //! catches functional regressions via the round-trip equality
    //! assertion.
    use super::*;
    use crate::ccm_format::parse_bdd_bin;
    use cudd_sys::cudd::{
        Cudd_Init, Cudd_Quit, Cudd_RecursiveDeref, CUDD_CACHE_SLOTS, CUDD_UNIQUE_SLOTS,
    };

    /// Build a minimal hand-authored canonical node table for the
    /// formula `x0 ∧ x1`. Two terminals at indices 0/1, plus one
    /// non-terminal at index 2 (`var=0, low=FALSE, high=high_var`) and
    /// one at index 3 (`var=1, low=FALSE, high=TRUE`). Per ADR-0005 §4
    /// the order is: TERMINAL_FALSE first, then TERMINAL_TRUE, then
    /// non-terminals in post-order. Root = index 3 (the `x0` node) per
    /// post-order: x1's node (index 2) is the high-child of x0, and
    /// children precede parents.
    fn and_two_var_table() -> (Vec<BddNode>, u32) {
        let nodes = vec![
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_FALSE,
                high_id: TERMINAL_FALSE,
                flags: 0,
            },
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_TRUE,
                high_id: TERMINAL_TRUE,
                flags: 0,
            },
            // x1 node: low=FALSE (index 0), high=TRUE (index 1).
            BddNode {
                var_index: 1,
                low_id: 0,
                high_id: 1,
                flags: 0,
            },
            // x0 node: low=FALSE (index 0), high=x1 (index 2).
            BddNode {
                var_index: 0,
                low_id: 0,
                high_id: 2,
                flags: 0,
            },
        ];
        (nodes, 3)
    }

    /// Spin up a CUDD manager exactly as `CuddBackend::new_session`
    /// does, returning the raw `*mut DdManager` for tests that work
    /// directly with `canonical_to_cudd` / `cudd_to_canonical`. The
    /// caller is responsible for `Cudd_Quit` plus dereferencing every
    /// reffed node; the helper does not own the manager.
    unsafe fn fresh_cudd_manager() -> NonNull<DdManager> {
        let m = Cudd_Init(0, 0, CUDD_UNIQUE_SLOTS, CUDD_CACHE_SLOTS, 0);
        NonNull::new(m).expect("Cudd_Init must succeed in tests")
    }

    #[test]
    fn round_trip_trivial_true_is_byte_identical() {
        // Empty-CCM analogue: zero variables, root = TERMINAL_TRUE.
        // Bytes A: header + root_table=[TERMINAL_TRUE] + two terminal
        // records. After CUDD ingest + re-emit we expect the same
        // bytes back.
        let nodes = vec![
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_FALSE,
                high_id: TERMINAL_FALSE,
                flags: 0,
            },
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_TRUE,
                high_id: TERMINAL_TRUE,
                flags: 0,
            },
        ];
        let bytes_a = encode_canonical_to_bytes(0, &nodes, TERMINAL_TRUE);

        // Round-trip through CUDD.
        // SAFETY: each unsafe block is bounded; the manager lives only
        // for the test; every node we ref we deref before Cudd_Quit.
        unsafe {
            let mgr = fresh_cudd_manager();
            let payload = parse_bdd_bin(&bytes_a, 0, nodes.len() as u64)
                .expect("parse_bdd_bin must accept the trivial-true emission");
            let translated = canonical_to_cudd(mgr, payload.var_count, &payload.nodes, payload.roots[0])
                .expect("canonical_to_cudd must accept the trivial-true table");
            let (out_nodes, out_root) = cudd_to_canonical(mgr, translated.root);
            let bytes_b = encode_canonical_to_bytes(payload.var_count, &out_nodes, out_root);

            // Drop the root ref then Quit.
            Cudd_RecursiveDeref(mgr.as_ptr(), translated.root.as_ptr());
            for v in translated.var_functions {
                Cudd_RecursiveDeref(mgr.as_ptr(), v.as_ptr());
            }
            Cudd_Quit(mgr.as_ptr());

            assert_eq!(
                bytes_a, bytes_b,
                "trivial-TRUE round-trip must be byte-identical"
            );
        }
    }

    #[test]
    fn round_trip_trivial_false_is_byte_identical() {
        // Same shape as the TRUE case but with root = TERMINAL_FALSE.
        let nodes = vec![
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_FALSE,
                high_id: TERMINAL_FALSE,
                flags: 0,
            },
            BddNode {
                var_index: TERMINAL_VAR_INDEX,
                low_id: TERMINAL_TRUE,
                high_id: TERMINAL_TRUE,
                flags: 0,
            },
        ];
        let bytes_a = encode_canonical_to_bytes(0, &nodes, TERMINAL_FALSE);

        unsafe {
            let mgr = fresh_cudd_manager();
            let payload = parse_bdd_bin(&bytes_a, 0, nodes.len() as u64)
                .expect("parse_bdd_bin must accept the trivial-false emission");
            let translated = canonical_to_cudd(mgr, payload.var_count, &payload.nodes, payload.roots[0])
                .expect("canonical_to_cudd must accept the trivial-false table");
            let (out_nodes, out_root) = cudd_to_canonical(mgr, translated.root);
            let bytes_b = encode_canonical_to_bytes(payload.var_count, &out_nodes, out_root);

            Cudd_RecursiveDeref(mgr.as_ptr(), translated.root.as_ptr());
            for v in translated.var_functions {
                Cudd_RecursiveDeref(mgr.as_ptr(), v.as_ptr());
            }
            Cudd_Quit(mgr.as_ptr());

            assert_eq!(
                bytes_a, bytes_b,
                "trivial-FALSE round-trip must be byte-identical"
            );
        }
    }

    #[test]
    fn round_trip_two_var_and_is_byte_identical() {
        // The acceptance-criterion test: a small but non-trivial BDD
        // (`x0 ∧ x1`) goes through the full forward + reverse chain
        // and must emerge byte-identical. This proves both directions
        // of the translation pass and proves the emit path is
        // deterministic for a fixed input topology.
        let (nodes, root) = and_two_var_table();
        let bytes_a = encode_canonical_to_bytes(2, &nodes, root);

        unsafe {
            let mgr = fresh_cudd_manager();
            let payload = parse_bdd_bin(&bytes_a, 2, nodes.len() as u64)
                .expect("parse_bdd_bin must accept the x0 ∧ x1 emission");
            let translated = canonical_to_cudd(mgr, payload.var_count, &payload.nodes, payload.roots[0])
                .expect("canonical_to_cudd must accept the x0 ∧ x1 table");
            let (out_nodes, out_root) = cudd_to_canonical(mgr, translated.root);
            let bytes_b = encode_canonical_to_bytes(payload.var_count, &out_nodes, out_root);

            Cudd_RecursiveDeref(mgr.as_ptr(), translated.root.as_ptr());
            for v in translated.var_functions {
                Cudd_RecursiveDeref(mgr.as_ptr(), v.as_ptr());
            }
            Cudd_Quit(mgr.as_ptr());

            assert_eq!(
                bytes_a, bytes_b,
                "x0 ∧ x1 round-trip must be byte-identical (CUDD-emitted .ccm \
                 must re-emit deterministically per configflux-fvew acceptance)"
            );
        }
    }

    #[test]
    fn double_round_trip_is_byte_stable() {
        // Determinism check: the second round-trip must equal the
        // first. This catches a subtle class of bugs where the first
        // walk happens to match the input by coincidence (e.g. CUDD's
        // first build of the DAG matches the input topology) but a
        // second build through the same path produces different bytes.
        let (nodes, root) = and_two_var_table();
        let bytes_a = encode_canonical_to_bytes(2, &nodes, root);

        let mut prior: Option<Vec<u8>> = None;
        for iter in 0..3 {
            // SAFETY: bounded refcount discipline; see other tests.
            unsafe {
                let mgr = fresh_cudd_manager();
                let payload = parse_bdd_bin(&bytes_a, 2, nodes.len() as u64)
                    .expect("parse_bdd_bin");
                let translated =
                    canonical_to_cudd(mgr, payload.var_count, &payload.nodes, payload.roots[0])
                        .expect("canonical_to_cudd");
                let (out_nodes, out_root) = cudd_to_canonical(mgr, translated.root);
                let bytes_b = encode_canonical_to_bytes(payload.var_count, &out_nodes, out_root);
                Cudd_RecursiveDeref(mgr.as_ptr(), translated.root.as_ptr());
                for v in translated.var_functions {
                    Cudd_RecursiveDeref(mgr.as_ptr(), v.as_ptr());
                }
                Cudd_Quit(mgr.as_ptr());

                if let Some(p) = prior.as_ref() {
                    assert_eq!(p, &bytes_b, "iteration {iter}: round-trip must be stable");
                }
                prior = Some(bytes_b);
            }
        }
    }
}
