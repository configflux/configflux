// SPDX-License-Identifier: BUSL-1.1
//
// The environment manifest — a first-class `cfx` input (ADR-0059 D1/D2/M5).
//
// An environment manifest is a small JSON file the USER owns and version
// controls. It gives each deployment target a name and binds it to the three
// free values of a resolution: a scope, a set of context tags, and a set of
// selection choices. The field names are exactly the convention ADR-0032 D1
// documented and `examples/resolve_environment.sh` has read since; ADR-0059
// reverses D1 for `cfx` only and freezes that convention into a parsed schema,
// because a convention with no parser has no way to be wrong out loud — the
// reference script's own header and its own validator already disagreed about
// `schema_version`, and both shipped examples carried private, subtly
// different re-implementations of the loop.
//
// Two things live here and nothing else: the D1 parser, and M5's cell
// enumerator. `resolve_cells` yields the `(environment, scope, context_tags,
// choices)` tuple per cell and nothing more — what to DO with a cell (resolve
// once and write, as `cfx resolve --all` does, or resolve twice and compare, as
// `cfx diff` will) stays with the caller. One enumerator is the point: a `diff`
// visiting a cell set the matrix did not write is a wrong answer with a clean
// exit code.
//
// Narrowing is ONE model (ADR-0059 M5): a SET of environment names drawn from
// the manifest, where the empty set selects every environment. `--environment
// <name>` is the one-element case and `--all` is the empty set spelled
// explicitly. `resolve` requires that explicit spelling because it WRITES FILES
// — a bare `--manifest` must never fan out and populate a tree nobody asked for.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::input_file::read_input_file;
use crate::pipeline::{PipelineError, DEFAULT_SCOPE};

/// The only manifest schema version `cfx` accepts (ADR-0059 D1). All three
/// committed manifests carry it; the reference script's validator requires it.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// A parsed environment manifest.
///
/// `deny_unknown_fields` is the whole point of parsing this file rather than
/// reading it with `jq`: an unknown key is a typo or a stale field, and either
/// silently resolves a target the author did not describe.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentManifest {
    pub schema_version: u32,
    pub environments: BTreeMap<String, Environment>,
    /// The file this manifest was read from, for diagnostics. `skip` keeps it
    /// out of the accepted key set, so a manifest declaring `source` is still
    /// rejected as an unknown field.
    #[serde(skip)]
    source: String,
}

/// One named deployment target.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// The resolution scope. Absent means the whole-model `all` scope, the
    /// same default a `cfx resolve` with no selection file takes.
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    /// Accepted and IGNORED (ADR-0059 D1). The target's class is a deploy-guard
    /// concern owned by `examples/deploy_guard.sh` (ADR-0035); `cfx` must
    /// neither restate nor reinterpret its fail-safe rule. The field is
    /// load-bearing by merely EXISTING: without it `deny_unknown_fields` would
    /// reject every manifest declaring a class, including the committed
    /// `examples/05-compose-fleet/environments.json`. Never read, on purpose.
    #[serde(default)]
    #[allow(dead_code)]
    pub class: Option<String>,
}

/// One `(environment, scope)` resolution — the unit both `cfx resolve --all`
/// and `cfx diff` enumerate.
#[derive(Debug, Clone)]
pub struct Cell {
    pub environment: String,
    pub scope: String,
    pub context_tags: BTreeMap<String, String>,
    pub choices: BTreeMap<String, String>,
}

/// The cell set one `cfx resolve --manifest` invocation addresses.
pub struct CellPlan {
    /// Sorted `(environment, scope)` — ADR-0059 M1.
    pub cells: Vec<Cell>,
    /// `--all`: write each cell under `<out>/<environment>/<root>/`. A single
    /// `--environment` keeps the flat `<out>` layout (M4).
    pub nested: bool,
}

/// Names become path components, so only the file-safe token set is accepted
/// (ADR-0059 D1).
fn is_valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Read and parse a manifest, then validate the frozen schema.
pub fn load(path: &Path) -> Result<EnvironmentManifest, PipelineError> {
    let bytes = read_input_file(path, "--manifest")?;
    let mut manifest: EnvironmentManifest =
        serde_json::from_slice(&bytes).map_err(|err| malformed(path, &bytes, err))?;
    manifest.source = path.display().to_string();
    manifest.validate()?;
    Ok(manifest)
}

/// Turn a parse failure into a usage error naming the file. A structurally valid
/// document whose `environments` is not an object gets a targeted message,
/// because serde's own "invalid type" text does not name the field.
fn malformed(path: &Path, bytes: &[u8], err: serde_json::Error) -> PipelineError {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if value.get("environments").is_some_and(|envs| !envs.is_object()) {
            return PipelineError::usage(format!(
                "malformed environment manifest '{}': 'environments' must be an object",
                path.display()
            ));
        }
    }
    PipelineError::usage(format!(
        "malformed environment manifest '{}' ({err})",
        path.display()
    ))
}

impl EnvironmentManifest {
    fn validate(&self) -> Result<(), PipelineError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(PipelineError::usage(format!(
                "unsupported environment manifest '{}': schema_version is {}, expected {}",
                self.source, self.schema_version, MANIFEST_SCHEMA_VERSION
            )));
        }
        for name in self.environments.keys() {
            if !is_valid_environment_name(name) {
                return Err(PipelineError::usage(format!(
                    "invalid environment name '{name}' in '{}': a name becomes a path \
                     component and must match [A-Za-z0-9._-]+",
                    self.source
                )));
            }
        }
        Ok(())
    }

    /// The manifest's environment names, sorted, as one comma-joined line.
    fn available(&self) -> String {
        self.environments
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Enumerate the cells a narrowing addresses (ADR-0059 M5).
    ///
    /// `environments` is the narrowing SET: empty selects every environment,
    /// one name is the `--environment` case, and `cfx diff` will pass several.
    /// `scopes`, when present, REPLACES each environment's own scope, so one
    /// manifest sweeps several service scopes without being edited.
    ///
    /// Cells come out fully sorted on BOTH axes (ADR-0059 M1). That is not
    /// cosmetic: cell order is product output — the order of the JSON Lines
    /// stream and of a diff report's cells — and ADR-0042 §3 requires identical
    /// stdout across runs, so reordering `--scopes` must not reorder a report.
    pub fn resolve_cells(
        &self,
        environments: &[String],
        scopes: Option<&[String]>,
    ) -> Result<Vec<Cell>, PipelineError> {
        let selected: Vec<&String> = if environments.is_empty() {
            // BTreeMap keys are already sorted.
            self.environments.keys().collect()
        } else {
            let mut names = Vec::with_capacity(environments.len());
            for name in environments {
                if !self.environments.contains_key(name) {
                    return Err(PipelineError::usage(format!(
                        "environment '{name}' is not defined in '{}'; available: {}",
                        self.source,
                        self.available()
                    )));
                }
                names.push(name);
            }
            names.sort();
            names.dedup();
            names
        };

        let mut cells = Vec::new();
        for name in selected {
            let environment = &self.environments[name];
            let own_scope = environment.scope.as_deref().unwrap_or(DEFAULT_SCOPE);
            let cell_scopes: Vec<&str> = match scopes {
                Some(list) => list.iter().map(String::as_str).collect(),
                None => vec![own_scope],
            };
            for scope in cell_scopes {
                cells.push(Cell {
                    environment: name.clone(),
                    scope: scope.to_string(),
                    context_tags: environment.context_tags.clone(),
                    choices: environment.choices.clone(),
                });
            }
        }
        Ok(cells)
    }
}

/// Split a `--scopes` comma list into the sorted, de-duplicated scope axis.
/// Sorting here is what makes the M1 ordering hold regardless of the order the
/// caller typed the flag in.
pub(crate) fn parse_scopes(raw: &str) -> Result<Vec<String>, PipelineError> {
    let mut scopes: Vec<String> = Vec::new();
    for entry in raw.split(',') {
        let scope = entry.trim();
        if scope.is_empty() {
            return Err(PipelineError::usage(format!(
                "invalid --scopes '{raw}': every entry must be a non-empty scope selector"
            )));
        }
        scopes.push(scope.to_string());
    }
    scopes.sort();
    scopes.dedup();
    Ok(scopes)
}

/// Validate the manifest flag combination and, when a manifest is in play,
/// enumerate the cells it addresses.
///
/// `Ok(None)` means no manifest was given and the caller keeps its existing
/// `--selection-file` / flags-only path.
pub fn plan(
    manifest: Option<&Path>,
    environment: Option<&str>,
    all: bool,
    scopes: Option<&str>,
    selection_file: Option<&Path>,
) -> Result<Option<CellPlan>, PipelineError> {
    let Some(path) = manifest else {
        if environment.is_some() || all || scopes.is_some() {
            return Err(PipelineError::usage(
                "--environment, --all and --scopes require --manifest <PATH>",
            ));
        }
        return Ok(None);
    };

    // Two spellings of the same three inputs cannot both be authoritative, and
    // silently preferring one would resolve a target the user did not ask for.
    if selection_file.is_some() {
        return Err(PipelineError::usage(
            "--manifest and --selection-file are mutually exclusive: each supplies the scope, \
             context tags and choices, so give exactly one",
        ));
    }

    match (environment, all) {
        (Some(_), true) => {
            return Err(PipelineError::usage(
                "--environment and --all are mutually exclusive: name one environment, or use \
                 --all to resolve every environment in the manifest",
            ))
        }
        (None, false) => {
            return Err(PipelineError::usage(
                "--manifest requires exactly one of --environment <NAME> or --all",
            ))
        }
        _ => {}
    }

    // `--all` is the required opt-in for a fleet-wide fan-out, so `--scopes` —
    // which replaces every environment's own scope — has a meaning only there.
    // A single `--environment` resolves the one scope its manifest entry names.
    let scopes = match scopes {
        Some(raw) => {
            if !all {
                return Err(PipelineError::usage(
                    "--scopes applies only with --all: a single --environment resolves the scope \
                     its manifest entry names",
                ));
            }
            Some(parse_scopes(raw)?)
        }
        None => None,
    };

    let manifest = load(path)?;
    let narrowing: Vec<String> = environment.map(|name| vec![name.to_string()]).unwrap_or_default();
    let cells = manifest.resolve_cells(&narrowing, scopes.as_deref())?;
    Ok(Some(CellPlan { cells, nested: all }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> EnvironmentManifest {
        let mut parsed: EnvironmentManifest = serde_json::from_str(json).expect("manifest parses");
        parsed.source = "environments.json".to_string();
        parsed
    }

    const TWO_ENVIRONMENTS: &str = r#"{"schema_version":1,"environments":{
        "robot-alpha":{"scope":"component:vision","choices":{"tier":"edge"}},
        "local":{"scope":"component:telemetry","context_tags":{"site":"lab"},"class":"local"}}}"#;

    #[test]
    fn empty_narrowing_selects_every_environment_sorted() {
        let cells = manifest(TWO_ENVIRONMENTS)
            .resolve_cells(&[], None)
            .expect("enumerates");
        let seen: Vec<(&str, &str)> = cells
            .iter()
            .map(|c| (c.environment.as_str(), c.scope.as_str()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("local", "component:telemetry"),
                ("robot-alpha", "component:vision")
            ]
        );
        // `class` is carried by the parser and never reaches a cell.
        assert_eq!(cells[0].context_tags["site"], "lab");
    }

    #[test]
    fn scopes_override_each_environments_own_scope_and_sort() {
        // Passed vision-first; cells must still come out sorted on both axes.
        let scopes = parse_scopes("component:vision,component:audio").expect("valid list");
        let cells = manifest(TWO_ENVIRONMENTS)
            .resolve_cells(&[], Some(&scopes))
            .expect("enumerates");
        let seen: Vec<(&str, &str)> = cells
            .iter()
            .map(|c| (c.environment.as_str(), c.scope.as_str()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("local", "component:audio"),
                ("local", "component:vision"),
                ("robot-alpha", "component:audio"),
                ("robot-alpha", "component:vision"),
            ]
        );
    }

    #[test]
    fn a_missing_scope_defaults_to_the_whole_model() {
        let cells = manifest(r#"{"schema_version":1,"environments":{"bare":{}}}"#)
            .resolve_cells(&[], None)
            .expect("enumerates");
        assert_eq!(cells[0].scope, DEFAULT_SCOPE);
        assert!(cells[0].choices.is_empty());
    }

    #[test]
    fn an_unknown_narrowing_name_lists_the_available_ones() {
        let err = manifest(TWO_ENVIRONMENTS)
            .resolve_cells(&["prod".to_string()], None)
            .expect_err("unknown name must be refused");
        assert_eq!(err.exit_code, crate::EXIT_USAGE);
        assert!(err.message.contains("prod"), "{}", err.message);
        assert!(
            err.message.contains("local, robot-alpha"),
            "must list the available names sorted: {}",
            err.message
        );
    }

    #[test]
    fn parse_scopes_rejects_an_empty_entry() {
        assert!(parse_scopes("all,").is_err());
        assert!(parse_scopes("").is_err());
        assert_eq!(parse_scopes(" all , all ").unwrap(), vec!["all".to_string()]);
    }
}
