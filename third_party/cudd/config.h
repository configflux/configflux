/* config.h — hand-authored for the Bazel cc_library wrapping CUDD 3.0.0.
 *
 * The upstream tarball ships `config.h.in`, which autotools fills in via
 * `./configure`. Per ADR-0004 §3 second-pass amendment (2026-05-09,
 * configflux-7hn2), this vendor tree intentionally drops autotools to
 * meet the dep-onboarding ACs (no autotools at build time, no network
 * fetches at build time, build via Bazel cc_library only).
 *
 * Scope. The values below cover the union of `HAVE_*` / `PACKAGE_*` /
 * `SIZEOF_*` macros referenced by the C source files we link
 * (`cudd/`, `mtr/`, `dddmp/`, `st/`, `util/`, `epd/`). The set was
 * derived from `grep -E "HAVE_|PACKAGE_|SIZEOF_|VERSION" cudd/*.c
 * util/*.c mtr/*.c dddmp/*.c st/*.c epd/*.c` against the trimmed tree.
 *
 * Target. Linux on glibc with a C11 toolchain (the only platform the
 * configflux Bazel toolchain currently supports per `MODULE.bazel`).
 * Modern Linux supplies all referenced POSIX headers and functions
 * unconditionally, so each `HAVE_*_H` and `HAVE_*` toggle is set to
 * `1` rather than left undefined.
 *
 * SIZEOF_*. Set for LP64 Linux x86_64 / aarch64 — `int=4`, `long=8`,
 * `void*=8`, `long double=16`. These match every Linux target CUDD
 * upstream's autoconf would produce on the supported toolchain. If
 * future cross-compilation surfaces (e.g. ILP32, Windows), the
 * `cc_library` should switch on `select()` to swap this header.
 *
 * Maintenance note. When bumping CUDD, re-run the grep above against
 * the new source tree and add any new `HAVE_*` macro that appears.
 */

#ifndef CUDD_VENDORED_CONFIG_H
#define CUDD_VENDORED_CONFIG_H

/* Package identification — referenced by cuddInt.h and cuddUtil.c. */
#define PACKAGE          "cudd"
#define PACKAGE_BUGREPORT "Fabio@Colorado.EDU"
#define PACKAGE_NAME     "cudd"
#define PACKAGE_STRING   "cudd 3.0.0"
#define PACKAGE_TARNAME  "cudd"
#define PACKAGE_URL      ""
#define PACKAGE_VERSION  "3.0.0"
#define VERSION          "3.0.0"

/* C standard headers — universally available on a C11 Linux toolchain. */
#define HAVE_ASSERT_H    1
#define HAVE_FLOAT_H     1
#define HAVE_INTTYPES_H  1
#define HAVE_LIMITS_H    1
#define HAVE_MATH_H      1
#define HAVE_MEMORY_H    1
#define HAVE_STDDEF_H    1
#define HAVE_STDINT_H    1
#define HAVE_STDLIB_H    1
#define HAVE_STRINGS_H   1
#define HAVE_STRING_H    1
#define HAVE_SYS_STAT_H  1
#define HAVE_SYS_TYPES_H 1
#define HAVE_DLFCN_H     1
#define STDC_HEADERS     1

/* POSIX headers — universally available on Linux/glibc. */
#define HAVE_SYS_RESOURCE_H 1
#define HAVE_SYS_TIMES_H    1
#define HAVE_SYS_TIME_H     1
#define HAVE_SYS_WAIT_H     1
#define HAVE_UNISTD_H       1

/* C library functions — universally available on Linux/glibc. */
#define HAVE_GETHOSTNAME 1
#define HAVE_GETRLIMIT   1
#define HAVE_GETRUSAGE   1
#define HAVE_POW         1
#define HAVE_POWL        1
#define HAVE_SQRT        1
#define HAVE_STRCHR      1
#define HAVE_STRSTR      1
#define HAVE_SYSCONF     1

/* Threading model. CUDD 3.0.0 uses C11 `_Thread_local` when this is
 * defined, which is supported by every C11 compiler the Bazel toolchain
 * pins. */
#define HAVE_WORKING_THREAD 1

/* Floating-point. Linux/glibc provides IEEE 754 floats unconditionally. */
#define HAVE_IEEE_754 1

/* C language features — required by CUDD's headers and util types. */
#define HAVE__BOOL      1
#define HAVE_PTRDIFF_T  1

/* Integer sizes (bytes). LP64 — Linux x86_64 / aarch64. */
#define SIZEOF_INT         4
#define SIZEOF_LONG        8
#define SIZEOF_LONG_DOUBLE 16
#define SIZEOF_VOID_P      8

#endif /* CUDD_VENDORED_CONFIG_H */
