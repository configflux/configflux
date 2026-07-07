// 02-sensor-gateway — components chunk (ADR-0027 E-5, configflux-bmgy).
// Authored source for the CUE front-end. A component param's `inherits` pointer
// is gap-filled against the sibling 00_definitions chunk during whole-pack
// evaluation; `cue export` then emits the inheritance-resolved 10_components.json
// that the compiler ingests. `condition`/`overrides` are opaque pass-through.
package configflux

chunk: #Config & {
	package: "sensor_gateway"
	version: "1.0.0"

	components: {
		sensor_bus: {
			type: "module"
			params: {
				protocol: {
					inherits: "protocol"
					value:    "modbus_rtu"
					overrides: [
						{
							condition: "bus_type == 'ethernet'"
							value:     "modbus_tcp"
						},
					]
				}
			}
		}
		data_logger: {
			type: "module"
			depends_on: ["sensor_bus"]
			params: {
				poll_interval: {
					inherits: "poll_interval_ms"
					value:    1000
					overrides: [
						{
							condition: "environment == 'high_speed'"
							value:     100
						},
					]
				}
				buffer_depth: {
					inherits: "buffer_depth"
					value:    64
					overrides: [
						{
							condition: "environment == 'high_speed'"
							value:     256
						},
					]
				}
			}
		}
		// Conditional component: only included for ethernet deployments.
		network_monitor: {
			type: "module"
			depends_on: ["sensor_bus"]
			condition: "bus_type == 'ethernet'"
		}
	}
}
