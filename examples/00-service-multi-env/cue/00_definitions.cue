// 00-service-multi-env — definitions, facets, and constraints chunk.
//
// CUE authoring front-end; exported to 00_definitions.json (the inheritance
// roots are valueless — #Definition forbids `value`). `facets` declares the
// four selection dimensions this service exposes; a facet's declared values
// become the resolvable option domain, including a default arm that no
// condition names.
//
// `constraints` declares this model's policy (ADR-0054 §1): a named assertion
// over facet values that must hold in every resolved configuration. Policy is
// its own construct — it is NOT a component. A `condition` on a component or an
// override is an inclusion selector and nothing else (ADR-0054 §3), so the rule
// "no debug logging in prod" belongs here, not in the components chunk.
package configflux

chunk: #Config & {
	package: "service_multi_env"
	version: "1.0.0"

	definitions: {
		request_timeout_ms: {
			type:      "integer"
			unit:      "ms"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Inbound HTTP request timeout in milliseconds"
		}
		deploy_tier: {
			type:      "string"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "Deployment tier label surfaced to the running service"
		}
		health_check_path: {
			type:      "string"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "HTTP path the platform probes for liveness"
		}
	}
	facets: {
		environment: {values: ["dev", "staging", "prod"], default: "dev", doc: "Deployment target. prod tightens the request timeout and flips the tier label."}
		log_level: {values: ["info", "debug"], default: "info", doc: "Log verbosity. debug is disallowed in prod (see the prod_forbids_debug constraint)."}
		replica_class: {values: ["single", "scaled"], default: "single", doc: "Sizing selection recorded for the environment; consumed downstream to pick a replica count."}
		beta_dashboard: {values: ["off", "on"], default: "off", doc: "Feature toggle for the beta dashboard UI, recorded per environment."}
	}
	constraints: {
		prod_forbids_debug: {
			condition: "environment != 'prod' || log_level != 'debug'"
			doc:       "Debug logging is not permitted in production."
		}
	}
}
