// SPDX-License-Identifier: BUSL-1.1
//
// Presentation for `cfx resolve` (ADR-0042 §3). Text output is the default,
// human-facing hash lineage; JSON output reuses the existing `ResolveResult`
// operation schema byte-for-byte (no new JSON shape is invented). Output is
// deterministic: no timestamps, no absolute paths — the `wrote:` lines carry
// the artifact-relative paths, already sorted by the pipeline.

use std::io::Write;

use compiler::loader_api::{ExplainRejectionResult, GetSelectionOptionsResult, ResolveResult};

use crate::explain::ResolveContextConflict;
use crate::options::OptionsOutcome;
use crate::pipeline::ResolveOutcome;

/// Render the default human text lineage: one line per stage plus one
/// `wrote:` line per exported file (sorted). Determinism-safe.
pub fn render_text<W: Write>(outcome: &ResolveOutcome, out: &mut W) -> std::io::Result<()> {
    writeln!(out, "model_hash: {}", outcome.model_hash)?;
    writeln!(out, "selection_state_hash: {}", outcome.selection_state_hash)?;
    writeln!(out, "resolve_hash: {}", outcome.resolve_hash)?;
    // ADR-0047 §5/§6: surface the auto-bound-default provenance. Sorted
    // (`BTreeMap`), determinism-safe, and absent entirely for a facet-free
    // resolve (empty map → no lines), so pre-ADR output is byte-unchanged.
    for (facet, value) in &outcome.resolve_result.defaulted_choices {
        writeln!(out, "defaulted: {facet}={value}")?;
    }
    for path in &outcome.written {
        writeln!(out, "wrote: {path}")?;
    }
    Ok(())
}

/// Render the resolved payload as JSON — the existing `ResolveResult` schema,
/// serialized exactly as the interpreter's `resolve` op emits it, with a
/// trailing newline.
pub fn render_json<W: Write>(outcome: &ResolveOutcome, out: &mut W) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(&outcome.resolve_result)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    bytes.push(b'\n');
    out.write_all(&bytes)
}

/// Render the `cfx options` human text: one block per facet (sorted), a header
/// line `facet <name> [selected: <option>]` (or `[open]`), then the valid
/// options indented two spaces (already sorted by the loader). Determinism-safe:
/// no hashes, no timestamps, no absolute paths.
pub fn render_options_text<W: Write>(
    outcome: &OptionsOutcome,
    out: &mut W,
) -> std::io::Result<()> {
    for listing in &outcome.facets {
        // ADR-0047 §6: annotate a declared facet's default arm. `default` is
        // skip-if-none, so an undeclared / default-less facet appends nothing
        // and its header is byte-identical to the pre-ADR-0047 form.
        let default_suffix = match &listing.result.default {
            Some(default) => format!(", default: {default}"),
            None => String::new(),
        };
        match &listing.selected {
            // A selection has been made: the selection-state token names the
            // chosen option. Schema kind is not repeated here.
            Some(option) => {
                writeln!(out, "facet {} [selected: {}{}]", listing.facet, option, default_suffix)?
            }
            // No selection yet. ADR-0047 §6 (Amendment 1): render the truthful
            // schema-kind token from `declared_open` — `closed`/`open` for a
            // declared facet — instead of the bare `[open]` that used to mean
            // "unselected" and collided with the schema flag. An undeclared
            // facet has no schema kind (`None`) and keeps today's `[open]`
            // unselected marker, so its output is byte-identical.
            None => {
                let kind_token = match listing.result.declared_open {
                    Some(false) => "closed",
                    Some(true) | None => "open",
                };
                writeln!(out, "facet {} [{}{}]", listing.facet, kind_token, default_suffix)?
            }
        }
        for option in &listing.result.valid_options {
            writeln!(out, "  {option}")?;
        }
    }
    Ok(())
}

/// Render the `cfx options` JSON: the EXISTING per-facet `GetSelectionOptionsResult`
/// objects UNMODIFIED (ADR-0042 §3), as a JSON array in facet order with a
/// trailing newline. The `Vec` of result structs is serialized DIRECTLY — never
/// via `serde_json::Value`, whose map would re-sort keys alphabetically — so each
/// array element is byte-identical to the interpreter `options` envelope response
/// for that facet.
pub fn render_options_json<W: Write>(
    outcome: &OptionsOutcome,
    out: &mut W,
) -> std::io::Result<()> {
    let results: Vec<&GetSelectionOptionsResult> =
        outcome.facets.iter().map(|listing| &listing.result).collect();
    let mut bytes = serde_json::to_vec(&results)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    bytes.push(b'\n');
    out.write_all(&bytes)
}

/// Render `cfx explain` as human text. A genuine conflict renders the shared
/// unsat-core block via the extracted `explain_renderer::render_unsat_core` (the
/// SAME renderer the interpreter carries — ADR-0042 §2, configflux-2awb.3), so
/// the human wording is defined in exactly one place. A rejection that carries
/// no solver core (a division-of-labor rejection, e.g. an invalid option —
/// ADR-0031 D2) has nothing to minimize, so its canonical message is shown
/// instead. Determinism-safe: no hashes, no timestamps, no absolute paths.
pub fn render_explain_text<W: Write>(
    result: &ExplainRejectionResult,
    out: &mut W,
) -> std::io::Result<()> {
    match &result.rejection.unsat_core {
        Some(core) => writeln!(out, "{}", explain_renderer::render_unsat_core(core)),
        None => writeln!(out, "{}", result.rejection.message),
    }
}

/// Emit the `cfx explain` JSON — the EXISTING `ExplainRejectionResult` schema
/// UNMODIFIED (ADR-0042 §3), serialized exactly as the interpreter `explain`
/// envelope path emits it (`serde_json::to_vec` of the same struct, then a
/// trailing newline), so the bytes are identical to that path for the same
/// request. Serialized DIRECTLY from the struct — never via `serde_json::Value`,
/// whose map would re-order keys.
pub fn render_explain_json<W: Write>(
    result: &ExplainRejectionResult,
    out: &mut W,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(result)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    bytes.push(b'\n');
    out.write_all(&bytes)
}

/// Dispatch the resolve-context explanation (configflux-sc69) to JSON (the
/// unmodified `ResolveResult` envelope) or human text (the named unbound tags).
pub fn render_resolve_context<W: Write>(
    conflict: &ResolveContextConflict,
    json: bool,
    out: &mut W,
) -> std::io::Result<()> {
    if json {
        render_resolve_context_json(&conflict.resolve_result, out)
    } else {
        render_resolve_context_text(conflict, out)
    }
}

/// Render a resolve-context unsatisfiability as human text (configflux-sc69):
/// the same "unsatisfiable" verdict `cfx resolve` reaches, but EXPLAINED — name
/// the tag(s) an active condition needs that no choice or context tag binds, and
/// echo the resolve diagnostic (which carries the failing condition) and its
/// hint. Determinism-safe: no hashes, no timestamps, no absolute paths.
pub fn render_resolve_context_text<W: Write>(
    conflict: &ResolveContextConflict,
    out: &mut W,
) -> std::io::Result<()> {
    writeln!(
        out,
        "selection is unsatisfiable: an active model condition references a tag that no selection binds"
    )?;
    if !conflict.unbound_tags.is_empty() {
        writeln!(out, "  unbound tag(s): {}", conflict.unbound_tags.join(", "))?;
    }
    if let Some(diag) = conflict.resolve_result.diagnostics.diagnostics.first() {
        writeln!(out, "  {}", diag.message)?;
        if let Some(hint) = &diag.hint {
            writeln!(out, "  hint: {hint}")?;
        }
    }
    Ok(())
}

/// Emit the resolve-context failure as JSON — the EXISTING `ResolveResult`
/// schema UNMODIFIED (ADR-0042 §3), serialized exactly as `cfx resolve` and the
/// interpreter `resolve` op emit it, with a trailing newline. Serialized DIRECTLY
/// from the struct — never via `serde_json::Value`, whose map would re-order keys.
pub fn render_resolve_context_json<W: Write>(
    result: &ResolveResult,
    out: &mut W,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(result)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    bytes.push(b'\n');
    out.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compiler::loader_api::ResolveResult;
    use compiler::product_api::{
        Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
    };
    use std::collections::BTreeMap;

    fn outcome() -> ResolveOutcome {
        ResolveOutcome {
            model_hash: "aaaa".to_string(),
            selection_state_hash: "bbbb".to_string(),
            resolve_hash: "cccc".to_string(),
            written: vec![
                "generated/config.hpp".to_string(),
                "generated/config_build_flags.cmake".to_string(),
            ],
            resolve_result: ResolveResult {
                schema_version: PRODUCT_SCHEMA_VERSION,
                status: OperationStatus::Ok,
                model_hash: "aaaa".to_string(),
                scope: "all".to_string(),
                selection_state_hash: "bbbb".to_string(),
                resolve_hash: Some("cccc".to_string()),
                resolved_output: Some(serde_json::json!({"k": "v"})),
                context_tags: BTreeMap::new(),
                choices: BTreeMap::new(),
                defaulted_choices: BTreeMap::new(),
                resolved_component_dependencies: BTreeMap::new(),
                resolved_artifacts: BTreeMap::new(),
                error_count: 0,
                warning_count: 0,
                diagnostics_ref: None,
                diagnostics: DiagnosticsReport {
                    schema_version: PRODUCT_SCHEMA_VERSION,
                    diagnostics: Vec::new(),
                    error_count: 0,
                    warning_count: 0,
                },
            },
        }
    }

    #[test]
    fn text_lineage_is_ordered_and_relative() {
        let mut buf = Vec::new();
        render_text(&outcome(), &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(
            text,
            "model_hash: aaaa\n\
             selection_state_hash: bbbb\n\
             resolve_hash: cccc\n\
             wrote: generated/config.hpp\n\
             wrote: generated/config_build_flags.cmake\n"
        );
    }

    #[test]
    fn json_is_resolve_result_with_trailing_newline() {
        let mut buf = Vec::new();
        render_json(&outcome(), &mut buf).unwrap();
        assert!(buf.ends_with(b"\n"));
        let value: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(value["resolve_hash"], "cccc");
        assert_eq!(value["status"], "ok");
    }

    fn options_result(facet: &str, valid: &[&str]) -> GetSelectionOptionsResult {
        options_result_declared(facet, valid, None, None)
    }

    fn options_result_declared(
        facet: &str,
        valid: &[&str],
        default: Option<&str>,
        declared_open: Option<bool>,
    ) -> GetSelectionOptionsResult {
        GetSelectionOptionsResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Ok,
            model_hash: "aaaa".to_string(),
            scope: "all".to_string(),
            facet: facet.to_string(),
            valid_options: valid.iter().map(|s| s.to_string()).collect(),
            default: default.map(str::to_string),
            declared_open,
            pruned_options: None,
            selection_state_hash: "bbbb".to_string(),
            error_count: 0,
            warning_count: 0,
            diagnostics_ref: None,
            diagnostics: DiagnosticsReport {
                schema_version: PRODUCT_SCHEMA_VERSION,
                diagnostics: Vec::new(),
                error_count: 0,
                warning_count: 0,
            },
        }
    }

    fn options_outcome() -> OptionsOutcome {
        OptionsOutcome {
            facets: vec![
                crate::options::FacetListing {
                    facet: "cooling_brand".to_string(),
                    selected: Some("hydra".to_string()),
                    result: options_result("cooling_brand", &["aeroflux", "hydra"]),
                },
                crate::options::FacetListing {
                    facet: "cooling_model".to_string(),
                    selected: None,
                    result: options_result("cooling_model", &["x200"]),
                },
            ],
        }
    }

    #[test]
    fn options_text_blocks_mark_selected_and_open() {
        // Undeclared facets (declared_open: None): the unselected marker stays
        // the byte-identical pre-ADR-0047 `[open]`.
        let mut buf = Vec::new();
        render_options_text(&options_outcome(), &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(
            text,
            "facet cooling_brand [selected: hydra]\n\
             \x20 aeroflux\n\
             \x20 hydra\n\
             facet cooling_model [open]\n\
             \x20 x200\n"
        );
    }

    #[test]
    fn options_text_blocks_render_truthful_schema_kind_for_declared_facets() {
        // ADR-0047 §6 (Amendment 1): a declared CLOSED facet with no selection
        // renders `[closed, default: ...]`, not the old collided `[open]`; a
        // declared OPEN facet renders `[open, default: ...]`; a selected facet
        // shows only the selection-state token.
        let outcome = OptionsOutcome {
            facets: vec![
                crate::options::FacetListing {
                    facet: "bus_type".to_string(),
                    selected: None,
                    result: options_result_declared(
                        "bus_type",
                        &["ethernet", "serial"],
                        Some("serial"),
                        Some(false),
                    ),
                },
                crate::options::FacetListing {
                    facet: "plugins".to_string(),
                    selected: None,
                    result: options_result_declared(
                        "plugins",
                        &["metrics"],
                        Some("metrics"),
                        Some(true),
                    ),
                },
                crate::options::FacetListing {
                    facet: "bus_type".to_string(),
                    selected: Some("ethernet".to_string()),
                    result: options_result_declared(
                        "bus_type",
                        &["ethernet", "serial"],
                        Some("serial"),
                        Some(false),
                    ),
                },
            ],
        };
        let mut buf = Vec::new();
        render_options_text(&outcome, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(
            text,
            "facet bus_type [closed, default: serial]\n\
             \x20 ethernet\n\
             \x20 serial\n\
             facet plugins [open, default: metrics]\n\
             \x20 metrics\n\
             facet bus_type [selected: ethernet, default: serial]\n\
             \x20 ethernet\n\
             \x20 serial\n"
        );
    }

    #[test]
    fn options_json_is_array_of_unmodified_results() {
        let mut buf = Vec::new();
        render_options_json(&options_outcome(), &mut buf).unwrap();
        assert!(buf.ends_with(b"\n"));
        let value: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        let arr = value.as_array().expect("options JSON is an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["facet"], "cooling_brand");
        assert_eq!(arr[0]["valid_options"][0], "aeroflux");
        // Unmodified per-facet result: no pruned_options key (skipped when None).
        assert!(arr[0].get("pruned_options").is_none());
        assert_eq!(arr[1]["facet"], "cooling_model");
    }

    fn context_conflict() -> ResolveContextConflict {
        ResolveContextConflict {
            resolve_result: ResolveResult {
                schema_version: PRODUCT_SCHEMA_VERSION,
                status: OperationStatus::Error,
                model_hash: "aaaa".to_string(),
                scope: "all".to_string(),
                selection_state_hash: "bbbb".to_string(),
                resolve_hash: None,
                resolved_output: None,
                context_tags: BTreeMap::new(),
                choices: BTreeMap::new(),
                defaulted_choices: BTreeMap::new(),
                resolved_component_dependencies: BTreeMap::new(),
                resolved_artifacts: BTreeMap::new(),
                error_count: 1,
                warning_count: 0,
                diagnostics_ref: None,
                diagnostics: DiagnosticsReport {
                    schema_version: PRODUCT_SCHEMA_VERSION,
                    diagnostics: vec![Diagnostic {
                        code: "E_RESOLVE_CONTEXT_UNSATISFIED".to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: "Failed to evaluate condition: 'region == 'eu''".to_string(),
                        source_id: None,
                        entity_path: None,
                        hint: Some("Provide all required selection choices/context_tags for active conditions".to_string()),
                    }],
                    error_count: 1,
                    warning_count: 0,
                },
            },
            unbound_tags: vec!["region".to_string()],
        }
    }

    #[test]
    fn resolve_context_text_names_tag_and_agrees_with_resolve() {
        let mut buf = Vec::new();
        render_resolve_context_text(&context_conflict(), &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        // Must AGREE with resolve (unsatisfiable) — never claim "nothing to explain".
        assert!(text.contains("unsatisfiable"), "{text}");
        assert!(!text.contains("nothing to explain"), "{text}");
        // Must actually explain: name the unbound tag and echo the condition.
        assert!(text.contains("unbound tag(s): region"), "{text}");
        assert!(text.contains("region == 'eu'"), "{text}");
        assert!(text.contains("hint:"), "{text}");
    }

    #[test]
    fn resolve_context_json_is_unmodified_resolve_result() {
        let mut buf = Vec::new();
        render_resolve_context_json(&context_conflict().resolve_result, &mut buf).unwrap();
        assert!(buf.ends_with(b"\n"));
        let value: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(value["status"], "error");
        assert_eq!(
            value["diagnostics"]["diagnostics"][0]["code"],
            "E_RESOLVE_CONTEXT_UNSATISFIED"
        );
    }
}
