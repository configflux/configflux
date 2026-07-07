# Promotion lineage-link convention

When a value that was first set as a temporary, unit-scoped override is
promoted to become the new base value for everyone, the promotion is performed
as an ordinary source change: a developer edits the source configuration and
opens a normal pull request. The compiler, solver, and gate provide the
per-unit validation as usual.

That manual step has one gap. The override the promotion came from, and the
owner who approved making it permanent, are not visible in the resulting source
diff. Without an explicit record, the provenance chain from the originating
override to the promoted base value is lost at the moment of the git commit.

This convention closes that gap: **a promotion commit records the lineage link
— the originating override id and the approving owner — directly in the commit
message, so the override-to-base provenance chain survives the manual git
step.** It is the same record described in the provenance and override-intent
design decisions, expressed as a concrete commit
convention and backed by an automated check.

## Roles

This convention uses two generic roles. They map onto whatever titles a given
deployment uses.

- **Operator** — acts on a single unit. An operator creates an override to
  adjust a running unit. An override is temporary and unit-scoped; on its own it
  never changes the base value for anything else.
- **Owner** (product owner) — blesses a change for everyone. Promoting an
  override to the base value is an owner decision. The owner approves; a
  developer mechanically executes the promotion by editing the source.

An override may be created and committed for its own unit without waiting for an
owner. Promotion to base — making the value apply everywhere — is the step this
convention governs.

## What a promotion is

A unit's resolved configuration has a **base value** for each parameter, plus an
optional **override layer** on top of it. Promotion takes a value that proved
itself in the override layer and writes it into the base, so it becomes the
default everywhere rather than a per-unit deviation.

Concretely, promotion changes a **base value** in the source — the authored
value a parameter resolves to before any override is applied. That source edit
is the manual git step. Editing the override layer, or any other file, is not a
promotion and is not governed by this convention.

## The lineage trailers

A promotion commit MUST carry two
[git trailers](https://git-scm.com/docs/git-interpret-trailers) in its commit
message — `Key: value` lines, conventionally in the final paragraph:

```
feat(scope): promote the commissioning flow ceiling to base

Promoted-Override-Id: <originating override id>
Promotion-Approved-By: <owner who approved the promotion>
```

| Trailer | Meaning |
|---|---|
| `Promoted-Override-Id` | The id of the originating override the promoted base value descends from. This is the override's own identity — the same identity carried by the override record in the provenance lineage. |
| `Promotion-Approved-By` | The owner who blessed the promotion. This is the record of who decided the value should apply to everyone, captured at the moment the base value changes. |

Both trailers are required, and both must carry a non-empty value. A commit that
changes a base value but is missing either trailer — or leaves one empty — is
rejected by the check described below.

### Optional lineage reference

A promotion commit MAY also carry:

| Trailer | Meaning |
|---|---|
| `Lineage-Ref` | The id of the provenance lineage entry the promoted base value links back to. This pins the promotion to a specific point in the unit's recorded lineage. It is optional: the originating override id is sufficient to reconstruct the chain, and the lineage reference is a convenience for tooling that wants a direct pointer. |

The lineage data model these ids refer to — the versioned, parent-linked
provenance entries and the override identity — lives in the runtime contract
surface (`compiler/src/runtime_api/contracts.rs`). This convention only fixes
*where* the link is recorded at promotion time; it does not change that model.

## Enforcement

The convention is enforced automatically. A check runs whenever a commit changes
a base value in the source configuration tree and fails the commit if the
lineage trailers are missing or malformed.

**What counts as a base-value change.** The check looks at the commit's own diff.
A commit changes a base value when it adds or removes a `value:` assignment in a
source configuration file under the base tree. This rule is deliberately simple
and robust:

- A change that does not touch the base tree at all — documentation, code,
  tooling, tests — is not a promotion and is unaffected.
- A change inside the base tree that does not alter a `value:` line — for
  example a comment or formatting edit — changes no base value and is
  unaffected.
- Generated artifacts derived from the source (the exported `.json` companions)
  are not the authored base source and are not treated as base-value changes.

**Outcome.**

- Base-value change **with** both required trailers, well-formed → the check
  passes.
- Base-value change **missing** either trailer, or with an empty trailer value →
  the check fails with a message naming the missing trailers and the offending
  file, and the commit does not pass the gate.
- A commit that changes no base value → the check passes; the trailers are not
  required.

The check is wired into the standard local gate as one of the leak/provenance
checks, scoped so it only runs when a base-value source file is part of the
change. It is also available as a standalone command for inspecting a specific
commit.

## Why this matters

A promoted value is a value someone decided everyone should depend on. Recording
the originating override and the approving owner at the moment of promotion
keeps that decision auditable: anyone reading the history can see not just that
the base value changed, but which field-proven override it came from and who
authorized making it permanent. The provenance chain that the override layer
maintains continues unbroken across the one step — the manual git edit — where
it would otherwise be lost.
