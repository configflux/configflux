// 06-catalogue-polyrepo — the COMPUTE repository's chunk.
//
// The second service repository. Like the vision repository it requires the
// line's container and inherits `container_dim_mm` from the catalogue
// repository's definitions chunk — and, like it, it cannot be verified on its
// own.
//
// It adds one thing the vision service does not: an `accepts` list. This
// service can only work with the two containers named, so an entry outside the
// list is never OFFERED by `cfx options` and a forced choice is refused by the
// requirement's name (ADR-0057 §D4).
//
// The catalogue holds THREE containers, so this list is doing real work: `c3`
// is a legal entry of `containers` and a legal value of `line_container`, and
// it is this list alone that keeps the line off it. `cfx options` never offers
// it, and `--select line_container=c3` is refused naming
// `accepts:compute_service.container` — even though the vision service, which
// declares no list, would have taken it.
//
// The three service repositories never reference each other. Each is compiled
// together with the catalogue; the catalogue is what they share.
package configflux

chunk: #Config & {
	package: "compute_service"
	version: "1.0.0"

	components: {
		compute_service: {
			type: "service"
			requires: container: {
				binding: "line_container"
				accepts: ["c1", "c2"]
			}
			params: {
				grid_cell_mm: {
					inherits: "container_dim_mm"
					doc:      "Edge length of one occupancy-grid cell over the container footprint."
					value:    50
				}
				worker_threads: {
					type:      "integer"
					lifecycle: "startup"
					access:    "integrator"
					doc:       "Worker threads the planner runs."
					value:     4
					overrides: [
						{condition: "site == 'factory_b'", value: 8},
					]
				}
			}
		}
	}
}
