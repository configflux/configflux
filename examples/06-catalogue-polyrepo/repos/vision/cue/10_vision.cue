// 06-catalogue-polyrepo — the VISION repository's chunk.
//
// A components chunk that lives in its own repository and declares, in
// `requires`, that it needs one container from the shared line (ADR-0057 §D4).
// That declaration is enforced two ways: compiling this chunk alone fails with
// E_REQUIRES_INVALID, because the binding is declared in another repository and
// the compile set is the unit of verification; and a resolve delivers the
// entry INSIDE this component, so the service reads
// `requires.container.width_mm` and never has to know where the catalogue
// lives.
//
// A requirement is not a `depends_on` edge. It names a shared CHOICE, not
// another component, so it joins no dependency closure — which is exactly why
// scoping a resolve to this service alone still hands it the container.
//
// `roi_margin_mm` inherits `container_dim_mm` — a definition declared in a
// DIFFERENT repository. Inheritance is resolved across the whole pack at
// export time (examples/export_pack.sh), so the exported chunk already carries
// the inherited type, unit, lifecycle and access. The `doc` is authored here
// and therefore wins: inheritance fills gaps, it does not overwrite.
package configflux

chunk: #Config & {
	package: "vision_service"
	version: "1.0.0"

	components: {
		vision_service: {
			type: "service"
			requires: container: "line_container"
			params: {
				frame_rate_hz: {
					type:      "integer"
					unit:      "hz"
					lifecycle: "startup"
					access:    "integrator"
					doc:       "Camera frame rate for the inspection loop."
					value:     30
					overrides: [
						{condition: "site == 'factory_b'", value: 60},
					]
				}
				roi_margin_mm: {
					inherits: "container_dim_mm"
					doc:      "Margin held around the container when framing the region of interest."
					value:    40
				}
			}
		}
	}
}
