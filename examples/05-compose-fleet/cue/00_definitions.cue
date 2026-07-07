// 05-compose-fleet — definitions chunk (ADR-0027 E-5 authoring front-end).
// CUE authoring source; exported out-of-band to 00_definitions.json via the
// hermetic, Bazel-pinned cue evaluator (ADR-0021 Decision 5). run.sh consumes
// the committed JSON — it never invokes cue. Definitions are valueless roots;
// each using component param authors its own `value`.
package configflux

chunk: #Config & {
	package: "compose_fleet"
	version: "1.0.0"

	definitions: {
		// Shared platform identity, stamped per instance at construction.
		instance_id: {
			type:      "string"
			lifecycle: "construction"
			access:    "developer"
			doc:       "Stable identifier for one running instance in a stack"
		}
		// Where a service publishes telemetry / reads work. Differs per named
		// environment (a real broker on a node, a localhost stub for dev).
		broker_endpoint: {
			type:      "string"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Message-broker endpoint the service connects to at startup"
		}
		// Service cadence (frame rate / publish interval). Tuned down locally so
		// a developer machine is not saturated.
		tick_interval_ms: {
			type:      "integer"
			unit:      "ms"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Service work-loop interval in milliseconds"
		}
		// Verbosity. A field deployment runs quiet; a local debug run is chatty.
		log_level: {
			type:      "string"
			lifecycle: "runtime"
			access:    "technician"
			doc:       "Runtime log verbosity (info, debug, trace)"
		}
	}
}
