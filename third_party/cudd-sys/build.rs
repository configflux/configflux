// Cargo-path build of the vendored CUDD 3.0.0 C library.
//
// Bazel does NOT run this script: the `rust_library` at
// `//third_party/cudd-sys:cudd_sys` links the hand-authored cc_library
// `//third_party/cudd:cudd` instead (ADR-0004 §3 second-pass amendment).
// This script exists for the *cargo* build graph only — release binary
// cross-compilation via cargo-zigbuild (ADR-0009 §2) and source builds by
// public consumers without Bazel (ADR-0009 §"build from source"). Since the
// solver became the decision engine (ADR-0017/ADR-0030), all three shipped
// binaries link cudd-sys, so cargo must be able to provide the C symbols.
//
// The compile recipe mirrors `third_party/cudd/BUILD.bazel` exactly:
// identical source lists (upstream `cplusplus/` and `nanotrav/` stay
// excluded), the hand-authored `config.h` reachable as `<config.h>`, and
// the same copts (`-DHAVE_CONFIG_H=1 -w -fno-strict-aliasing -pthread`).
// Keep the two in sync — the Bazel file is the source of truth.

const UTIL_SRCS: &[&str] = &[
    "util/cpu_stats.c",
    "util/cpu_time.c",
    "util/cstringstream.c",
    "util/datalimit.c",
    "util/pathsearch.c",
    "util/pipefork.c",
    "util/prtime.c",
    "util/safe_mem.c",
    "util/strsav.c",
    "util/texpand.c",
    "util/ucbqsort.c",
];

const EPD_SRCS: &[&str] = &["epd/epd.c"];

const ST_SRCS: &[&str] = &["st/st.c"];

const MTR_SRCS: &[&str] = &["mtr/mtrBasic.c", "mtr/mtrGroup.c"];

const CUDD_CORE_SRCS: &[&str] = &[
    "cudd/cuddAPI.c",
    "cudd/cuddAddAbs.c",
    "cudd/cuddAddApply.c",
    "cudd/cuddAddFind.c",
    "cudd/cuddAddInv.c",
    "cudd/cuddAddIte.c",
    "cudd/cuddAddNeg.c",
    "cudd/cuddAddWalsh.c",
    "cudd/cuddAndAbs.c",
    "cudd/cuddAnneal.c",
    "cudd/cuddApa.c",
    "cudd/cuddApprox.c",
    "cudd/cuddBddAbs.c",
    "cudd/cuddBddCorr.c",
    "cudd/cuddBddIte.c",
    "cudd/cuddBridge.c",
    "cudd/cuddCache.c",
    "cudd/cuddCheck.c",
    "cudd/cuddClip.c",
    "cudd/cuddCof.c",
    "cudd/cuddCompose.c",
    "cudd/cuddDecomp.c",
    "cudd/cuddEssent.c",
    "cudd/cuddExact.c",
    "cudd/cuddExport.c",
    "cudd/cuddGenCof.c",
    "cudd/cuddGenetic.c",
    "cudd/cuddGroup.c",
    "cudd/cuddHarwell.c",
    "cudd/cuddInit.c",
    "cudd/cuddInteract.c",
    "cudd/cuddLCache.c",
    "cudd/cuddLevelQ.c",
    "cudd/cuddLinear.c",
    "cudd/cuddLiteral.c",
    "cudd/cuddMatMult.c",
    "cudd/cuddPriority.c",
    "cudd/cuddRead.c",
    "cudd/cuddRef.c",
    "cudd/cuddReorder.c",
    "cudd/cuddSat.c",
    "cudd/cuddSign.c",
    "cudd/cuddSolve.c",
    "cudd/cuddSplit.c",
    "cudd/cuddSubsetHB.c",
    "cudd/cuddSubsetSP.c",
    "cudd/cuddSymmetry.c",
    "cudd/cuddTable.c",
    "cudd/cuddUtil.c",
    "cudd/cuddWindow.c",
    "cudd/cuddZddCount.c",
    "cudd/cuddZddFuncs.c",
    "cudd/cuddZddGroup.c",
    "cudd/cuddZddIsop.c",
    "cudd/cuddZddLin.c",
    "cudd/cuddZddMisc.c",
    "cudd/cuddZddPort.c",
    "cudd/cuddZddReord.c",
    "cudd/cuddZddSetop.c",
    "cudd/cuddZddSymm.c",
    "cudd/cuddZddUtil.c",
];

const DDDMP_SRCS: &[&str] = &[
    "dddmp/dddmpBinary.c",
    "dddmp/dddmpConvert.c",
    "dddmp/dddmpDbg.c",
    "dddmp/dddmpLoad.c",
    "dddmp/dddmpLoadCnf.c",
    "dddmp/dddmpNodeAdd.c",
    "dddmp/dddmpNodeBdd.c",
    "dddmp/dddmpNodeCnf.c",
    "dddmp/dddmpStoreAdd.c",
    "dddmp/dddmpStoreBdd.c",
    "dddmp/dddmpStoreCnf.c",
    "dddmp/dddmpStoreMisc.c",
    "dddmp/dddmpUtil.c",
];

fn main() {
    let cudd_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cudd-sys lives under third_party/")
        .join("cudd");

    let mut build = cc::Build::new();
    build
        .include(&cudd_root) // `<config.h>` (hand-authored Linux/LP64 substitute)
        .include(cudd_root.join("util"))
        .include(cudd_root.join("epd"))
        .include(cudd_root.join("st"))
        .include(cudd_root.join("mtr"))
        .include(cudd_root.join("cudd"))
        .include(cudd_root.join("dddmp"))
        .define("HAVE_CONFIG_H", "1")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-pthread")
        .warnings(false);

    for srcs in [UTIL_SRCS, EPD_SRCS, ST_SRCS, MTR_SRCS, CUDD_CORE_SRCS, DDDMP_SRCS] {
        for src in srcs {
            build.file(cudd_root.join(src));
            println!("cargo:rerun-if-changed={}", cudd_root.join(src).display());
        }
    }
    println!("cargo:rerun-if-changed={}", cudd_root.join("config.h").display());

    build.compile("cudd");
}
