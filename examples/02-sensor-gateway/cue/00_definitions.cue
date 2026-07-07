// 02-sensor-gateway — definitions chunk (ADR-0027 E-5, configflux-bmgy).
// CUE authoring front-end; exported to 00_definitions.json. Definitions are the
// valueless inheritance roots (#Definition forbids `value`).
package configflux

chunk: #Config & {
	package: "sensor_gateway"
	version: "1.0.0"

	definitions: {
		poll_interval_ms: {
			type:      "integer"
			unit:      "ms"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Sensor poll interval in milliseconds"
		}
		protocol: {
			type:      "string"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Field-bus protocol used by the gateway"
		}
		buffer_depth: {
			type:      "integer"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Number of readings held in the ring buffer"
		}
	}
}
