// 05-compose-fleet — components chunk (ADR-0027 E-5 authoring front-end).
// Component params inherit shape from the sibling 00_definitions chunk during
// whole-pack `cue export`; the committed 10_components.json is the
// inheritance-resolved emission the compiler ingests. `condition`/`overrides`
// are opaque pass-through late-bound data (ADR-0003) — CUE does not interpret
// them; the solver/compiler do at resolve time.
//
// The model is deliberately minimal but has TWO independently-scopable service
// components — vision_service and telemetry_service — plus a shared platform
// component. Two facets drive the per-environment differences:
//
//   deploy_env  : "robot" | "local"     (which named environment is resolving)
//   log_profile : "quiet" | "verbose"   (field-quiet vs developer-chatty)
//
// MODELLING NOTE — every value a facet can take is given its OWN override
// condition, so BOTH options of each facet are first-class, selectable values.
// The solver enumerates a facet's valid options from the conditions that
// reference it; a value that never appears in a condition is a silent default,
// not a selectable option, and a resolve that leaves such a facet undecided
// fails closed (E_RESOLVE_CONTEXT_UNSATISFIED). Authoring both arms explicitly
// is what lets a named environment (environments.json) pin deploy_env=robot OR
// deploy_env=local and resolve either way against the SAME model. The inert
// `value:` is a never-selected fallback kept only to satisfy the authored shape.
package configflux

chunk: #Config & {
	package: "compose_fleet"
	version: "1.0.0"

	components: {
		// Shared platform layer both services depend on.
		platform: {
			type: "module"
			params: {
				instance_id: {
					inherits: "instance_id"
					value:    "instance-0001"
				}
				log_level: {
					inherits: "log_level"
					value:    "info"
					overrides: [
						{
							condition: "log_profile == 'quiet'"
							value:     "warn"
						},
						{
							condition: "log_profile == 'verbose'"
							value:     "debug"
						},
					]
				}
			}
		}

		// Camera/perception service. Ticks fast on a field node, slow locally;
		// talks to the field broker on a node, to a localhost stub in dev.
		vision_service: {
			type: "controller"
			depends_on: ["platform"]
			params: {
				broker_endpoint: {
					inherits: "broker_endpoint"
					value:    "tcp://unset.invalid:1883"
					overrides: [
						{
							condition: "deploy_env == 'robot'"
							value:     "tcp://broker.fleet.local:1883"
						},
						{
							condition: "deploy_env == 'local'"
							value:     "tcp://127.0.0.1:1883"
						},
					]
				}
				tick_interval: {
					inherits: "tick_interval_ms"
					value:    0
					overrides: [
						{
							condition: "deploy_env == 'robot'"
							value:     33
						},
						{
							condition: "deploy_env == 'local'"
							value:     200
						},
					]
				}
				log_level: {
					inherits: "log_level"
					value:    "info"
					overrides: [
						{
							condition: "log_profile == 'quiet'"
							value:     "info"
						},
						{
							condition: "log_profile == 'verbose'"
							value:     "trace"
						},
					]
				}
			}
		}

		// Telemetry/publish service. Publishes faster on a field node than
		// locally; same broker split as vision_service.
		telemetry_service: {
			type: "controller"
			depends_on: ["platform"]
			params: {
				broker_endpoint: {
					inherits: "broker_endpoint"
					value:    "tcp://unset.invalid:1883"
					overrides: [
						{
							condition: "deploy_env == 'robot'"
							value:     "tcp://broker.fleet.local:1883"
						},
						{
							condition: "deploy_env == 'local'"
							value:     "tcp://127.0.0.1:1883"
						},
					]
				}
				tick_interval: {
					inherits: "tick_interval_ms"
					value:    0
					overrides: [
						{
							condition: "deploy_env == 'robot'"
							value:     1000
						},
						{
							condition: "deploy_env == 'local'"
							value:     5000
						},
					]
				}
				log_level: {
					inherits: "log_level"
					value:    "info"
					overrides: [
						{
							condition: "log_profile == 'quiet'"
							value:     "info"
						},
						{
							condition: "log_profile == 'verbose'"
							value:     "debug"
						},
					]
				}
			}
		}
	}
}
