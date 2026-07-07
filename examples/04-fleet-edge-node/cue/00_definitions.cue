// 04-fleet-edge-node — definitions chunk (ADR-0027 E-5, configflux-bmgy).
// CUE authoring front-end; exported to 00_definitions.json. Valueless roots.
package configflux

chunk: #Config & {
	package: "fleet_edge_node"
	version: "1.0.0"

	definitions: {
		device_id: {
			type:      "string"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Unique device identifier assigned at manufacture"
		}
		firmware_slot: {
			type:      "artifact"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Firmware binary selected per device_class"
		}
		update_endpoint: {
			type:      "string"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Fleet-manager update endpoint URL"
		}
		poll_interval_s: {
			type:      "integer"
			unit:      "s"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Update-check poll interval in seconds"
		}
		watchdog_timeout_ms: {
			type:      "integer"
			unit:      "ms"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Watchdog bite timeout in milliseconds"
		}
		log_level: {
			type:      "string"
			lifecycle: "runtime"
			access:    "technician"
			doc:       "Runtime log verbosity (info, debug, trace)"
		}
	}
}
