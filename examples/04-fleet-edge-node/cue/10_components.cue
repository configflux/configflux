// 04-fleet-edge-node — components + artifacts chunk (ADR-0027 E-5,
// configflux-bmgy). Authored source for the CUE front-end; component params
// inherit from the sibling 00_definitions chunk during whole-pack evaluation,
// and `cue export` emits the inheritance-resolved 10_components.json that the
// compiler ingests. `condition`/`overrides` are opaque pass-through data.
package configflux

chunk: #Config & {
	package: "fleet_edge_node"
	version: "1.0.0"

	artifacts: {
		edge_firmware: {
			name:    "edge_firmware"
			version: "2.4.0"
			hash:    "sha256-edge-firmware"
			source:  "artifact://firmware/edge.bin"
			target:  "/opt/configflux/firmware/edge.bin"
			doc:     "Edge-device firmware image"
		}
		gateway_firmware: {
			name:    "gateway_firmware"
			version: "2.4.0"
			hash:    "sha256-gateway-firmware"
			source:  "artifact://firmware/gateway.bin"
			target:  "/opt/configflux/firmware/gateway.bin"
			doc:     "Gateway-device firmware image"
		}
	}

	components: {
		network_stack: {
			type: "module"
			params: {
				device_id: {
					inherits: "device_id"
					value:    "edge-0001"
				}
			}
		}
		update_agent: {
			type: "module"
			depends_on: ["network_stack"]
			params: {
				firmware: {
					inherits: "firmware_slot"
					value:    "edge_firmware"
					overrides: [
						{
							condition: "device_class == 'gateway'"
							value:     "gateway_firmware"
						},
					]
				}
				endpoint: {
					inherits: "update_endpoint"
					value:    "https://fleet.example.com/updates/stable"
					overrides: [
						{
							condition: "update_channel == 'canary'"
							value:     "https://fleet.example.com/updates/canary"
						},
					]
				}
				poll_interval: {
					inherits: "poll_interval_s"
					value:    300
					overrides: [
						{
							condition: "update_channel == 'canary'"
							value:     60
						},
					]
				}
			}
		}
		watchdog: {
			type: "module"
			params: {
				watchdog_timeout: {
					inherits: "watchdog_timeout_ms"
					value:    5000
					overrides: [
						{
							condition: "device_class == 'gateway'"
							value:     10000
						},
					]
				}
			}
		}
		runtime_tuner: {
			type: "controller"
			depends_on: ["update_agent", "watchdog"]
			params: {
				log_level: {
					inherits: "log_level"
					value:    "info"
				}
			}
		}
		// Region-based conditional component: GDPR data-residency guard only in
		// EU. Present in the compiled model only when region == 'eu'.
		regional_compliance: {
			type: "module"
			depends_on: ["update_agent"]
			condition: "region == 'eu'"
		}
	}
}
