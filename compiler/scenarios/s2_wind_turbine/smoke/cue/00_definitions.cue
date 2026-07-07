// S2 wind turbine / smoke — definitions chunk, authored in CUE (ADR 0021, phase 6).
//
// Semantically equivalent to ../chunks/00_definitions.toml. Exported to
// 00_definitions.json via the pinned cue binary (compiler/cue/export_fixtures.sh);
// the differential gate (scenario_cue_equivalence_tests.rs) proves the
// CUE-derived CMP is byte-identical to the TOML-derived CMP.
//
// No `import` statement: there is no cue.mod yet, so this file is evaluated
// together with compiler/cue/schema.cue (same `package configflux`) by passing
// both paths to `cue`, exactly as the phase-3 validator does. `-e chunk` emits
// only this chunk's concrete JSON (#-definitions and _hidden fields are not
// exported).
package configflux

chunk: #Config & {
	package: "s2_wind_turbine"
	version: "1.0.0"
	definitions: {
		grid_profile: {
			type:      "string"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Grid code commissioning profile"
		}
		control_driver_slot: {
			type:      "artifact"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Selected control driver artifact"
		}
		pitch_trim_gain: {
			type:      "float"
			unit:      "ratio"
			lifecycle: "runtime"
			access:    "technician"
			doc:       "Runtime pitch trim gain"
		}
	}
}
