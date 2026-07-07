// SPDX-License-Identifier: BUSL-1.1

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use compiler::ccm_emitter::{emit_ccm_dir, ConditionModel};
use solver::{OxiddBackend, Session, SolverBackend};

#[test]
fn emitted_ccm_loads_into_solver_with_symbols() {
    let base = tempdir_for("ccm_emitter_roundtrip");
    let ccm_dir = base.join("ccm");
    let model = ConditionModel {
        bound_model_hash: "22".repeat(32),
        clauses: vec!["a == 'on' && b == 'enabled'".to_string()],
    };

    emit_ccm_dir(&model, &ccm_dir).expect("emit solver-loadable CCM");

    let ccm = Session::<OxiddBackend>::load_ccm(&ccm_dir).expect("load emitted CCM");
    assert_eq!(ccm.bound_model_hash(), [0x22; 32]);

    let symbols = ccm.symbols().expect("symbols should be populated");
    assert_eq!(symbols.var_count(), 2);
    assert_eq!(
        symbols.variable_order().collect::<Vec<_>>(),
        vec!["a.on", "b.enabled"]
    );
    assert_eq!(
        symbols.labels().collect::<Vec<_>>(),
        vec!["a=on", "b=enabled"]
    );
    assert_eq!(symbols.var_for_facet("a.on"), Some(0));
    assert_eq!(symbols.var_for_facet("b.enabled"), Some(1));

    let session = Session::<OxiddBackend>::new(ccm).expect("deserialize emitted BDD");
    let current = session.current();
    assert!(!session.backend().is_false(current));
    assert!(!session.backend().is_true(current));
}

fn tempdir_for(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "configflux-compiler-{test_name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}
