// 00-service-multi-env — components chunk.
//
// Authored source for the CUE front-end; component params inherit from the
// sibling 00_definitions chunk during whole-pack evaluation, and `cue export`
// emits the inheritance-resolved 10_components.json that the compiler ingests.
// `condition`/`overrides` are opaque late-bound pass-through data.
//
// The single `webapp` service is the whole model. Its per-environment values
// come from overrides gated on `environment == 'prod'`, so prod hardens the
// shared defaults. Those `condition`s are inclusion selectors and nothing else
// (ADR-0054 §3): they decide which override applies, never what a user may
// pick. The policy that makes prod + log_level=debug unsatisfiable — the case
// `cfx explain` narrates — is the `prod_forbids_debug` constraint, declared in
// the sibling 00_definitions chunk.
package configflux

chunk: #Config & {
	package: "service_multi_env"
	version: "1.0.0"

	components: {
		webapp: {
			type: "service"
			params: {
				request_timeout_ms: {
					inherits: "request_timeout_ms"
					value:    30000
					overrides: [
						{condition: "environment == 'prod'", value: 5000},
					]
				}
				deploy_tier: {
					inherits: "deploy_tier"
					value:    "nonprod"
					overrides: [
						{condition: "environment == 'prod'", value: "production"},
					]
				}
				health_check_path: {
					inherits: "health_check_path"
					value:    "/healthz"
				}
			}
		}
	}
}
