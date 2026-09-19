// 06-catalogue-polyrepo — the CATALOGUE repository's chunk.
//
// This repository owns two things: the table of physical containers the
// deployment sites use, and the rule about which site uses which. It is the
// pack's DEFINITIONS chunk, so it also declares the single inheritance root
// (`container_dim_mm`) the service repositories inherit from.
//
// The table is a first-class `catalogues` entry (ADR-0057 §D2): named entries,
// each supplying every declared field with a value of the declared type. It is
// data the compiler understands, not a component pretending to be data, so
// nothing has to be generated from it by comprehension — the binding's value
// domain IS the table's entry ids, checked at ingest rather than by
// construction.
//
// A binding is one shared choice of an entry (ADR-0057 §D3). It is a declared
// closed facet whose values are the catalogue's entries, so an environment
// binds it exactly as it binds `site`, `cfx options` lists it, and
// `cfx explain` names the rules over it.
//
// Two bindings draw from this one table, and the pair is the point:
// `line_container` carries a `derive` table saying which container each site is
// equipped for, so naming the site is enough and an environment never repeats a
// decision the model already made; `sorter_container` carries neither a derive
// table nor a default, so it stays a free decision the environment must state.
package configflux

chunk: #Config & {
	package: "site_catalogue"
	version: "1.0.0"

	definitions: {
		container_dim_mm: {
			type:      "integer"
			unit:      "mm"
			lifecycle: "startup"
			access:    "integrator"
			doc:       "An outside dimension of a catalogue container, in millimetres."
		}
	}

	facets: {
		site: {
			values: ["factory_a", "factory_b"]
			default: "factory_a"
			doc:     "Which deployment site this configuration is for."
		}
	}

	catalogues: {
		containers: {
			doc: "The physical containers the deployment sites are equipped with."
			fields: {
				length_mm: {type: "integer", unit: "mm", doc: "Outside length."}
				width_mm: {type:  "integer", unit: "mm", doc: "Outside width."}
				height_mm: {type: "integer", unit: "mm", doc: "Outside height."}
			}
			entries: {
				c1: {length_mm: 1200, width_mm: 800, height_mm: 1000}
				c2: {length_mm: 800, width_mm:  600, height_mm: 700}
				c3: {length_mm: 600, width_mm:  400, height_mm: 400}
			}
		}
	}

	// Two bindings draw from the SAME table, and they are the two shapes a
	// shared choice comes in.
	bindings: {
		line_container: {
			catalogue: "containers"
			doc:       "The container the line runs at the selected site."
			// Each site is equipped for one container. This DERIVES the
			// container rather than merely validating the pair: an environment
			// that names its site leaves the container to the model.
			derive: {
				site: {
					factory_a: "c1"
					factory_b: "c2"
				}
			}
		}
		sorter_container: {
			catalogue: "containers"
			doc:       "The container the sorter handles. A free decision per site."
			// No `default` and no `derive`, deliberately. Nothing in the model
			// decides this one, so an environment that leaves it out is refused
			// rather than quietly defaulted — which is what makes it a FREE
			// decision, and what `environments.json` is for.
		}
	}
}
