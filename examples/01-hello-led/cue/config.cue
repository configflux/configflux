// 01-hello-led — a bare-minimum CUE-authored configuration model.
//
// CUE is the authoring front-end: a component parameter's `inherits`
// pointer is gap-filled against the sibling definition during
// whole-pack evaluation. This .cue is the human-authored source;
// `cue export` produces the inheritance-resolved config.json that the
// compiler ingests.
//
// Export:  CUE=/path/to/cue cue export -e chunk examples/01-hello-led/cue/config.cue > examples/01-hello-led/config.json
// (then re-resolve via compiler/cue/schema.cue #ResolvePack; see compiler/cue/README.md).
package configflux

chunk: #Config & {
	package: "hello_led"
	version: "1.0.0"

	// A single parameter definition: the LED blink interval.
	definitions: {
		blink_interval_ms: {
			type:      "integer"
			unit:      "ms"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "LED blink interval in milliseconds"
		}
	}

	// One component with one parameter — the bare minimum. `blink_interval`
	// inherits its type/unit/lifecycle/access/doc from `blink_interval_ms`.
	components: {
		led_driver: {
			type: "module"
			params: {
				blink_interval: {
					inherits: "blink_interval_ms"
					value:    500
				}
			}
		}
	}
}
