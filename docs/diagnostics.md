# Diagnostic Codes

<!-- Generated file - do not edit by hand. The cause and remedy text for each
     code lives in a doc comment beside that code's definition in the crate
     sources; edit it there and regenerate this file. -->

Every error ConfigFlux reports carries a stable diagnostic code, such as
`E_RESOLVE_FACET_UNBOUND`. The code is part of the interface contract: it does
not change when a message is reworded, so it is safe to branch on, log, and
search for.

This page lists every code the compiler, interpreter, and runtime can emit,
what causes it, and what to do about it. Codes are grouped by the stage that
raises them, and each row names the code's family — the shared prefix that
tells you which stage a code came from when you meet it out of context.

A code alone is rarely the whole story: the diagnostic that carries it also
carries a message naming the specific facet, path, or value involved, and often
a hint. Read those first; the table below is for when you need the general
shape of the failure and the standard way out of it.

The registry currently defines 77 codes.

## Ingestion

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_INGEST_DUPLICATE_FACET` | `INGEST` | the same facet is declared in more than one source chunk, and a facet is a pack-global domain that may have only one declaring chunk | keep the declaration in exactly one chunk and let the others reference the facet without redeclaring it |

## Facets

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_FACET_VALUE_UNDECLARED` | `FACET` | a constraint names a facet the model never declares, or a condition or constraint names a value that is not in a closed facet's exhaustively declared domain | declare the missing facet with its value domain, or drop it from the constraint; for an undeclared value, add it to the facet's declared values, mark the facet open if its domain is genuinely extensible, or correct the reference to use a declared value |

## Component graph

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_COMPONENT_DEP_CYCLE` | `COMPONENT` | the component dependency graph contains a cycle, so no valid build or initialization order exists | break the cycle the diagnostic traces, so that the dependency graph is acyclic |
| `E_COMPONENT_DEP_DIAMOND` | `COMPONENT` | no current code path emits this code; diamond-shaped dependencies are permitted and the component graph may be any acyclic graph | no action is needed: the code is reserved and never reused so that the registry stays a stable contract, and a diamond dependency is accepted |
| `E_UNKNOWN_COMPONENT_DEP` | `COMPONENT` | a component's depends_on entry names a component identifier that does not exist in the model | correct the identifier or add the missing component; every depends_on target must resolve to a component the model defines |

## Compilation

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_COMPILE_EMIT_FAILED` | `COMPILE` | the model verified successfully but its artifacts could not be written, which is almost always a permissions or filesystem problem on the output directory | choose a writable output directory and check its ownership and mount options; the diagnostic's hint names the specific filesystem condition |
| `E_COMPILE_INPUT_INVALID` | `COMPILE` | a source chunk could not be ingested, or the model failed link and verification for a reason outside the dependency, cycle, and closed-facet families | read the wrapped message: it names the offending source and the schema or structural rule the input broke |
| `E_UNSUPPORTED_SCHEMA_VERSION` | `COMPILE` | the request's schema_version is not the version this build implements, and older versions are rejected rather than silently adapted | set schema_version to the version this binary reports, or use a binary built for the version your caller targets |

## Model inspection

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_INSPECT_QUERY_INVALID` | `INSPECT` | the inspect query is malformed: an identifier is not a valid snake-case name, the scope string does not parse, or a parameter's metadata could not be materialized | correct the identifier or scope syntax; if the failure is in materializing parameter metadata, check the parameter's definition chain for a broken link |
| `E_INSPECT_UNKNOWN_ARTIFACT` | `INSPECT` | the inspect query names an artifact identifier the model does not define | check the identifier for typos and list the model's artifacts to find the one you meant |
| `E_INSPECT_UNKNOWN_COMPONENT` | `INSPECT` | the inspect query names a component identifier the model does not define | check the identifier for typos and list the model's components to find the one you meant |
| `E_INSPECT_UNKNOWN_DEFINITION` | `INSPECT` | the inspect query names a definition identifier the model does not define | check the identifier for typos and list the model's definitions to find the one you meant |
| `E_INSPECT_UNKNOWN_PARAMETER` | `INSPECT` | the component in the inspect query exists, but it declares no parameter by the requested key | inspect the component first to list the parameter keys it actually declares |
| `E_INSPECT_UNKNOWN_SCOPE` | `INSPECT` | the inspect scope names a component that does not exist, names a component that is not a platform where a platform was required, or matches no components at all | use a scope whose component exists and has the expected type; a platform selector must name a component declared as a platform |

## Model package loading

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_LOADER_INDEX_INVALID` | `LOADER` | the package index failed to load, its recorded config hash does not match the hash computed from its contents, or a chunk file it names is missing or modified | restore the complete, unmodified package directory or recompile the model: every chunk file the index names must be present and byte-identical |
| `E_LOADER_MANIFEST_INCONSISTENT` | `LOADER` | the manifest parses but disagrees with the package it describes: an unsupported manifest version, a mismatched IR format, hash algorithm or model hash, or recorded statistics that do not match the index | recompile the model to regenerate a self-consistent package, and do not hand-edit a manifest or mix files from separate compilations |
| `E_LOADER_MANIFEST_INVALID` | `LOADER` | the compiled model package manifest could not be read or parsed as JSON, so the package cannot be opened | point the model handle at a manifest produced by a successful compile, and recompile the model if the file is damaged |
| `E_LOADER_UNSUPPORTED_SCHEMA_VERSION` | `LOADER` | the request's schema_version is not the version this build implements | set schema_version to the version this binary reports, or use a binary built for the version your caller targets |

## Selection

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_SELECTION_CONFLICT` | `SELECTION` | the choice contradicts something already fixed: an immutable context tag, an earlier selection of the same facet, or a declared constraint the choice would violate | read the conflict the diagnostic names and drop or change the earlier choice; a constraint violation identifies the rule under an entity path of constraints/<id> |
| `E_SELECTION_ENGINE_DIVERGENCE` | `SELECTION` | the solver faulted while adjudicating the request, or rejected a selection the model's own semantics accept; the inputs are not at fault | this is a defect rather than a usage error: re-run with the same inputs to confirm, then report it with the model package and the exact sequence of selections |
| `E_SELECTION_INVALID_OPTION` | `SELECTION` | the facet exists, but the requested option is not a member of that facet's declared domain | choose one of the options the diagnostic lists, or add the value to the facet's domain in the model source and recompile |
| `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` | `SELECTION` | selection could not reach a usable solver model: the package's solver-model reference is empty, the artifact will not load, or it carries no symbol table | recompile the model so a complete solver model is emitted beside the package, and keep the two together whenever the package is copied or moved |
| `E_SELECTION_STATE_INVALID` | `SELECTION` | the supplied selection state is not consistent with the open model: its model hash or scope differs, its recorded state hash does not match its contents, or a choice contradicts a context tag | start from a fresh selection state initialized against the model you opened, and pass it back unmodified between calls |
| `E_SELECTION_UNKNOWN_FACET` | `SELECTION` | the requested facet name is blank, or no facet by that name is declared or discovered anywhere in the opened model | check the name for typos and list the model's facets first; a facet must exist in the compiled model before it can be selected |
| `E_SELECTION_UNSATISFIABLE` | `SELECTION` | the option is valid on its own, but once applied no assignment of the remaining facets satisfies the model | query the valid options for the facet before choosing, or run explain to see the minimal set of choices that conflict |

## Resolution

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_RESOLVE_CONTEXT_UNSATISFIED` | `RESOLVE` | an active condition references a facet or tag that nothing in the selection binds, so the condition cannot be evaluated | supply the missing facet as an explicit choice or as a context tag before resolving |
| `E_RESOLVE_FACET_UNBOUND` | `RESOLVE` | a declared facet with no default is left unbound while an active condition requires it, so the model is satisfiable but underspecified | bind the facet the diagnostic names to one of the values in its reported domain, or give that facet a default in the model source |
| `E_RESOLVE_FAILED` | `RESOLVE` | resolution failed for a reason outside the scope and context families, or the resolved output could not be canonically serialized for hashing | read the wrapped message for the underlying cause; report a serialization failure with the model package, since deterministic hashing must succeed |
| `E_RESOLVE_MODEL_INVALID` | `RESOLVE` | the model could not be loaded for resolution, or a declared constraint carries an expression the resolver cannot parse, so resolution fails closed rather than skipping the rule | recompile the model with the current toolchain, and correct any constraint expression the diagnostic names |
| `E_RESOLVE_SCOPE_INVALID` | `RESOLVE` | the scope selector could not be parsed, or names a form the resolver does not recognize | use a supported selector such as component:<id>, platform:<id>, platform:all, or all |
| `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` | `RESOLVE` | resolution could not reach a usable solver model to check satisfiability, or the solver faulted while replaying the committed choices | recompile the model so a complete solver model is emitted beside the package, and keep the two together when the package is moved |

## Export

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_EXPORT_ARTIFACT_INVALID` | `EXPORT` | a construction-lifecycle parameter typed as an artifact holds something other than a non-empty artifact identifier string | give the parameter the diagnostic names a valid artifact identifier in the model source, then recompile and resolve before exporting again |
| `E_EXPORT_FAILED` | `EXPORT` | the export artifacts could not be produced, most often because a resolved floating-point value is not finite and has no deterministic representation | replace any not-a-number or infinite value in the model with a finite one; report other failures with the resolved output that produced them |
| `E_EXPORT_PROFILE_INVALID` | `EXPORT` | the requested export profile is not one this build supports | use the early-binding profile named in the export contract; it is the only profile this version accepts |
| `E_EXPORT_RESOLVE_INVALID` | `EXPORT` | the resolve result handed to export is unusable: a wrong schema version, a status other than success, a missing resolve hash, or resolved output that cannot be decoded | pass the complete, unmodified result of a successful resolve rather than a hand-assembled or partially copied structure |
| `E_EXPORT_SYMBOL_INVALID` | `EXPORT` | a generated C++ or CMake symbol is not a valid identifier, or two parameters generate the same symbol and would collide in the emitted header | rename the component or parameter the diagnostic names so the generated symbols are both valid and unique |

## Software bill of materials

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_SBOM_ARTIFACT_INVALID` | `SBOM` | an artifact-typed parameter holds a blank or non-string value, or names an artifact that is absent from the resolved artifact catalog | ensure the resolve result you pass carries every artifact its parameters reference, then regenerate the document |
| `E_SBOM_BINDING_INVALID` | `SBOM` | a parameter in the resolved output declares a lifecycle outside the supported set | give every parameter one of the supported lifecycles: construction, startup, or runtime |
| `E_SBOM_FAILED` | `SBOM` | the assembled bill of materials could not be canonically serialized, so its content hash could not be computed | report this with the resolve result used, since canonical serialization is expected to succeed for every well-formed document |
| `E_SBOM_HASH_INVALID` | `SBOM` | a stored document fails verification: its hash algorithm, canonicalization version, document version, or recorded hash does not match its contents | regenerate the document; a mismatch means the stored bytes changed after the document was produced, so the copy in hand cannot be trusted as evidence |
| `E_SBOM_PATH_INVALID` | `SBOM` | a component in the bill of materials names a dependency that is not itself present in the document | resolve a scope that includes every component reachable through depends_on, so the dependency graph in the document is closed |
| `E_SBOM_PROFILE_INVALID` | `SBOM` | the requested software bill of materials profile is not one this build supports | use the full-audit profile to include resolved values, or the value-redacted profile to omit them |
| `E_SBOM_RESOLVE_INVALID` | `SBOM` | the supplied resolve result is unusable or self-contradictory: a wrong shape, empty resolved output, or a component or parameter carrying conflicting values across scope roots | pass the complete, unmodified result of a successful resolve, and resolve a scope whose roots agree on every shared component and parameter |
| `E_SBOM_STATS_INVALID` | `SBOM` | a document's recorded component, parameter, or artifact counts disagree with its own contents, which signals tampering or corruption rather than bad input | regenerate the document from a fresh resolve; a stored bill of materials must never be hand-edited, because its counts and hash are part of its evidence value |

## Runtime session

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_RUNTIME_ARTIFACT_UNKNOWN` | `RUNTIME` | an artifact-typed parameter holds a blank or non-string value, or names an artifact that the session's resolved artifact catalog does not contain | open the session with a resolve result that carries every artifact its parameters reference, so the catalog is complete |
| `E_RUNTIME_AUDIT_INVALID` | `RUNTIME` | the session's audit log is malformed: a zero or non-ascending sequence, a blank actor, unsorted changed paths, or an uploaded-sequence marker beyond the events actually recorded | re-open the session from a fresh resolve; an audit log that fails these checks has been truncated or edited outside the runtime and is no longer evidence |
| `E_RUNTIME_DIRTY_INVALID` | `RUNTIME` | an override operation is inconsistent with the session's override state: a blank actor, a rollback of a path that is not overridden, a generation that disagrees with the recorded one, or a working-configuration identifier that no longer matches | re-read the current override state before acting on it; a working-configuration mismatch means another writer changed the session first, so refresh and retry |
| `E_RUNTIME_EVENT_INVALID` | `RUNTIME` | the session's event buffer is malformed: a zero capacity or sequence, or buffered events that are not in strict ascending sequence order | re-open the session from a fresh resolve; a persisted snapshot whose event buffer fails these checks has been truncated or edited outside the runtime |
| `E_RUNTIME_HASH_MISMATCH` | `RUNTIME` | the resolve hash recomputed at open time does not match the hash supplied with the request, so the selection fields and the hash no longer agree | pass the resolve result through to open unmodified: dropping or editing the choices, context tags, or defaulted choices invalidates the hash |
| `E_RUNTIME_LIFECYCLE_IMMUTABLE` | `RUNTIME` | the parameter is declared with a construction or startup lifecycle, so it is fixed for the life of the session and cannot be written at runtime | change the value at the lifecycle stage that owns it and re-open the session, or declare the parameter with a runtime lifecycle if it genuinely needs to be mutable |
| `E_RUNTIME_LIMIT_VIOLATION` | `RUNTIME` | the value written to a parameter falls outside the limits that parameter declares, whether a string length bound or a numeric minimum or maximum | send a value inside the declared bounds, or widen the limits in the model source and recompile if the bound itself is wrong |
| `E_RUNTIME_OPEN_INVALID` | `RUNTIME` | the open request or the snapshot it produces is malformed: a hash field that is not a 64-character hexadecimal digest, an unusable scope, undecodable resolved output, a dependency list that is unsorted or names an unknown component, or a parameter path that is ambiguous across scope roots | open with the unmodified output of a successful resolve, and qualify any parameter path the diagnostic reports as ambiguous with its scope root |
| `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` | `RUNTIME` | the snapshot's solver-model reference is empty, will not load, or carries no symbol table, so the session cannot be opened | recompile the model so a complete solver model is emitted beside the package, and keep it reachable from the snapshot's reference |
| `E_RUNTIME_TYPE_MISMATCH` | `RUNTIME` | the value written to a parameter is not compatible with the type that parameter declares | send a value of the declared type; the diagnostic names both the expected type and the kind of value it received |
| `E_RUNTIME_UNKNOWN_PATH` | `RUNTIME` | the parameter path does not have the form component.<id>.param.<key>, or no scope root in the session contains that component and parameter | list the session's parameters to find the exact path, and qualify it with a scope root when the same component appears under more than one |
| `E_RUNTIME_UNKNOWN_SCOPE` | `RUNTIME` | the scope root is blank, or it is not present in the resolved output the session was opened with | use a scope root that appears in the opened snapshot, or re-open the session against a resolve that covers the scope you need |
| `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION` | `RUNTIME` | the request's schema_version, or a persisted snapshot's schema_version, is not the version this build implements | set schema_version to the version this binary reports; a snapshot written by an older build must be re-opened from a fresh resolve rather than replayed |

## Runtime commit

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_RUNTIME_COMMIT_BASE_MISMATCH` | `RUNTIME_COMMIT` | the commit supplied an expected base configuration identifier that no longer matches the session's committed configuration, so another commit landed first | re-read the current configuration identity, reconcile your changes against it, and retry the commit |
| `E_RUNTIME_COMMIT_INVALID` | `RUNTIME_COMMIT` | the commit request is unusable: a blank actor, or a changed-path hint naming a path that is not currently overridden | supply a non-empty actor and list only paths that are actually overridden, or omit the hint and let the commit determine the changed set itself |
| `E_RUNTIME_COMMIT_TARGET_HASH_MISMATCH` | `RUNTIME_COMMIT` | the commit finished but left the committed and working configuration identifiers diverged, which means override state survived a commit that should have cleared it | this is an internal invariant failure rather than a usage error: report it with the session's override state and the commit request |

## Runtime synchronization

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_RUNTIME_SYNC_BACKPRESSURE` | `RUNTIME_SYNC` | a non-blocking publish was refused because the transport's outbound queue is already at its configured depth | let the queue drain and retry, or raise the configured queue depth if the publish rate is legitimately higher than the link can carry |
| `E_RUNTIME_SYNC_BASE_MISMATCH` | `RUNTIME_SYNC` | an incremental update names a base configuration that is not the session's current committed configuration, so the delta was computed against a state this session has moved past | request a delta rebased on the session's current configuration, and reserve a full snapshot for bootstrap or divergence recovery |
| `E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH` | `RUNTIME_SYNC` | an incremental write omits the prior value's hash, or the hash it carries does not match the parameter's current committed value | include an accurate prior-value hash on every incremental write, and rebase the delta when a hash no longer matches |
| `E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN` | `RUNTIME_SYNC` | a warning, not a failure: the update succeeded, and in doing so replaced local overrides on one or more paths, which the result lists | no action is required for the update itself; review the listed paths to decide whether any local intent needs to be re-applied |
| `E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED` | `RUNTIME_SYNC` | the update is marked incremental but omits the base or target configuration identifier that an incremental apply requires | include both identifiers on an incremental update, or send the update as a full snapshot |
| `E_RUNTIME_SYNC_INTERNAL` | `RUNTIME_SYNC` | the sync transport failed for a reason it does not attribute to the link, the payload, or the deadline | report this with the transport configuration in use; retrying an unchanged request is not expected to help |
| `E_RUNTIME_SYNC_INVALID` | `RUNTIME_SYNC` | the update request is unusable: a blank actor, the same path written twice, or a synchronization status field carrying an impossible value | send one write per path with a non-empty actor; duplicate paths are rejected rather than silently reduced to a last-writer-wins result |
| `E_RUNTIME_SYNC_PAYLOAD_INVALID` | `RUNTIME_SYNC` | the transport received a payload it could not interpret, or a configuration or leaf hash field is not a 64-character hexadecimal digest | check that hash-shaped fields carry full digests, and that both ends of the link run compatible versions; unlike a timeout, this is not worth retrying unchanged |
| `E_RUNTIME_SYNC_TARGET_HASH_MISMATCH` | `RUNTIME_SYNC` | the update payload is internally inconsistent: a write's stated resulting hash does not match its own value, or the configuration identifier after applying the writes is not the one the payload claimed | regenerate the update payload from the producing side; a mismatch here means the payload was assembled or edited incorrectly, not that the session drifted |
| `E_RUNTIME_SYNC_TIMEOUT` | `RUNTIME_SYNC` | the sync transport did not receive a response within its deadline | retry the operation, since this condition is transient; a persistent timeout points at network latency or an unresponsive backend rather than the request |
| `E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED` | `RUNTIME_SYNC` | the sync transport link is down, or an update was requested while the backend is marked disconnected | restore connectivity and retry; a session may keep serving its committed configuration while disconnected, so this does not by itself invalidate local state |

## Runtime security

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_RUNTIME_SECURITY_AUTHN_FAILED` | `RUNTIME_SECURITY` | the sync transport's credentials were rejected when connecting to the backend | check that the credentials are correct, current, and authorized for this device, then reconnect; retrying with the same rejected credentials will not succeed |

## Runtime telemetry

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_RUNTIME_TELEMETRY_BACKPRESSURE` | `RUNTIME_TELEMETRY` | reserved for a telemetry sink refusing a record because its buffer is full; no current code path emits it, because the sinks drop the oldest record and count the drop instead | no action is needed for this code; to detect record loss, read the dropped-record counter a sink exposes rather than watching for this diagnostic |
| `E_RUNTIME_TELEMETRY_IO` | `RUNTIME_TELEMETRY` | a telemetry spool or archive file could not be created, read, written, renamed, or removed on the local device | check the free space, permissions, and mount state of the telemetry directory; the diagnostic names the specific file and the underlying system error |
| `E_RUNTIME_TELEMETRY_PUBLISH_FAILED` | `RUNTIME_TELEMETRY` | a telemetry flush reached its publisher, and the publisher itself rejected or could not deliver the batch | treat this as a delivery failure rather than a data failure: the records stay spooled, so restoring the publisher's availability lets the next flush drain them |
| `E_RUNTIME_TELEMETRY_RECORD_CORRUPT` | `RUNTIME_TELEMETRY` | a telemetry record could not be formed or read back: an audit event with a blank actor, or a spooled record that will not serialize or parse | supply a non-empty actor on every audit event; a record that fails to parse on read-back indicates a damaged spool file, which can be removed to resume telemetry |

## Authority

| Code | Family | Cause | Remedy |
| --- | --- | --- | --- |
| `E_AUTHORITY_ROLE_DENIED` | `AUTHORITY` | the acting role is not authorized for the requested action, or the role itself was not recognized and the check failed closed | perform the action under a role that holds the required authority; the denial deliberately carries no detail about the role, unit, or value involved |
