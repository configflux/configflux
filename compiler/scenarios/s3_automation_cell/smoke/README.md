# S3 Automation Cell Smoke Profile

Loop 3 adds a constrained-selection smoke dataset for multi-facet narrowing.

Facets:
- `conveyor_brand`: `swiftmove`, `beltmax`
- `vision_stack`: `opticore`, `camplus`
- `safety_mode`: `pl_d`, `pl_e`
- `network_topology`: `ring`, `star`

Intentional constraints:
- `swiftmove` only pairs with `opticore` on `ring`.
- `beltmax` only pairs with `camplus` on `star`.
- `safety_mode` can narrow independently to `pl_d` or `pl_e` within each valid path.
