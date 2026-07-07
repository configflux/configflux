// S2 wind turbine / smoke — components + artifacts chunk, authored in CUE
// (ADR 0021, phase 6). Semantically equivalent to ../chunks/10_components.toml.
//
// `condition` strings and `overrides` blocks are opaque pass-through data here
// (ADR 0021 decision 1): CUE types and emits them verbatim; the Rust resolver +
// BDD solver own their late-binding semantics. The 0.08 pitch-trim-gain float is
// the spike's existential float byte-equality target (configflux-7iuw).
//
// See 00_definitions.cue for why there is no `import`.
package configflux

chunk: #Config & {
	package: "s2_wind_turbine"
	version: "1.0.0"
	artifacts: {
		async_control_driver: {
			name:    "async_control_driver"
			version: "3.2.0"
			hash:    "sha256-async-control-driver"
			source:  "artifact://wind/async_control_driver.so"
			target:  "/opt/configflux/wind/async_control_driver.so"
			doc:     "Asynchronous control driver for geared turbines"
		}
		direct_drive_control_driver: {
			name:    "direct_drive_control_driver"
			version: "3.4.1"
			hash:    "sha256-direct-drive-control-driver"
			source:  "artifact://wind/direct_drive_control_driver.so"
			target:  "/opt/configflux/wind/direct_drive_control_driver.so"
			doc:     "Control driver for direct-drive turbines"
		}
	}
	components: {
		grid_interface: {type: "module"}
		sensor_stack: {type:   "module"}
		turbine_controller: {
			type:       "controller"
			depends_on: ["grid_interface", "sensor_stack"]
			params: {
				grid_profile: {
					inherits: "grid_profile"
					value:    "ieee_1547"
					overrides: [
						{condition: "grid_code == 'iec_61400'", value: "iec_61400"},
					]
				}
				control_driver: {
					inherits: "control_driver_slot"
					value:    "async_control_driver"
					overrides: [
						{condition: "gearbox_type == 'direct_drive'", value: "direct_drive_control_driver"},
					]
				}
				pitch_trim_gain: {
					inherits: "pitch_trim_gain"
					value:    0.08
				}
			}
		}
	}
}
