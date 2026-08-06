# ConfigFlux Model Explorer

A static, local, read-only web UI for browsing the canonical JSON artifacts the
ConfigFlux pipeline already emits — the compiled model, resolved snapshots, and
explain output. It is pure presentation over files you already have.

- **No build step.** No npm, no bundler, no lockfiles. The files in this
  directory are the files that run.
- **No network.** No CDN, no web fonts, no telemetry. It works fully offline.
- **No backend.** Artifacts are loaded with a file picker or drag-and-drop; the
  explorer never runs ConfigFlux and never writes anything.

## Opening it

Open `explorer/index.html` directly in **Firefox** or **Safari** (they load
local ES modules over `file://`). **Chrome** restricts `file://` module loading,
so serve the folder with any static file server — for example, from this
directory:

```bash
python3 -m http.server 8000
# then browse to http://localhost:8000/
```

A static file server is not a backend: it only hands over the files unchanged.

## The three views

1. **Model** — load a model summary (`compiler inspect … summary`) or a
   facet-options list (`cfx options --format json`) to browse the
   component / facet / option structure as a collapsible, searchable tree,
   above a hash header (schema version, counts, and the hash the artifact
   carries — `source_digest` for a model summary, `model_hash` for a
   facet-options list).
2. **Resolution** — load a resolved snapshot (`cfx resolve --format json`) to
   see the `model_hash → selection_state_hash → resolve_hash` lineage and a
   searchable table of every resolved parameter.
3. **Explain** — load an explain output (`cfx explain --format json`) to see the
   minimal conflicting-constraint set behind a rejected selection, rendered with
   the same wording the CLI prints.

Accessibility (keyboard navigation, ARIA roles, visible focus), a light/dark
theme toggle, and honest error states (bad file, unrecognized shape, unsupported
schema version — never a blank page) are built in.

## Sample artifacts

Ready-to-load samples for the water-pump scenario live in `fixtures/`. Load any
of them with the file picker. Their provenance and the exact command to
regenerate them from source are documented in [`fixtures/README.md`](fixtures/README.md).

## Schema compatibility

The explorer renders one product schema version at a time; the supported version
is the single `SUPPORTED_VERSIONS` constant in [`js/schema.js`](js/schema.js).
Loading an artifact from a different schema version produces a visible error that
names both the found and the supported version. A hermetic test keeps the sample
fixtures and that constant aligned with the pipeline's current schema:

```bash
bazel test //explorer:fixture_schema_test
```

A separate browser-based smoke check (content, search, dark mode, zero console
errors, zero external network requests, basic accessibility) is run on demand by
maintainers; it drives a real browser and is intentionally not part of the
standard `bazel build //...`.

## Layout

```
explorer/
  index.html            app shell (tabs, file picker, drop zone)
  css/explorer.css      light/dark theme, layout, focus styles
  js/
    app.js              bootstrap, tab routing, theme toggle
    loader.js           file read + parse + dispatch (no network)
    schema.js           supported versions + kind detection + guard
    dom.js              tiny, escaping DOM helpers
    model_view.js       tree + search + hash header
    resolution_view.js  hash lineage + params table
    explain_view.js     unsat-core render (CLI-parity wording)
  fixtures/             committed sample artifacts + regeneration doc
```
