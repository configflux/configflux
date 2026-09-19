// 06-catalogue-polyrepo — the SORTER repository's chunk.
//
// The third service repository, and the one that shows the OTHER two places a
// catalogue can be declared. The catalogue repository owns `containers`,
// because every unit on the line shares it. `lane_profile` is nobody else's
// business, so this unit declares it itself — a catalogue is an ordinary
// top-level namespace and any unit may carry one (ADR-0057 §D2). The third
// place is the integration unit, for a table that belongs to the deployment
// rather than to any one service; this example does not need one.
//
// Two requirements, both in the bare form: `sorter_container` draws from the
// SHARED table but through a DIFFERENT binding than the line's, so the sorter
// handles a container of its own while `vision_service` and `compute_service`
// share theirs. Same catalogue, different choice — which is exactly what a
// binding is for, and why sameness is spelled "same binding" rather than "same
// catalogue".
//
// `lane_width_margin_mm` inherits `container_dim_mm` from the CATALOGUE
// repository's definitions chunk, which is a third repository's definition
// reaching a third repository's parameter: inheritance is resolved across the
// whole pack at export time, not per repository.
package configflux

chunk: #Config & {
	package: "sorter_service"
	version: "1.0.0"

	catalogues: {
		lane_profile: {
			doc: "The belt profiles this sorter can be built with."
			fields: {
				lane_count: {type: "integer", doc: "Parallel lanes on the belt."}
				belt_width_mm: {type: "integer", unit: "mm", doc: "Belt width."}
			}
			entries: {
				narrow: {lane_count: 2, belt_width_mm: 400}
				wide: {lane_count:   4, belt_width_mm: 900}
			}
		}
	}

	bindings: {
		sorter_lanes: {
			catalogue: "lane_profile"
			default:   "narrow"
			doc:       "Which belt profile this sorter is built with."
		}
	}

	// The facet-equality form, ADR-0057 §D5. Uncomment the block below, then
	// re-export the pack with the command in this example's README, to require
	// that the sorter and the line draw the same container:
	//
	//	constraints: {
	//		sorter_matches_line: {
	//			condition: "sorter_container == line_container"
	//			doc:       "The sorter and the line handle the same container."
	//		}
	//	}
	//
	// The right-hand side is UNQUOTED, which is what makes it a facet name
	// rather than the literal value `line_container`. With it enabled,
	// `environments.json` stops resolving: both sites bind `sorter_container`
	// to `c3` while `line_container` derives to `c1` or `c2`, and no container
	// satisfies both rules at once. It therefore ships commented — an example
	// whose default state does not resolve teaches nothing. `run.sh` step 4c
	// enables it on a COPY of this unit's exported JSON and selects what an
	// environment actually states — `site=factory_a` plus the free
	// `sorter_container=c3`, never the derived `line_container` — so the rule is
	// machine-checked anyway: `cfx resolve` refuses it with exit 3 and writes
	// nothing, and because the contradiction is reached THROUGH the derived
	// binding, `cfx explain`'s core names the derive link
	// (`derive:line_container:site=factory_a`) as well as
	// `sorter_matches_line` itself.

	components: {
		sorter_service: {
			type: "service"
			requires: {
				container: "sorter_container"
				lanes:     "sorter_lanes"
			}
			params: {
				lane_width_margin_mm: {
					inherits: "container_dim_mm"
					doc:      "Clearance held either side of a container on the belt."
					value:    25
				}
				divert_delay_ms: {
					type:      "integer"
					unit:      "ms"
					lifecycle: "startup"
					access:    "integrator"
					doc:       "Delay between the read point and the diverter firing."
					value:     120
					overrides: [
						{condition: "site == 'factory_b'", value: 80},
					]
				}
			}
		}
	}
}
