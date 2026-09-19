// SPDX-License-Identifier: BUSL-1.1
//
// Schema-version guard and artifact-kind detection (ADR-0043 §2/§4, plan §9
// step 3). The explorer is a read-only presentation layer over the ConfigFlux
// pipeline's canonical JSON artifacts. It never invents a version: it checks the
// artifact's own `schema_version` against the single SUPPORTED_VERSIONS constant
// below and, on a mismatch, the app shows an honest error naming both versions
// (never a blank page).
//
// When the product schema version is bumped, update SUPPORTED_VERSIONS here and
// regenerate explorer/fixtures/ (see explorer/fixtures/README.md). The hermetic
// Bazel test //explorer:fixture_schema_test enforces that coupling: it reads
// PRODUCT_SCHEMA_VERSION out of compiler/src/product_api.rs, so it goes red on
// the bump itself — there is no constant to keep in lockstep on the Python side.

/** Product schema versions this build of the explorer can render. */
export const SUPPORTED_VERSIONS = Object.freeze([5]);

/** Stable, human-facing labels for each artifact kind. */
export const KIND = Object.freeze({
  MODEL: "model_summary",
  FACETS: "facets",
  RESOLVE: "resolve",
  EXPLAIN: "explain",
  UNKNOWN: "unknown",
});

/**
 * Classify a parsed artifact by structural shape alone — never by filename, so
 * a renamed fixture still routes correctly. Returns one of KIND.*.
 */
export function detectKind(data) {
  if (Array.isArray(data)) {
    const first = data[0];
    if (first && typeof first === "object" && "facet" in first && "valid_options" in first) {
      return KIND.FACETS;
    }
    return KIND.UNKNOWN;
  }
  if (!data || typeof data !== "object") {
    return KIND.UNKNOWN;
  }
  if ("rejection" in data) {
    return KIND.EXPLAIN;
  }
  if ("summary" in data && "query" in data) {
    return KIND.MODEL;
  }
  if ("resolve_hash" in data || ("resolved_output" in data && "selection_state_hash" in data)) {
    return KIND.RESOLVE;
  }
  return KIND.UNKNOWN;
}

/**
 * Extract the artifact's declared schema_version. For the facets artifact (a
 * JSON array of per-facet results) the version lives on each element; we read
 * the first. Returns null when no version field is present.
 */
export function schemaVersionOf(data) {
  if (Array.isArray(data)) {
    const first = data[0];
    return first && typeof first === "object" && typeof first.schema_version === "number"
      ? first.schema_version
      : null;
  }
  if (data && typeof data === "object" && typeof data.schema_version === "number") {
    return data.schema_version;
  }
  return null;
}

/**
 * The schema-version guard. Returns { ok, found, supported }.
 *   ok=false, found=null   → the artifact declares no schema_version at all.
 *   ok=false, found=<n>    → declares an unsupported version.
 *   ok=true                → declares a supported version.
 * The caller renders an honest error naming BOTH found and supported on ok=false.
 */
export function guardSchemaVersion(data) {
  const found = schemaVersionOf(data);
  if (found === null) {
    return { ok: false, found: null, supported: SUPPORTED_VERSIONS.slice() };
  }
  return {
    ok: SUPPORTED_VERSIONS.includes(found),
    found,
    supported: SUPPORTED_VERSIONS.slice(),
  };
}

/** Human label for a kind, used in headings and error copy. */
export function kindLabel(kind) {
  switch (kind) {
    case KIND.MODEL:
      return "model summary";
    case KIND.FACETS:
      return "facet options";
    case KIND.RESOLVE:
      return "resolved snapshot";
    case KIND.EXPLAIN:
      return "explanation";
    default:
      return "unrecognized artifact";
  }
}
