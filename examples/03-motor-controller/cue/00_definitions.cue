// 03-motor-controller — definitions chunk (ADR-0027 E-5, configflux-bmgy).
// CUE authoring front-end; exported to 00_definitions.json. Valueless roots.
package configflux

chunk: #Config & {
	package: "motor_controller"
	version: "1.0.0"

	definitions: {
		control_mode: {
			type:      "string"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Motor control algorithm: foc (field-oriented) or trapezoidal"
		}
		motor_driver_slot: {
			type:      "artifact"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Motor driver binary artifact selected at compile time"
		}
		encoder_driver_slot: {
			type:      "artifact"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Encoder driver binary artifact selected at compile time"
		}
		pwm_frequency_khz: {
			type:      "integer"
			unit:      "kHz"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "PWM switching frequency for the motor driver"
		}
		current_limit_a: {
			type:      "float"
			unit:      "A"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Maximum phase current limit"
		}
		pid_gain_trim: {
			type:      "float"
			unit:      "ratio"
			lifecycle: "runtime"
			access:    "technician"
			doc:       "Runtime PID gain trim for field tuning"
		}
	}
}
