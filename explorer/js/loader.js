// SPDX-License-Identifier: BUSL-1.1
//
// Artifact loader (ADR-0043 §1: read-only, no backend, no network). Files are
// read with the FileReader API from a file picker or drag-and-drop — never
// fetched over the network — so the explorer works from file:// and makes zero
// requests. Every failure path returns an honest, specific result the app turns
// into a visible error state (never a blank page): unreadable file, invalid
// JSON, unrecognized shape, or unsupported schema version.

import { detectKind, guardSchemaVersion, kindLabel, KIND } from "./schema.js";

/**
 * Read + classify + version-check one File. Resolves to a result object:
 *   { ok: true,  kind, data, name }
 *   { ok: false, error, name }        // human-readable, specific
 * Never rejects — the caller always gets a result to render.
 */
export function loadFile(file) {
  return new Promise((resolve) => {
    const name = file && file.name ? file.name : "(unnamed)";
    if (!file) {
      resolve({ ok: false, error: "No file was provided.", name });
      return;
    }
    const reader = new FileReader();
    reader.onerror = () =>
      resolve({ ok: false, error: `Could not read “${name}”. Is it a readable local file?`, name });
    reader.onload = () => resolve(classify(String(reader.result), name));
    try {
      reader.readAsText(file);
    } catch (err) {
      resolve({ ok: false, error: `Could not read “${name}”: ${err.message}`, name });
    }
  });
}

/** Parse text → classify shape → guard schema version. Pure; unit-testable. */
export function classify(text, name = "(text)") {
  let data;
  try {
    data = JSON.parse(text);
  } catch (err) {
    return { ok: false, error: `“${name}” is not valid JSON: ${err.message}`, name };
  }

  const kind = detectKind(data);
  if (kind === KIND.UNKNOWN) {
    return {
      ok: false,
      error:
        `“${name}” is JSON but not a recognized ConfigFlux artifact. Expected an ` +
        `inspect summary, cfx options list, resolved snapshot, or explain output.`,
      name,
    };
  }

  const guard = guardSchemaVersion(data);
  if (!guard.ok) {
    const supported = guard.supported.join(", ");
    const found = guard.found === null ? "none declared" : String(guard.found);
    return {
      ok: false,
      error:
        `“${name}” is a ${kindLabel(kind)}, but its schema_version (${found}) is not ` +
        `supported by this explorer (supports: ${supported}). Update the explorer or ` +
        `regenerate the artifact.`,
      name,
      kind,
    };
  }

  return { ok: true, kind, data, name };
}

/**
 * Wire a file <input> and a drop zone to `onResult`. Drag-over is styled via the
 * `.dragging` class. Only the first file is used (single-artifact views).
 */
export function wireFileControls({ inputEl, dropEl, onResult }) {
  if (inputEl) {
    inputEl.addEventListener("change", () => {
      const file = inputEl.files && inputEl.files[0];
      if (file) loadFile(file).then(onResult);
      inputEl.value = ""; // allow re-selecting the same file
    });
  }
  if (dropEl) {
    const stop = (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
    };
    ["dragenter", "dragover"].forEach((t) =>
      dropEl.addEventListener(t, (ev) => {
        stop(ev);
        dropEl.classList.add("dragging");
      })
    );
    ["dragleave", "drop"].forEach((t) =>
      dropEl.addEventListener(t, (ev) => {
        stop(ev);
        dropEl.classList.remove("dragging");
      })
    );
    dropEl.addEventListener("drop", (ev) => {
      const file = ev.dataTransfer && ev.dataTransfer.files && ev.dataTransfer.files[0];
      if (file) loadFile(file).then(onResult);
    });
  }
}
