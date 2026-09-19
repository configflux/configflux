// 03-motor-controller — components + artifacts chunk (ADR-0027 E-5,
// configflux-bmgy). Authored source for the CUE front-end; component params
// inherit type/unit/etc. from the sibling 00_definitions chunk during
// whole-pack evaluation, and `cue export` emits the inheritance-resolved
// 10_components.json the compiler ingests. `overrides` are opaque pass-through.
package configflux

chunk: #Config & {
	package: "motor_controller"
	version: "1.0.0"

	artifacts: {
		foc_driver: {
			name:    "foc_driver"
			version: "3.1.0"
			hash:    "sha256-foc-driver"
			source:  "artifact://motor/foc_driver.so"
			target:  "/opt/configflux/motor/foc_driver.so"
			doc:     "Field-oriented control motor driver"
		}
		trapz_driver: {
			name:    "trapz_driver"
			version: "2.4.0"
			hash:    "sha256-trapz-driver"
			source:  "artifact://motor/trapz_driver.so"
			target:  "/opt/configflux/motor/trapz_driver.so"
			doc:     "Trapezoidal commutation motor driver"
		}
		absolute_encoder_driver: {
			name:    "absolute_encoder_driver"
			version: "1.5.2"
			hash:    "sha256-absolute-encoder"
			source:  "artifact://encoder/absolute.so"
			target:  "/opt/configflux/encoder/absolute.so"
			doc:     "Absolute position encoder driver"
		}
		incremental_encoder_driver: {
			name:    "incremental_encoder_driver"
			version: "1.3.0"
			hash:    "sha256-incremental-encoder"
			source:  "artifact://encoder/incremental.so"
			target:  "/opt/configflux/encoder/incremental.so"
			doc:     "Incremental encoder driver"
		}
	}

	components: {
		motor_drive: {
			type: "module"
			params: {
				control_mode: {
					inherits: "control_mode"
					value:    "foc"
					overrides: [
						{
							condition: "motor_class == 'brushed_dc'"
							value:     "trapezoidal"
						},
					]
				}
				motor_driver: {
					inherits: "motor_driver_slot"
					value:    "foc_driver"
					overrides: [
						{
							condition: "motor_class == 'brushed_dc'"
							value:     "trapz_driver"
						},
					]
				}
				pwm_frequency: {
					inherits: "pwm_frequency_khz"
					value:    20
					overrides: [
						{
							condition: "motor_class == 'brushed_dc'"
							value:     10
						},
					]
				}
				current_limit: {
					inherits: "current_limit_a"
					value:    15.0
					overrides: [
						{
							condition: "power_rating == 'high'"
							value:     30.0
						},
					]
				}
			}
		}
		encoder_interface: {
			type: "module"
			params: {
				encoder_driver: {
					inherits: "encoder_driver_slot"
					value:    "incremental_encoder_driver"
					overrides: [
						{
							condition: "encoder_type == 'absolute'"
							value:     "absolute_encoder_driver"
						},
					]
				}
				// The runtime HANDLE of the encoder_type facet: the `facet` line
				// is what makes this parameter that facet, so a runtime write to
				// it is a facet selection the model's constraints govern. It
				// declares no `value` — the facet's resolved value is the
				// parameter's value.
				encoder_mode: {
					type:      "string"
					facet:     "encoder_type"
					lifecycle: "runtime"
					access:    "technician"
					doc:       "Encoder feedback mode; the runtime handle of the encoder_type facet"
				}
			}
		}
		motion_controller: {
			type: "controller"
			depends_on: ["motor_drive", "encoder_interface"]
			params: {
				pid_gain_trim: {
					inherits: "pid_gain_trim"
					value:    0.1
				}
			}
		}
		// Conditional: safety monitor only for high-power configurations.
		safety_monitor: {
			type: "module"
			depends_on: ["motor_drive"]
			condition: "power_rating == 'high'"
		}
	}
}
