// s3_automation_cell/large -- 10_components chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/10_components.toml,
// validated against compiler/cue/schema.cue, and exported to 10_components.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s3_automation_cell",
  "version": "1.0.0",
  "artifacts": {
    "swift_ring_driver": {
      "name": "swift_ring_driver",
      "version": "1.0.0",
      "hash": "sha256-swift-ring-driver",
      "source": "artifact://automation/swift_ring_driver.so",
      "target": "/opt/configflux/drivers/swift_ring_driver.so",
      "doc": "Driver for swiftmove + opticore ring topology"
    },
    "swift_ring_safety_driver": {
      "name": "swift_ring_safety_driver",
      "version": "1.0.0",
      "hash": "sha256-swift-ring-safety-driver",
      "source": "artifact://automation/swift_ring_safety_driver.so",
      "target": "/opt/configflux/drivers/swift_ring_safety_driver.so",
      "doc": "Safety-enhanced driver for swiftmove ring topology"
    },
    "belt_star_driver": {
      "name": "belt_star_driver",
      "version": "2.1.0",
      "hash": "sha256-belt-star-driver",
      "source": "artifact://automation/belt_star_driver.so",
      "target": "/opt/configflux/drivers/belt_star_driver.so",
      "doc": "Driver for beltmax + camplus star topology"
    },
    "belt_star_safety_driver": {
      "name": "belt_star_safety_driver",
      "version": "2.1.0",
      "hash": "sha256-belt-star-safety-driver",
      "source": "artifact://automation/belt_star_safety_driver.so",
      "target": "/opt/configflux/drivers/belt_star_safety_driver.so",
      "doc": "Safety-enhanced driver for beltmax star topology"
    },
    "helper_driver_001": {
      "name": "helper_driver_001",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-001",
      "source": "artifact://automation/helper_driver_001.so",
      "target": "/opt/configflux/drivers/helper_driver_001.so",
      "doc": "Helper cluster driver 001"
    },
    "helper_leaf_driver_001": {
      "name": "helper_leaf_driver_001",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-001",
      "source": "artifact://automation/helper_leaf_driver_001.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_001.so",
      "doc": "Helper leaf driver 001"
    },
    "helper_driver_002": {
      "name": "helper_driver_002",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-002",
      "source": "artifact://automation/helper_driver_002.so",
      "target": "/opt/configflux/drivers/helper_driver_002.so",
      "doc": "Helper cluster driver 002"
    },
    "helper_leaf_driver_002": {
      "name": "helper_leaf_driver_002",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-002",
      "source": "artifact://automation/helper_leaf_driver_002.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_002.so",
      "doc": "Helper leaf driver 002"
    },
    "helper_driver_003": {
      "name": "helper_driver_003",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-003",
      "source": "artifact://automation/helper_driver_003.so",
      "target": "/opt/configflux/drivers/helper_driver_003.so",
      "doc": "Helper cluster driver 003"
    },
    "helper_leaf_driver_003": {
      "name": "helper_leaf_driver_003",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-003",
      "source": "artifact://automation/helper_leaf_driver_003.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_003.so",
      "doc": "Helper leaf driver 003"
    },
    "helper_driver_004": {
      "name": "helper_driver_004",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-004",
      "source": "artifact://automation/helper_driver_004.so",
      "target": "/opt/configflux/drivers/helper_driver_004.so",
      "doc": "Helper cluster driver 004"
    },
    "helper_leaf_driver_004": {
      "name": "helper_leaf_driver_004",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-004",
      "source": "artifact://automation/helper_leaf_driver_004.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_004.so",
      "doc": "Helper leaf driver 004"
    },
    "helper_driver_005": {
      "name": "helper_driver_005",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-005",
      "source": "artifact://automation/helper_driver_005.so",
      "target": "/opt/configflux/drivers/helper_driver_005.so",
      "doc": "Helper cluster driver 005"
    },
    "helper_leaf_driver_005": {
      "name": "helper_leaf_driver_005",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-005",
      "source": "artifact://automation/helper_leaf_driver_005.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_005.so",
      "doc": "Helper leaf driver 005"
    },
    "helper_driver_006": {
      "name": "helper_driver_006",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-006",
      "source": "artifact://automation/helper_driver_006.so",
      "target": "/opt/configflux/drivers/helper_driver_006.so",
      "doc": "Helper cluster driver 006"
    },
    "helper_leaf_driver_006": {
      "name": "helper_leaf_driver_006",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-006",
      "source": "artifact://automation/helper_leaf_driver_006.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_006.so",
      "doc": "Helper leaf driver 006"
    },
    "helper_driver_007": {
      "name": "helper_driver_007",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-007",
      "source": "artifact://automation/helper_driver_007.so",
      "target": "/opt/configflux/drivers/helper_driver_007.so",
      "doc": "Helper cluster driver 007"
    },
    "helper_leaf_driver_007": {
      "name": "helper_leaf_driver_007",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-007",
      "source": "artifact://automation/helper_leaf_driver_007.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_007.so",
      "doc": "Helper leaf driver 007"
    },
    "helper_driver_008": {
      "name": "helper_driver_008",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-008",
      "source": "artifact://automation/helper_driver_008.so",
      "target": "/opt/configflux/drivers/helper_driver_008.so",
      "doc": "Helper cluster driver 008"
    },
    "helper_leaf_driver_008": {
      "name": "helper_leaf_driver_008",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-008",
      "source": "artifact://automation/helper_leaf_driver_008.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_008.so",
      "doc": "Helper leaf driver 008"
    },
    "helper_driver_009": {
      "name": "helper_driver_009",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-009",
      "source": "artifact://automation/helper_driver_009.so",
      "target": "/opt/configflux/drivers/helper_driver_009.so",
      "doc": "Helper cluster driver 009"
    },
    "helper_leaf_driver_009": {
      "name": "helper_leaf_driver_009",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-009",
      "source": "artifact://automation/helper_leaf_driver_009.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_009.so",
      "doc": "Helper leaf driver 009"
    },
    "helper_driver_010": {
      "name": "helper_driver_010",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-010",
      "source": "artifact://automation/helper_driver_010.so",
      "target": "/opt/configflux/drivers/helper_driver_010.so",
      "doc": "Helper cluster driver 010"
    },
    "helper_leaf_driver_010": {
      "name": "helper_leaf_driver_010",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-010",
      "source": "artifact://automation/helper_leaf_driver_010.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_010.so",
      "doc": "Helper leaf driver 010"
    },
    "helper_driver_011": {
      "name": "helper_driver_011",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-011",
      "source": "artifact://automation/helper_driver_011.so",
      "target": "/opt/configflux/drivers/helper_driver_011.so",
      "doc": "Helper cluster driver 011"
    },
    "helper_leaf_driver_011": {
      "name": "helper_leaf_driver_011",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-011",
      "source": "artifact://automation/helper_leaf_driver_011.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_011.so",
      "doc": "Helper leaf driver 011"
    },
    "helper_driver_012": {
      "name": "helper_driver_012",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-012",
      "source": "artifact://automation/helper_driver_012.so",
      "target": "/opt/configflux/drivers/helper_driver_012.so",
      "doc": "Helper cluster driver 012"
    },
    "helper_leaf_driver_012": {
      "name": "helper_leaf_driver_012",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-012",
      "source": "artifact://automation/helper_leaf_driver_012.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_012.so",
      "doc": "Helper leaf driver 012"
    },
    "helper_driver_013": {
      "name": "helper_driver_013",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-013",
      "source": "artifact://automation/helper_driver_013.so",
      "target": "/opt/configflux/drivers/helper_driver_013.so",
      "doc": "Helper cluster driver 013"
    },
    "helper_leaf_driver_013": {
      "name": "helper_leaf_driver_013",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-013",
      "source": "artifact://automation/helper_leaf_driver_013.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_013.so",
      "doc": "Helper leaf driver 013"
    },
    "helper_driver_014": {
      "name": "helper_driver_014",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-014",
      "source": "artifact://automation/helper_driver_014.so",
      "target": "/opt/configflux/drivers/helper_driver_014.so",
      "doc": "Helper cluster driver 014"
    },
    "helper_leaf_driver_014": {
      "name": "helper_leaf_driver_014",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-014",
      "source": "artifact://automation/helper_leaf_driver_014.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_014.so",
      "doc": "Helper leaf driver 014"
    },
    "helper_driver_015": {
      "name": "helper_driver_015",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-015",
      "source": "artifact://automation/helper_driver_015.so",
      "target": "/opt/configflux/drivers/helper_driver_015.so",
      "doc": "Helper cluster driver 015"
    },
    "helper_leaf_driver_015": {
      "name": "helper_leaf_driver_015",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-015",
      "source": "artifact://automation/helper_leaf_driver_015.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_015.so",
      "doc": "Helper leaf driver 015"
    },
    "helper_driver_016": {
      "name": "helper_driver_016",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-016",
      "source": "artifact://automation/helper_driver_016.so",
      "target": "/opt/configflux/drivers/helper_driver_016.so",
      "doc": "Helper cluster driver 016"
    },
    "helper_leaf_driver_016": {
      "name": "helper_leaf_driver_016",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-016",
      "source": "artifact://automation/helper_leaf_driver_016.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_016.so",
      "doc": "Helper leaf driver 016"
    },
    "helper_driver_017": {
      "name": "helper_driver_017",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-017",
      "source": "artifact://automation/helper_driver_017.so",
      "target": "/opt/configflux/drivers/helper_driver_017.so",
      "doc": "Helper cluster driver 017"
    },
    "helper_leaf_driver_017": {
      "name": "helper_leaf_driver_017",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-017",
      "source": "artifact://automation/helper_leaf_driver_017.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_017.so",
      "doc": "Helper leaf driver 017"
    },
    "helper_driver_018": {
      "name": "helper_driver_018",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-018",
      "source": "artifact://automation/helper_driver_018.so",
      "target": "/opt/configflux/drivers/helper_driver_018.so",
      "doc": "Helper cluster driver 018"
    },
    "helper_leaf_driver_018": {
      "name": "helper_leaf_driver_018",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-018",
      "source": "artifact://automation/helper_leaf_driver_018.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_018.so",
      "doc": "Helper leaf driver 018"
    },
    "helper_driver_019": {
      "name": "helper_driver_019",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-019",
      "source": "artifact://automation/helper_driver_019.so",
      "target": "/opt/configflux/drivers/helper_driver_019.so",
      "doc": "Helper cluster driver 019"
    },
    "helper_leaf_driver_019": {
      "name": "helper_leaf_driver_019",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-019",
      "source": "artifact://automation/helper_leaf_driver_019.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_019.so",
      "doc": "Helper leaf driver 019"
    },
    "helper_driver_020": {
      "name": "helper_driver_020",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-020",
      "source": "artifact://automation/helper_driver_020.so",
      "target": "/opt/configflux/drivers/helper_driver_020.so",
      "doc": "Helper cluster driver 020"
    },
    "helper_leaf_driver_020": {
      "name": "helper_leaf_driver_020",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-020",
      "source": "artifact://automation/helper_leaf_driver_020.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_020.so",
      "doc": "Helper leaf driver 020"
    },
    "helper_driver_021": {
      "name": "helper_driver_021",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-021",
      "source": "artifact://automation/helper_driver_021.so",
      "target": "/opt/configflux/drivers/helper_driver_021.so",
      "doc": "Helper cluster driver 021"
    },
    "helper_leaf_driver_021": {
      "name": "helper_leaf_driver_021",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-021",
      "source": "artifact://automation/helper_leaf_driver_021.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_021.so",
      "doc": "Helper leaf driver 021"
    },
    "helper_driver_022": {
      "name": "helper_driver_022",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-022",
      "source": "artifact://automation/helper_driver_022.so",
      "target": "/opt/configflux/drivers/helper_driver_022.so",
      "doc": "Helper cluster driver 022"
    },
    "helper_leaf_driver_022": {
      "name": "helper_leaf_driver_022",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-022",
      "source": "artifact://automation/helper_leaf_driver_022.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_022.so",
      "doc": "Helper leaf driver 022"
    },
    "helper_driver_023": {
      "name": "helper_driver_023",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-023",
      "source": "artifact://automation/helper_driver_023.so",
      "target": "/opt/configflux/drivers/helper_driver_023.so",
      "doc": "Helper cluster driver 023"
    },
    "helper_leaf_driver_023": {
      "name": "helper_leaf_driver_023",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-023",
      "source": "artifact://automation/helper_leaf_driver_023.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_023.so",
      "doc": "Helper leaf driver 023"
    },
    "helper_driver_024": {
      "name": "helper_driver_024",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-024",
      "source": "artifact://automation/helper_driver_024.so",
      "target": "/opt/configflux/drivers/helper_driver_024.so",
      "doc": "Helper cluster driver 024"
    },
    "helper_leaf_driver_024": {
      "name": "helper_leaf_driver_024",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-024",
      "source": "artifact://automation/helper_leaf_driver_024.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_024.so",
      "doc": "Helper leaf driver 024"
    },
    "helper_driver_025": {
      "name": "helper_driver_025",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-025",
      "source": "artifact://automation/helper_driver_025.so",
      "target": "/opt/configflux/drivers/helper_driver_025.so",
      "doc": "Helper cluster driver 025"
    },
    "helper_leaf_driver_025": {
      "name": "helper_leaf_driver_025",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-025",
      "source": "artifact://automation/helper_leaf_driver_025.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_025.so",
      "doc": "Helper leaf driver 025"
    },
    "helper_driver_026": {
      "name": "helper_driver_026",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-026",
      "source": "artifact://automation/helper_driver_026.so",
      "target": "/opt/configflux/drivers/helper_driver_026.so",
      "doc": "Helper cluster driver 026"
    },
    "helper_leaf_driver_026": {
      "name": "helper_leaf_driver_026",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-026",
      "source": "artifact://automation/helper_leaf_driver_026.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_026.so",
      "doc": "Helper leaf driver 026"
    },
    "helper_driver_027": {
      "name": "helper_driver_027",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-027",
      "source": "artifact://automation/helper_driver_027.so",
      "target": "/opt/configflux/drivers/helper_driver_027.so",
      "doc": "Helper cluster driver 027"
    },
    "helper_leaf_driver_027": {
      "name": "helper_leaf_driver_027",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-027",
      "source": "artifact://automation/helper_leaf_driver_027.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_027.so",
      "doc": "Helper leaf driver 027"
    },
    "helper_driver_028": {
      "name": "helper_driver_028",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-028",
      "source": "artifact://automation/helper_driver_028.so",
      "target": "/opt/configflux/drivers/helper_driver_028.so",
      "doc": "Helper cluster driver 028"
    },
    "helper_leaf_driver_028": {
      "name": "helper_leaf_driver_028",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-028",
      "source": "artifact://automation/helper_leaf_driver_028.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_028.so",
      "doc": "Helper leaf driver 028"
    },
    "helper_driver_029": {
      "name": "helper_driver_029",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-029",
      "source": "artifact://automation/helper_driver_029.so",
      "target": "/opt/configflux/drivers/helper_driver_029.so",
      "doc": "Helper cluster driver 029"
    },
    "helper_leaf_driver_029": {
      "name": "helper_leaf_driver_029",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-029",
      "source": "artifact://automation/helper_leaf_driver_029.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_029.so",
      "doc": "Helper leaf driver 029"
    },
    "helper_driver_030": {
      "name": "helper_driver_030",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-030",
      "source": "artifact://automation/helper_driver_030.so",
      "target": "/opt/configflux/drivers/helper_driver_030.so",
      "doc": "Helper cluster driver 030"
    },
    "helper_leaf_driver_030": {
      "name": "helper_leaf_driver_030",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-030",
      "source": "artifact://automation/helper_leaf_driver_030.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_030.so",
      "doc": "Helper leaf driver 030"
    },
    "helper_driver_031": {
      "name": "helper_driver_031",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-031",
      "source": "artifact://automation/helper_driver_031.so",
      "target": "/opt/configflux/drivers/helper_driver_031.so",
      "doc": "Helper cluster driver 031"
    },
    "helper_leaf_driver_031": {
      "name": "helper_leaf_driver_031",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-031",
      "source": "artifact://automation/helper_leaf_driver_031.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_031.so",
      "doc": "Helper leaf driver 031"
    },
    "helper_driver_032": {
      "name": "helper_driver_032",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-032",
      "source": "artifact://automation/helper_driver_032.so",
      "target": "/opt/configflux/drivers/helper_driver_032.so",
      "doc": "Helper cluster driver 032"
    },
    "helper_leaf_driver_032": {
      "name": "helper_leaf_driver_032",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-032",
      "source": "artifact://automation/helper_leaf_driver_032.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_032.so",
      "doc": "Helper leaf driver 032"
    },
    "helper_driver_033": {
      "name": "helper_driver_033",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-033",
      "source": "artifact://automation/helper_driver_033.so",
      "target": "/opt/configflux/drivers/helper_driver_033.so",
      "doc": "Helper cluster driver 033"
    },
    "helper_leaf_driver_033": {
      "name": "helper_leaf_driver_033",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-033",
      "source": "artifact://automation/helper_leaf_driver_033.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_033.so",
      "doc": "Helper leaf driver 033"
    },
    "helper_driver_034": {
      "name": "helper_driver_034",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-034",
      "source": "artifact://automation/helper_driver_034.so",
      "target": "/opt/configflux/drivers/helper_driver_034.so",
      "doc": "Helper cluster driver 034"
    },
    "helper_leaf_driver_034": {
      "name": "helper_leaf_driver_034",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-034",
      "source": "artifact://automation/helper_leaf_driver_034.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_034.so",
      "doc": "Helper leaf driver 034"
    },
    "helper_driver_035": {
      "name": "helper_driver_035",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-035",
      "source": "artifact://automation/helper_driver_035.so",
      "target": "/opt/configflux/drivers/helper_driver_035.so",
      "doc": "Helper cluster driver 035"
    },
    "helper_leaf_driver_035": {
      "name": "helper_leaf_driver_035",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-035",
      "source": "artifact://automation/helper_leaf_driver_035.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_035.so",
      "doc": "Helper leaf driver 035"
    },
    "helper_driver_036": {
      "name": "helper_driver_036",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-036",
      "source": "artifact://automation/helper_driver_036.so",
      "target": "/opt/configflux/drivers/helper_driver_036.so",
      "doc": "Helper cluster driver 036"
    },
    "helper_leaf_driver_036": {
      "name": "helper_leaf_driver_036",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-036",
      "source": "artifact://automation/helper_leaf_driver_036.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_036.so",
      "doc": "Helper leaf driver 036"
    },
    "helper_driver_037": {
      "name": "helper_driver_037",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-037",
      "source": "artifact://automation/helper_driver_037.so",
      "target": "/opt/configflux/drivers/helper_driver_037.so",
      "doc": "Helper cluster driver 037"
    },
    "helper_leaf_driver_037": {
      "name": "helper_leaf_driver_037",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-037",
      "source": "artifact://automation/helper_leaf_driver_037.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_037.so",
      "doc": "Helper leaf driver 037"
    },
    "helper_driver_038": {
      "name": "helper_driver_038",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-038",
      "source": "artifact://automation/helper_driver_038.so",
      "target": "/opt/configflux/drivers/helper_driver_038.so",
      "doc": "Helper cluster driver 038"
    },
    "helper_leaf_driver_038": {
      "name": "helper_leaf_driver_038",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-038",
      "source": "artifact://automation/helper_leaf_driver_038.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_038.so",
      "doc": "Helper leaf driver 038"
    },
    "helper_driver_039": {
      "name": "helper_driver_039",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-039",
      "source": "artifact://automation/helper_driver_039.so",
      "target": "/opt/configflux/drivers/helper_driver_039.so",
      "doc": "Helper cluster driver 039"
    },
    "helper_leaf_driver_039": {
      "name": "helper_leaf_driver_039",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-039",
      "source": "artifact://automation/helper_leaf_driver_039.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_039.so",
      "doc": "Helper leaf driver 039"
    },
    "helper_driver_040": {
      "name": "helper_driver_040",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-040",
      "source": "artifact://automation/helper_driver_040.so",
      "target": "/opt/configflux/drivers/helper_driver_040.so",
      "doc": "Helper cluster driver 040"
    },
    "helper_leaf_driver_040": {
      "name": "helper_leaf_driver_040",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-040",
      "source": "artifact://automation/helper_leaf_driver_040.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_040.so",
      "doc": "Helper leaf driver 040"
    },
    "helper_driver_041": {
      "name": "helper_driver_041",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-041",
      "source": "artifact://automation/helper_driver_041.so",
      "target": "/opt/configflux/drivers/helper_driver_041.so",
      "doc": "Helper cluster driver 041"
    },
    "helper_leaf_driver_041": {
      "name": "helper_leaf_driver_041",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-041",
      "source": "artifact://automation/helper_leaf_driver_041.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_041.so",
      "doc": "Helper leaf driver 041"
    },
    "helper_driver_042": {
      "name": "helper_driver_042",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-042",
      "source": "artifact://automation/helper_driver_042.so",
      "target": "/opt/configflux/drivers/helper_driver_042.so",
      "doc": "Helper cluster driver 042"
    },
    "helper_leaf_driver_042": {
      "name": "helper_leaf_driver_042",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-042",
      "source": "artifact://automation/helper_leaf_driver_042.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_042.so",
      "doc": "Helper leaf driver 042"
    },
    "helper_driver_043": {
      "name": "helper_driver_043",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-043",
      "source": "artifact://automation/helper_driver_043.so",
      "target": "/opt/configflux/drivers/helper_driver_043.so",
      "doc": "Helper cluster driver 043"
    },
    "helper_leaf_driver_043": {
      "name": "helper_leaf_driver_043",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-043",
      "source": "artifact://automation/helper_leaf_driver_043.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_043.so",
      "doc": "Helper leaf driver 043"
    },
    "helper_driver_044": {
      "name": "helper_driver_044",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-044",
      "source": "artifact://automation/helper_driver_044.so",
      "target": "/opt/configflux/drivers/helper_driver_044.so",
      "doc": "Helper cluster driver 044"
    },
    "helper_leaf_driver_044": {
      "name": "helper_leaf_driver_044",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-044",
      "source": "artifact://automation/helper_leaf_driver_044.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_044.so",
      "doc": "Helper leaf driver 044"
    },
    "helper_driver_045": {
      "name": "helper_driver_045",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-045",
      "source": "artifact://automation/helper_driver_045.so",
      "target": "/opt/configflux/drivers/helper_driver_045.so",
      "doc": "Helper cluster driver 045"
    },
    "helper_leaf_driver_045": {
      "name": "helper_leaf_driver_045",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-045",
      "source": "artifact://automation/helper_leaf_driver_045.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_045.so",
      "doc": "Helper leaf driver 045"
    },
    "helper_driver_046": {
      "name": "helper_driver_046",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-046",
      "source": "artifact://automation/helper_driver_046.so",
      "target": "/opt/configflux/drivers/helper_driver_046.so",
      "doc": "Helper cluster driver 046"
    },
    "helper_leaf_driver_046": {
      "name": "helper_leaf_driver_046",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-046",
      "source": "artifact://automation/helper_leaf_driver_046.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_046.so",
      "doc": "Helper leaf driver 046"
    },
    "helper_driver_047": {
      "name": "helper_driver_047",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-047",
      "source": "artifact://automation/helper_driver_047.so",
      "target": "/opt/configflux/drivers/helper_driver_047.so",
      "doc": "Helper cluster driver 047"
    },
    "helper_leaf_driver_047": {
      "name": "helper_leaf_driver_047",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-047",
      "source": "artifact://automation/helper_leaf_driver_047.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_047.so",
      "doc": "Helper leaf driver 047"
    },
    "helper_driver_048": {
      "name": "helper_driver_048",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-048",
      "source": "artifact://automation/helper_driver_048.so",
      "target": "/opt/configflux/drivers/helper_driver_048.so",
      "doc": "Helper cluster driver 048"
    },
    "helper_leaf_driver_048": {
      "name": "helper_leaf_driver_048",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-048",
      "source": "artifact://automation/helper_leaf_driver_048.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_048.so",
      "doc": "Helper leaf driver 048"
    },
    "helper_driver_049": {
      "name": "helper_driver_049",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-049",
      "source": "artifact://automation/helper_driver_049.so",
      "target": "/opt/configflux/drivers/helper_driver_049.so",
      "doc": "Helper cluster driver 049"
    },
    "helper_leaf_driver_049": {
      "name": "helper_leaf_driver_049",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-049",
      "source": "artifact://automation/helper_leaf_driver_049.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_049.so",
      "doc": "Helper leaf driver 049"
    },
    "helper_driver_050": {
      "name": "helper_driver_050",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-050",
      "source": "artifact://automation/helper_driver_050.so",
      "target": "/opt/configflux/drivers/helper_driver_050.so",
      "doc": "Helper cluster driver 050"
    },
    "helper_leaf_driver_050": {
      "name": "helper_leaf_driver_050",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-050",
      "source": "artifact://automation/helper_leaf_driver_050.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_050.so",
      "doc": "Helper leaf driver 050"
    },
    "helper_driver_051": {
      "name": "helper_driver_051",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-051",
      "source": "artifact://automation/helper_driver_051.so",
      "target": "/opt/configflux/drivers/helper_driver_051.so",
      "doc": "Helper cluster driver 051"
    },
    "helper_leaf_driver_051": {
      "name": "helper_leaf_driver_051",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-051",
      "source": "artifact://automation/helper_leaf_driver_051.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_051.so",
      "doc": "Helper leaf driver 051"
    },
    "helper_driver_052": {
      "name": "helper_driver_052",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-052",
      "source": "artifact://automation/helper_driver_052.so",
      "target": "/opt/configflux/drivers/helper_driver_052.so",
      "doc": "Helper cluster driver 052"
    },
    "helper_leaf_driver_052": {
      "name": "helper_leaf_driver_052",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-052",
      "source": "artifact://automation/helper_leaf_driver_052.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_052.so",
      "doc": "Helper leaf driver 052"
    },
    "helper_driver_053": {
      "name": "helper_driver_053",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-053",
      "source": "artifact://automation/helper_driver_053.so",
      "target": "/opt/configflux/drivers/helper_driver_053.so",
      "doc": "Helper cluster driver 053"
    },
    "helper_leaf_driver_053": {
      "name": "helper_leaf_driver_053",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-053",
      "source": "artifact://automation/helper_leaf_driver_053.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_053.so",
      "doc": "Helper leaf driver 053"
    },
    "helper_driver_054": {
      "name": "helper_driver_054",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-054",
      "source": "artifact://automation/helper_driver_054.so",
      "target": "/opt/configflux/drivers/helper_driver_054.so",
      "doc": "Helper cluster driver 054"
    },
    "helper_leaf_driver_054": {
      "name": "helper_leaf_driver_054",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-054",
      "source": "artifact://automation/helper_leaf_driver_054.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_054.so",
      "doc": "Helper leaf driver 054"
    },
    "helper_driver_055": {
      "name": "helper_driver_055",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-055",
      "source": "artifact://automation/helper_driver_055.so",
      "target": "/opt/configflux/drivers/helper_driver_055.so",
      "doc": "Helper cluster driver 055"
    },
    "helper_leaf_driver_055": {
      "name": "helper_leaf_driver_055",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-055",
      "source": "artifact://automation/helper_leaf_driver_055.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_055.so",
      "doc": "Helper leaf driver 055"
    },
    "helper_driver_056": {
      "name": "helper_driver_056",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-056",
      "source": "artifact://automation/helper_driver_056.so",
      "target": "/opt/configflux/drivers/helper_driver_056.so",
      "doc": "Helper cluster driver 056"
    },
    "helper_leaf_driver_056": {
      "name": "helper_leaf_driver_056",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-056",
      "source": "artifact://automation/helper_leaf_driver_056.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_056.so",
      "doc": "Helper leaf driver 056"
    },
    "helper_driver_057": {
      "name": "helper_driver_057",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-057",
      "source": "artifact://automation/helper_driver_057.so",
      "target": "/opt/configflux/drivers/helper_driver_057.so",
      "doc": "Helper cluster driver 057"
    },
    "helper_leaf_driver_057": {
      "name": "helper_leaf_driver_057",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-057",
      "source": "artifact://automation/helper_leaf_driver_057.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_057.so",
      "doc": "Helper leaf driver 057"
    },
    "helper_driver_058": {
      "name": "helper_driver_058",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-058",
      "source": "artifact://automation/helper_driver_058.so",
      "target": "/opt/configflux/drivers/helper_driver_058.so",
      "doc": "Helper cluster driver 058"
    },
    "helper_leaf_driver_058": {
      "name": "helper_leaf_driver_058",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-058",
      "source": "artifact://automation/helper_leaf_driver_058.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_058.so",
      "doc": "Helper leaf driver 058"
    },
    "helper_driver_059": {
      "name": "helper_driver_059",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-059",
      "source": "artifact://automation/helper_driver_059.so",
      "target": "/opt/configflux/drivers/helper_driver_059.so",
      "doc": "Helper cluster driver 059"
    },
    "helper_leaf_driver_059": {
      "name": "helper_leaf_driver_059",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-059",
      "source": "artifact://automation/helper_leaf_driver_059.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_059.so",
      "doc": "Helper leaf driver 059"
    },
    "helper_driver_060": {
      "name": "helper_driver_060",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-060",
      "source": "artifact://automation/helper_driver_060.so",
      "target": "/opt/configflux/drivers/helper_driver_060.so",
      "doc": "Helper cluster driver 060"
    },
    "helper_leaf_driver_060": {
      "name": "helper_leaf_driver_060",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-060",
      "source": "artifact://automation/helper_leaf_driver_060.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_060.so",
      "doc": "Helper leaf driver 060"
    },
    "helper_driver_061": {
      "name": "helper_driver_061",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-061",
      "source": "artifact://automation/helper_driver_061.so",
      "target": "/opt/configflux/drivers/helper_driver_061.so",
      "doc": "Helper cluster driver 061"
    },
    "helper_leaf_driver_061": {
      "name": "helper_leaf_driver_061",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-061",
      "source": "artifact://automation/helper_leaf_driver_061.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_061.so",
      "doc": "Helper leaf driver 061"
    },
    "helper_driver_062": {
      "name": "helper_driver_062",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-062",
      "source": "artifact://automation/helper_driver_062.so",
      "target": "/opt/configflux/drivers/helper_driver_062.so",
      "doc": "Helper cluster driver 062"
    },
    "helper_leaf_driver_062": {
      "name": "helper_leaf_driver_062",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-062",
      "source": "artifact://automation/helper_leaf_driver_062.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_062.so",
      "doc": "Helper leaf driver 062"
    },
    "helper_driver_063": {
      "name": "helper_driver_063",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-063",
      "source": "artifact://automation/helper_driver_063.so",
      "target": "/opt/configflux/drivers/helper_driver_063.so",
      "doc": "Helper cluster driver 063"
    },
    "helper_leaf_driver_063": {
      "name": "helper_leaf_driver_063",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-063",
      "source": "artifact://automation/helper_leaf_driver_063.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_063.so",
      "doc": "Helper leaf driver 063"
    },
    "helper_driver_064": {
      "name": "helper_driver_064",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-064",
      "source": "artifact://automation/helper_driver_064.so",
      "target": "/opt/configflux/drivers/helper_driver_064.so",
      "doc": "Helper cluster driver 064"
    },
    "helper_leaf_driver_064": {
      "name": "helper_leaf_driver_064",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-064",
      "source": "artifact://automation/helper_leaf_driver_064.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_064.so",
      "doc": "Helper leaf driver 064"
    },
    "helper_driver_065": {
      "name": "helper_driver_065",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-065",
      "source": "artifact://automation/helper_driver_065.so",
      "target": "/opt/configflux/drivers/helper_driver_065.so",
      "doc": "Helper cluster driver 065"
    },
    "helper_leaf_driver_065": {
      "name": "helper_leaf_driver_065",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-065",
      "source": "artifact://automation/helper_leaf_driver_065.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_065.so",
      "doc": "Helper leaf driver 065"
    },
    "helper_driver_066": {
      "name": "helper_driver_066",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-066",
      "source": "artifact://automation/helper_driver_066.so",
      "target": "/opt/configflux/drivers/helper_driver_066.so",
      "doc": "Helper cluster driver 066"
    },
    "helper_leaf_driver_066": {
      "name": "helper_leaf_driver_066",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-066",
      "source": "artifact://automation/helper_leaf_driver_066.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_066.so",
      "doc": "Helper leaf driver 066"
    },
    "helper_driver_067": {
      "name": "helper_driver_067",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-067",
      "source": "artifact://automation/helper_driver_067.so",
      "target": "/opt/configflux/drivers/helper_driver_067.so",
      "doc": "Helper cluster driver 067"
    },
    "helper_leaf_driver_067": {
      "name": "helper_leaf_driver_067",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-067",
      "source": "artifact://automation/helper_leaf_driver_067.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_067.so",
      "doc": "Helper leaf driver 067"
    },
    "helper_driver_068": {
      "name": "helper_driver_068",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-068",
      "source": "artifact://automation/helper_driver_068.so",
      "target": "/opt/configflux/drivers/helper_driver_068.so",
      "doc": "Helper cluster driver 068"
    },
    "helper_leaf_driver_068": {
      "name": "helper_leaf_driver_068",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-068",
      "source": "artifact://automation/helper_leaf_driver_068.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_068.so",
      "doc": "Helper leaf driver 068"
    },
    "helper_driver_069": {
      "name": "helper_driver_069",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-069",
      "source": "artifact://automation/helper_driver_069.so",
      "target": "/opt/configflux/drivers/helper_driver_069.so",
      "doc": "Helper cluster driver 069"
    },
    "helper_leaf_driver_069": {
      "name": "helper_leaf_driver_069",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-069",
      "source": "artifact://automation/helper_leaf_driver_069.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_069.so",
      "doc": "Helper leaf driver 069"
    },
    "helper_driver_070": {
      "name": "helper_driver_070",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-070",
      "source": "artifact://automation/helper_driver_070.so",
      "target": "/opt/configflux/drivers/helper_driver_070.so",
      "doc": "Helper cluster driver 070"
    },
    "helper_leaf_driver_070": {
      "name": "helper_leaf_driver_070",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-070",
      "source": "artifact://automation/helper_leaf_driver_070.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_070.so",
      "doc": "Helper leaf driver 070"
    },
    "helper_driver_071": {
      "name": "helper_driver_071",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-071",
      "source": "artifact://automation/helper_driver_071.so",
      "target": "/opt/configflux/drivers/helper_driver_071.so",
      "doc": "Helper cluster driver 071"
    },
    "helper_leaf_driver_071": {
      "name": "helper_leaf_driver_071",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-071",
      "source": "artifact://automation/helper_leaf_driver_071.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_071.so",
      "doc": "Helper leaf driver 071"
    },
    "helper_driver_072": {
      "name": "helper_driver_072",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-072",
      "source": "artifact://automation/helper_driver_072.so",
      "target": "/opt/configflux/drivers/helper_driver_072.so",
      "doc": "Helper cluster driver 072"
    },
    "helper_leaf_driver_072": {
      "name": "helper_leaf_driver_072",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-072",
      "source": "artifact://automation/helper_leaf_driver_072.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_072.so",
      "doc": "Helper leaf driver 072"
    },
    "helper_driver_073": {
      "name": "helper_driver_073",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-073",
      "source": "artifact://automation/helper_driver_073.so",
      "target": "/opt/configflux/drivers/helper_driver_073.so",
      "doc": "Helper cluster driver 073"
    },
    "helper_leaf_driver_073": {
      "name": "helper_leaf_driver_073",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-073",
      "source": "artifact://automation/helper_leaf_driver_073.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_073.so",
      "doc": "Helper leaf driver 073"
    },
    "helper_driver_074": {
      "name": "helper_driver_074",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-074",
      "source": "artifact://automation/helper_driver_074.so",
      "target": "/opt/configflux/drivers/helper_driver_074.so",
      "doc": "Helper cluster driver 074"
    },
    "helper_leaf_driver_074": {
      "name": "helper_leaf_driver_074",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-074",
      "source": "artifact://automation/helper_leaf_driver_074.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_074.so",
      "doc": "Helper leaf driver 074"
    },
    "helper_driver_075": {
      "name": "helper_driver_075",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-075",
      "source": "artifact://automation/helper_driver_075.so",
      "target": "/opt/configflux/drivers/helper_driver_075.so",
      "doc": "Helper cluster driver 075"
    },
    "helper_leaf_driver_075": {
      "name": "helper_leaf_driver_075",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-075",
      "source": "artifact://automation/helper_leaf_driver_075.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_075.so",
      "doc": "Helper leaf driver 075"
    },
    "helper_driver_076": {
      "name": "helper_driver_076",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-076",
      "source": "artifact://automation/helper_driver_076.so",
      "target": "/opt/configflux/drivers/helper_driver_076.so",
      "doc": "Helper cluster driver 076"
    },
    "helper_leaf_driver_076": {
      "name": "helper_leaf_driver_076",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-076",
      "source": "artifact://automation/helper_leaf_driver_076.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_076.so",
      "doc": "Helper leaf driver 076"
    },
    "helper_driver_077": {
      "name": "helper_driver_077",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-077",
      "source": "artifact://automation/helper_driver_077.so",
      "target": "/opt/configflux/drivers/helper_driver_077.so",
      "doc": "Helper cluster driver 077"
    },
    "helper_leaf_driver_077": {
      "name": "helper_leaf_driver_077",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-077",
      "source": "artifact://automation/helper_leaf_driver_077.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_077.so",
      "doc": "Helper leaf driver 077"
    },
    "helper_driver_078": {
      "name": "helper_driver_078",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-078",
      "source": "artifact://automation/helper_driver_078.so",
      "target": "/opt/configflux/drivers/helper_driver_078.so",
      "doc": "Helper cluster driver 078"
    },
    "helper_leaf_driver_078": {
      "name": "helper_leaf_driver_078",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-078",
      "source": "artifact://automation/helper_leaf_driver_078.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_078.so",
      "doc": "Helper leaf driver 078"
    },
    "helper_driver_079": {
      "name": "helper_driver_079",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-079",
      "source": "artifact://automation/helper_driver_079.so",
      "target": "/opt/configflux/drivers/helper_driver_079.so",
      "doc": "Helper cluster driver 079"
    },
    "helper_leaf_driver_079": {
      "name": "helper_leaf_driver_079",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-079",
      "source": "artifact://automation/helper_leaf_driver_079.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_079.so",
      "doc": "Helper leaf driver 079"
    },
    "helper_driver_080": {
      "name": "helper_driver_080",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-080",
      "source": "artifact://automation/helper_driver_080.so",
      "target": "/opt/configflux/drivers/helper_driver_080.so",
      "doc": "Helper cluster driver 080"
    },
    "helper_leaf_driver_080": {
      "name": "helper_leaf_driver_080",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-080",
      "source": "artifact://automation/helper_leaf_driver_080.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_080.so",
      "doc": "Helper leaf driver 080"
    },
    "helper_driver_081": {
      "name": "helper_driver_081",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-081",
      "source": "artifact://automation/helper_driver_081.so",
      "target": "/opt/configflux/drivers/helper_driver_081.so",
      "doc": "Helper cluster driver 081"
    },
    "helper_leaf_driver_081": {
      "name": "helper_leaf_driver_081",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-081",
      "source": "artifact://automation/helper_leaf_driver_081.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_081.so",
      "doc": "Helper leaf driver 081"
    },
    "helper_driver_082": {
      "name": "helper_driver_082",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-082",
      "source": "artifact://automation/helper_driver_082.so",
      "target": "/opt/configflux/drivers/helper_driver_082.so",
      "doc": "Helper cluster driver 082"
    },
    "helper_leaf_driver_082": {
      "name": "helper_leaf_driver_082",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-082",
      "source": "artifact://automation/helper_leaf_driver_082.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_082.so",
      "doc": "Helper leaf driver 082"
    },
    "helper_driver_083": {
      "name": "helper_driver_083",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-083",
      "source": "artifact://automation/helper_driver_083.so",
      "target": "/opt/configflux/drivers/helper_driver_083.so",
      "doc": "Helper cluster driver 083"
    },
    "helper_leaf_driver_083": {
      "name": "helper_leaf_driver_083",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-083",
      "source": "artifact://automation/helper_leaf_driver_083.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_083.so",
      "doc": "Helper leaf driver 083"
    },
    "helper_driver_084": {
      "name": "helper_driver_084",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-084",
      "source": "artifact://automation/helper_driver_084.so",
      "target": "/opt/configflux/drivers/helper_driver_084.so",
      "doc": "Helper cluster driver 084"
    },
    "helper_leaf_driver_084": {
      "name": "helper_leaf_driver_084",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-084",
      "source": "artifact://automation/helper_leaf_driver_084.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_084.so",
      "doc": "Helper leaf driver 084"
    },
    "helper_driver_085": {
      "name": "helper_driver_085",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-085",
      "source": "artifact://automation/helper_driver_085.so",
      "target": "/opt/configflux/drivers/helper_driver_085.so",
      "doc": "Helper cluster driver 085"
    },
    "helper_leaf_driver_085": {
      "name": "helper_leaf_driver_085",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-085",
      "source": "artifact://automation/helper_leaf_driver_085.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_085.so",
      "doc": "Helper leaf driver 085"
    },
    "helper_driver_086": {
      "name": "helper_driver_086",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-086",
      "source": "artifact://automation/helper_driver_086.so",
      "target": "/opt/configflux/drivers/helper_driver_086.so",
      "doc": "Helper cluster driver 086"
    },
    "helper_leaf_driver_086": {
      "name": "helper_leaf_driver_086",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-086",
      "source": "artifact://automation/helper_leaf_driver_086.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_086.so",
      "doc": "Helper leaf driver 086"
    },
    "helper_driver_087": {
      "name": "helper_driver_087",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-087",
      "source": "artifact://automation/helper_driver_087.so",
      "target": "/opt/configflux/drivers/helper_driver_087.so",
      "doc": "Helper cluster driver 087"
    },
    "helper_leaf_driver_087": {
      "name": "helper_leaf_driver_087",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-087",
      "source": "artifact://automation/helper_leaf_driver_087.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_087.so",
      "doc": "Helper leaf driver 087"
    },
    "helper_driver_088": {
      "name": "helper_driver_088",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-088",
      "source": "artifact://automation/helper_driver_088.so",
      "target": "/opt/configflux/drivers/helper_driver_088.so",
      "doc": "Helper cluster driver 088"
    },
    "helper_leaf_driver_088": {
      "name": "helper_leaf_driver_088",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-088",
      "source": "artifact://automation/helper_leaf_driver_088.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_088.so",
      "doc": "Helper leaf driver 088"
    },
    "helper_driver_089": {
      "name": "helper_driver_089",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-089",
      "source": "artifact://automation/helper_driver_089.so",
      "target": "/opt/configflux/drivers/helper_driver_089.so",
      "doc": "Helper cluster driver 089"
    },
    "helper_leaf_driver_089": {
      "name": "helper_leaf_driver_089",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-089",
      "source": "artifact://automation/helper_leaf_driver_089.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_089.so",
      "doc": "Helper leaf driver 089"
    },
    "helper_driver_090": {
      "name": "helper_driver_090",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-090",
      "source": "artifact://automation/helper_driver_090.so",
      "target": "/opt/configflux/drivers/helper_driver_090.so",
      "doc": "Helper cluster driver 090"
    },
    "helper_leaf_driver_090": {
      "name": "helper_leaf_driver_090",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-090",
      "source": "artifact://automation/helper_leaf_driver_090.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_090.so",
      "doc": "Helper leaf driver 090"
    },
    "helper_driver_091": {
      "name": "helper_driver_091",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-091",
      "source": "artifact://automation/helper_driver_091.so",
      "target": "/opt/configflux/drivers/helper_driver_091.so",
      "doc": "Helper cluster driver 091"
    },
    "helper_leaf_driver_091": {
      "name": "helper_leaf_driver_091",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-091",
      "source": "artifact://automation/helper_leaf_driver_091.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_091.so",
      "doc": "Helper leaf driver 091"
    },
    "helper_driver_092": {
      "name": "helper_driver_092",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-092",
      "source": "artifact://automation/helper_driver_092.so",
      "target": "/opt/configflux/drivers/helper_driver_092.so",
      "doc": "Helper cluster driver 092"
    },
    "helper_leaf_driver_092": {
      "name": "helper_leaf_driver_092",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-092",
      "source": "artifact://automation/helper_leaf_driver_092.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_092.so",
      "doc": "Helper leaf driver 092"
    },
    "helper_driver_093": {
      "name": "helper_driver_093",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-093",
      "source": "artifact://automation/helper_driver_093.so",
      "target": "/opt/configflux/drivers/helper_driver_093.so",
      "doc": "Helper cluster driver 093"
    },
    "helper_leaf_driver_093": {
      "name": "helper_leaf_driver_093",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-093",
      "source": "artifact://automation/helper_leaf_driver_093.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_093.so",
      "doc": "Helper leaf driver 093"
    },
    "helper_driver_094": {
      "name": "helper_driver_094",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-094",
      "source": "artifact://automation/helper_driver_094.so",
      "target": "/opt/configflux/drivers/helper_driver_094.so",
      "doc": "Helper cluster driver 094"
    },
    "helper_leaf_driver_094": {
      "name": "helper_leaf_driver_094",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-094",
      "source": "artifact://automation/helper_leaf_driver_094.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_094.so",
      "doc": "Helper leaf driver 094"
    },
    "helper_driver_095": {
      "name": "helper_driver_095",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-095",
      "source": "artifact://automation/helper_driver_095.so",
      "target": "/opt/configflux/drivers/helper_driver_095.so",
      "doc": "Helper cluster driver 095"
    },
    "helper_leaf_driver_095": {
      "name": "helper_leaf_driver_095",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-095",
      "source": "artifact://automation/helper_leaf_driver_095.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_095.so",
      "doc": "Helper leaf driver 095"
    },
    "helper_driver_096": {
      "name": "helper_driver_096",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-096",
      "source": "artifact://automation/helper_driver_096.so",
      "target": "/opt/configflux/drivers/helper_driver_096.so",
      "doc": "Helper cluster driver 096"
    },
    "helper_leaf_driver_096": {
      "name": "helper_leaf_driver_096",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-096",
      "source": "artifact://automation/helper_leaf_driver_096.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_096.so",
      "doc": "Helper leaf driver 096"
    },
    "helper_driver_097": {
      "name": "helper_driver_097",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-097",
      "source": "artifact://automation/helper_driver_097.so",
      "target": "/opt/configflux/drivers/helper_driver_097.so",
      "doc": "Helper cluster driver 097"
    },
    "helper_leaf_driver_097": {
      "name": "helper_leaf_driver_097",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-097",
      "source": "artifact://automation/helper_leaf_driver_097.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_097.so",
      "doc": "Helper leaf driver 097"
    },
    "helper_driver_098": {
      "name": "helper_driver_098",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-098",
      "source": "artifact://automation/helper_driver_098.so",
      "target": "/opt/configflux/drivers/helper_driver_098.so",
      "doc": "Helper cluster driver 098"
    },
    "helper_leaf_driver_098": {
      "name": "helper_leaf_driver_098",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-098",
      "source": "artifact://automation/helper_leaf_driver_098.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_098.so",
      "doc": "Helper leaf driver 098"
    },
    "helper_driver_099": {
      "name": "helper_driver_099",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-099",
      "source": "artifact://automation/helper_driver_099.so",
      "target": "/opt/configflux/drivers/helper_driver_099.so",
      "doc": "Helper cluster driver 099"
    },
    "helper_leaf_driver_099": {
      "name": "helper_leaf_driver_099",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-099",
      "source": "artifact://automation/helper_leaf_driver_099.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_099.so",
      "doc": "Helper leaf driver 099"
    },
    "helper_driver_100": {
      "name": "helper_driver_100",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-100",
      "source": "artifact://automation/helper_driver_100.so",
      "target": "/opt/configflux/drivers/helper_driver_100.so",
      "doc": "Helper cluster driver 100"
    },
    "helper_leaf_driver_100": {
      "name": "helper_leaf_driver_100",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-100",
      "source": "artifact://automation/helper_leaf_driver_100.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_100.so",
      "doc": "Helper leaf driver 100"
    },
    "helper_driver_101": {
      "name": "helper_driver_101",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-101",
      "source": "artifact://automation/helper_driver_101.so",
      "target": "/opt/configflux/drivers/helper_driver_101.so",
      "doc": "Helper cluster driver 101"
    },
    "helper_leaf_driver_101": {
      "name": "helper_leaf_driver_101",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-101",
      "source": "artifact://automation/helper_leaf_driver_101.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_101.so",
      "doc": "Helper leaf driver 101"
    },
    "helper_driver_102": {
      "name": "helper_driver_102",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-102",
      "source": "artifact://automation/helper_driver_102.so",
      "target": "/opt/configflux/drivers/helper_driver_102.so",
      "doc": "Helper cluster driver 102"
    },
    "helper_leaf_driver_102": {
      "name": "helper_leaf_driver_102",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-102",
      "source": "artifact://automation/helper_leaf_driver_102.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_102.so",
      "doc": "Helper leaf driver 102"
    },
    "helper_driver_103": {
      "name": "helper_driver_103",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-103",
      "source": "artifact://automation/helper_driver_103.so",
      "target": "/opt/configflux/drivers/helper_driver_103.so",
      "doc": "Helper cluster driver 103"
    },
    "helper_leaf_driver_103": {
      "name": "helper_leaf_driver_103",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-103",
      "source": "artifact://automation/helper_leaf_driver_103.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_103.so",
      "doc": "Helper leaf driver 103"
    },
    "helper_driver_104": {
      "name": "helper_driver_104",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-104",
      "source": "artifact://automation/helper_driver_104.so",
      "target": "/opt/configflux/drivers/helper_driver_104.so",
      "doc": "Helper cluster driver 104"
    },
    "helper_leaf_driver_104": {
      "name": "helper_leaf_driver_104",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-104",
      "source": "artifact://automation/helper_leaf_driver_104.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_104.so",
      "doc": "Helper leaf driver 104"
    },
    "helper_driver_105": {
      "name": "helper_driver_105",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-105",
      "source": "artifact://automation/helper_driver_105.so",
      "target": "/opt/configflux/drivers/helper_driver_105.so",
      "doc": "Helper cluster driver 105"
    },
    "helper_leaf_driver_105": {
      "name": "helper_leaf_driver_105",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-105",
      "source": "artifact://automation/helper_leaf_driver_105.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_105.so",
      "doc": "Helper leaf driver 105"
    },
    "helper_driver_106": {
      "name": "helper_driver_106",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-106",
      "source": "artifact://automation/helper_driver_106.so",
      "target": "/opt/configflux/drivers/helper_driver_106.so",
      "doc": "Helper cluster driver 106"
    },
    "helper_leaf_driver_106": {
      "name": "helper_leaf_driver_106",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-106",
      "source": "artifact://automation/helper_leaf_driver_106.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_106.so",
      "doc": "Helper leaf driver 106"
    },
    "helper_driver_107": {
      "name": "helper_driver_107",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-107",
      "source": "artifact://automation/helper_driver_107.so",
      "target": "/opt/configflux/drivers/helper_driver_107.so",
      "doc": "Helper cluster driver 107"
    },
    "helper_leaf_driver_107": {
      "name": "helper_leaf_driver_107",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-107",
      "source": "artifact://automation/helper_leaf_driver_107.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_107.so",
      "doc": "Helper leaf driver 107"
    },
    "helper_driver_108": {
      "name": "helper_driver_108",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-108",
      "source": "artifact://automation/helper_driver_108.so",
      "target": "/opt/configflux/drivers/helper_driver_108.so",
      "doc": "Helper cluster driver 108"
    },
    "helper_leaf_driver_108": {
      "name": "helper_leaf_driver_108",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-108",
      "source": "artifact://automation/helper_leaf_driver_108.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_108.so",
      "doc": "Helper leaf driver 108"
    },
    "helper_driver_109": {
      "name": "helper_driver_109",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-109",
      "source": "artifact://automation/helper_driver_109.so",
      "target": "/opt/configflux/drivers/helper_driver_109.so",
      "doc": "Helper cluster driver 109"
    },
    "helper_leaf_driver_109": {
      "name": "helper_leaf_driver_109",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-109",
      "source": "artifact://automation/helper_leaf_driver_109.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_109.so",
      "doc": "Helper leaf driver 109"
    },
    "helper_driver_110": {
      "name": "helper_driver_110",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-110",
      "source": "artifact://automation/helper_driver_110.so",
      "target": "/opt/configflux/drivers/helper_driver_110.so",
      "doc": "Helper cluster driver 110"
    },
    "helper_leaf_driver_110": {
      "name": "helper_leaf_driver_110",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-110",
      "source": "artifact://automation/helper_leaf_driver_110.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_110.so",
      "doc": "Helper leaf driver 110"
    },
    "helper_driver_111": {
      "name": "helper_driver_111",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-111",
      "source": "artifact://automation/helper_driver_111.so",
      "target": "/opt/configflux/drivers/helper_driver_111.so",
      "doc": "Helper cluster driver 111"
    },
    "helper_leaf_driver_111": {
      "name": "helper_leaf_driver_111",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-111",
      "source": "artifact://automation/helper_leaf_driver_111.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_111.so",
      "doc": "Helper leaf driver 111"
    },
    "helper_driver_112": {
      "name": "helper_driver_112",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-112",
      "source": "artifact://automation/helper_driver_112.so",
      "target": "/opt/configflux/drivers/helper_driver_112.so",
      "doc": "Helper cluster driver 112"
    },
    "helper_leaf_driver_112": {
      "name": "helper_leaf_driver_112",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-112",
      "source": "artifact://automation/helper_leaf_driver_112.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_112.so",
      "doc": "Helper leaf driver 112"
    },
    "helper_driver_113": {
      "name": "helper_driver_113",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-113",
      "source": "artifact://automation/helper_driver_113.so",
      "target": "/opt/configflux/drivers/helper_driver_113.so",
      "doc": "Helper cluster driver 113"
    },
    "helper_leaf_driver_113": {
      "name": "helper_leaf_driver_113",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-113",
      "source": "artifact://automation/helper_leaf_driver_113.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_113.so",
      "doc": "Helper leaf driver 113"
    },
    "helper_driver_114": {
      "name": "helper_driver_114",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-114",
      "source": "artifact://automation/helper_driver_114.so",
      "target": "/opt/configflux/drivers/helper_driver_114.so",
      "doc": "Helper cluster driver 114"
    },
    "helper_leaf_driver_114": {
      "name": "helper_leaf_driver_114",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-114",
      "source": "artifact://automation/helper_leaf_driver_114.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_114.so",
      "doc": "Helper leaf driver 114"
    },
    "helper_driver_115": {
      "name": "helper_driver_115",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-115",
      "source": "artifact://automation/helper_driver_115.so",
      "target": "/opt/configflux/drivers/helper_driver_115.so",
      "doc": "Helper cluster driver 115"
    },
    "helper_leaf_driver_115": {
      "name": "helper_leaf_driver_115",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-115",
      "source": "artifact://automation/helper_leaf_driver_115.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_115.so",
      "doc": "Helper leaf driver 115"
    },
    "helper_driver_116": {
      "name": "helper_driver_116",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-116",
      "source": "artifact://automation/helper_driver_116.so",
      "target": "/opt/configflux/drivers/helper_driver_116.so",
      "doc": "Helper cluster driver 116"
    },
    "helper_leaf_driver_116": {
      "name": "helper_leaf_driver_116",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-116",
      "source": "artifact://automation/helper_leaf_driver_116.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_116.so",
      "doc": "Helper leaf driver 116"
    },
    "helper_driver_117": {
      "name": "helper_driver_117",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-117",
      "source": "artifact://automation/helper_driver_117.so",
      "target": "/opt/configflux/drivers/helper_driver_117.so",
      "doc": "Helper cluster driver 117"
    },
    "helper_leaf_driver_117": {
      "name": "helper_leaf_driver_117",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-117",
      "source": "artifact://automation/helper_leaf_driver_117.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_117.so",
      "doc": "Helper leaf driver 117"
    },
    "helper_driver_118": {
      "name": "helper_driver_118",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-118",
      "source": "artifact://automation/helper_driver_118.so",
      "target": "/opt/configflux/drivers/helper_driver_118.so",
      "doc": "Helper cluster driver 118"
    },
    "helper_leaf_driver_118": {
      "name": "helper_leaf_driver_118",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-118",
      "source": "artifact://automation/helper_leaf_driver_118.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_118.so",
      "doc": "Helper leaf driver 118"
    },
    "helper_driver_119": {
      "name": "helper_driver_119",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-119",
      "source": "artifact://automation/helper_driver_119.so",
      "target": "/opt/configflux/drivers/helper_driver_119.so",
      "doc": "Helper cluster driver 119"
    },
    "helper_leaf_driver_119": {
      "name": "helper_leaf_driver_119",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-119",
      "source": "artifact://automation/helper_leaf_driver_119.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_119.so",
      "doc": "Helper leaf driver 119"
    },
    "helper_driver_120": {
      "name": "helper_driver_120",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-120",
      "source": "artifact://automation/helper_driver_120.so",
      "target": "/opt/configflux/drivers/helper_driver_120.so",
      "doc": "Helper cluster driver 120"
    },
    "helper_leaf_driver_120": {
      "name": "helper_leaf_driver_120",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-120",
      "source": "artifact://automation/helper_leaf_driver_120.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_120.so",
      "doc": "Helper leaf driver 120"
    },
    "helper_driver_121": {
      "name": "helper_driver_121",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-121",
      "source": "artifact://automation/helper_driver_121.so",
      "target": "/opt/configflux/drivers/helper_driver_121.so",
      "doc": "Helper cluster driver 121"
    },
    "helper_leaf_driver_121": {
      "name": "helper_leaf_driver_121",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-121",
      "source": "artifact://automation/helper_leaf_driver_121.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_121.so",
      "doc": "Helper leaf driver 121"
    },
    "helper_driver_122": {
      "name": "helper_driver_122",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-122",
      "source": "artifact://automation/helper_driver_122.so",
      "target": "/opt/configflux/drivers/helper_driver_122.so",
      "doc": "Helper cluster driver 122"
    },
    "helper_leaf_driver_122": {
      "name": "helper_leaf_driver_122",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-122",
      "source": "artifact://automation/helper_leaf_driver_122.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_122.so",
      "doc": "Helper leaf driver 122"
    },
    "helper_driver_123": {
      "name": "helper_driver_123",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-123",
      "source": "artifact://automation/helper_driver_123.so",
      "target": "/opt/configflux/drivers/helper_driver_123.so",
      "doc": "Helper cluster driver 123"
    },
    "helper_leaf_driver_123": {
      "name": "helper_leaf_driver_123",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-123",
      "source": "artifact://automation/helper_leaf_driver_123.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_123.so",
      "doc": "Helper leaf driver 123"
    },
    "helper_driver_124": {
      "name": "helper_driver_124",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-124",
      "source": "artifact://automation/helper_driver_124.so",
      "target": "/opt/configflux/drivers/helper_driver_124.so",
      "doc": "Helper cluster driver 124"
    },
    "helper_leaf_driver_124": {
      "name": "helper_leaf_driver_124",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-124",
      "source": "artifact://automation/helper_leaf_driver_124.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_124.so",
      "doc": "Helper leaf driver 124"
    },
    "helper_driver_125": {
      "name": "helper_driver_125",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-125",
      "source": "artifact://automation/helper_driver_125.so",
      "target": "/opt/configflux/drivers/helper_driver_125.so",
      "doc": "Helper cluster driver 125"
    },
    "helper_leaf_driver_125": {
      "name": "helper_leaf_driver_125",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-125",
      "source": "artifact://automation/helper_leaf_driver_125.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_125.so",
      "doc": "Helper leaf driver 125"
    },
    "helper_driver_126": {
      "name": "helper_driver_126",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-126",
      "source": "artifact://automation/helper_driver_126.so",
      "target": "/opt/configflux/drivers/helper_driver_126.so",
      "doc": "Helper cluster driver 126"
    },
    "helper_leaf_driver_126": {
      "name": "helper_leaf_driver_126",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-126",
      "source": "artifact://automation/helper_leaf_driver_126.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_126.so",
      "doc": "Helper leaf driver 126"
    },
    "helper_driver_127": {
      "name": "helper_driver_127",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-127",
      "source": "artifact://automation/helper_driver_127.so",
      "target": "/opt/configflux/drivers/helper_driver_127.so",
      "doc": "Helper cluster driver 127"
    },
    "helper_leaf_driver_127": {
      "name": "helper_leaf_driver_127",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-127",
      "source": "artifact://automation/helper_leaf_driver_127.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_127.so",
      "doc": "Helper leaf driver 127"
    },
    "helper_driver_128": {
      "name": "helper_driver_128",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-128",
      "source": "artifact://automation/helper_driver_128.so",
      "target": "/opt/configflux/drivers/helper_driver_128.so",
      "doc": "Helper cluster driver 128"
    },
    "helper_leaf_driver_128": {
      "name": "helper_leaf_driver_128",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-128",
      "source": "artifact://automation/helper_leaf_driver_128.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_128.so",
      "doc": "Helper leaf driver 128"
    },
    "helper_driver_129": {
      "name": "helper_driver_129",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-129",
      "source": "artifact://automation/helper_driver_129.so",
      "target": "/opt/configflux/drivers/helper_driver_129.so",
      "doc": "Helper cluster driver 129"
    },
    "helper_leaf_driver_129": {
      "name": "helper_leaf_driver_129",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-129",
      "source": "artifact://automation/helper_leaf_driver_129.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_129.so",
      "doc": "Helper leaf driver 129"
    },
    "helper_driver_130": {
      "name": "helper_driver_130",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-130",
      "source": "artifact://automation/helper_driver_130.so",
      "target": "/opt/configflux/drivers/helper_driver_130.so",
      "doc": "Helper cluster driver 130"
    },
    "helper_leaf_driver_130": {
      "name": "helper_leaf_driver_130",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-130",
      "source": "artifact://automation/helper_leaf_driver_130.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_130.so",
      "doc": "Helper leaf driver 130"
    },
    "helper_driver_131": {
      "name": "helper_driver_131",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-131",
      "source": "artifact://automation/helper_driver_131.so",
      "target": "/opt/configflux/drivers/helper_driver_131.so",
      "doc": "Helper cluster driver 131"
    },
    "helper_leaf_driver_131": {
      "name": "helper_leaf_driver_131",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-131",
      "source": "artifact://automation/helper_leaf_driver_131.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_131.so",
      "doc": "Helper leaf driver 131"
    },
    "helper_driver_132": {
      "name": "helper_driver_132",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-132",
      "source": "artifact://automation/helper_driver_132.so",
      "target": "/opt/configflux/drivers/helper_driver_132.so",
      "doc": "Helper cluster driver 132"
    },
    "helper_leaf_driver_132": {
      "name": "helper_leaf_driver_132",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-132",
      "source": "artifact://automation/helper_leaf_driver_132.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_132.so",
      "doc": "Helper leaf driver 132"
    },
    "helper_driver_133": {
      "name": "helper_driver_133",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-133",
      "source": "artifact://automation/helper_driver_133.so",
      "target": "/opt/configflux/drivers/helper_driver_133.so",
      "doc": "Helper cluster driver 133"
    },
    "helper_leaf_driver_133": {
      "name": "helper_leaf_driver_133",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-133",
      "source": "artifact://automation/helper_leaf_driver_133.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_133.so",
      "doc": "Helper leaf driver 133"
    },
    "helper_driver_134": {
      "name": "helper_driver_134",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-134",
      "source": "artifact://automation/helper_driver_134.so",
      "target": "/opt/configflux/drivers/helper_driver_134.so",
      "doc": "Helper cluster driver 134"
    },
    "helper_leaf_driver_134": {
      "name": "helper_leaf_driver_134",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-134",
      "source": "artifact://automation/helper_leaf_driver_134.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_134.so",
      "doc": "Helper leaf driver 134"
    },
    "helper_driver_135": {
      "name": "helper_driver_135",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-135",
      "source": "artifact://automation/helper_driver_135.so",
      "target": "/opt/configflux/drivers/helper_driver_135.so",
      "doc": "Helper cluster driver 135"
    },
    "helper_leaf_driver_135": {
      "name": "helper_leaf_driver_135",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-135",
      "source": "artifact://automation/helper_leaf_driver_135.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_135.so",
      "doc": "Helper leaf driver 135"
    },
    "helper_driver_136": {
      "name": "helper_driver_136",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-136",
      "source": "artifact://automation/helper_driver_136.so",
      "target": "/opt/configflux/drivers/helper_driver_136.so",
      "doc": "Helper cluster driver 136"
    },
    "helper_leaf_driver_136": {
      "name": "helper_leaf_driver_136",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-136",
      "source": "artifact://automation/helper_leaf_driver_136.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_136.so",
      "doc": "Helper leaf driver 136"
    },
    "helper_driver_137": {
      "name": "helper_driver_137",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-137",
      "source": "artifact://automation/helper_driver_137.so",
      "target": "/opt/configflux/drivers/helper_driver_137.so",
      "doc": "Helper cluster driver 137"
    },
    "helper_leaf_driver_137": {
      "name": "helper_leaf_driver_137",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-137",
      "source": "artifact://automation/helper_leaf_driver_137.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_137.so",
      "doc": "Helper leaf driver 137"
    },
    "helper_driver_138": {
      "name": "helper_driver_138",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-138",
      "source": "artifact://automation/helper_driver_138.so",
      "target": "/opt/configflux/drivers/helper_driver_138.so",
      "doc": "Helper cluster driver 138"
    },
    "helper_leaf_driver_138": {
      "name": "helper_leaf_driver_138",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-138",
      "source": "artifact://automation/helper_leaf_driver_138.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_138.so",
      "doc": "Helper leaf driver 138"
    },
    "helper_driver_139": {
      "name": "helper_driver_139",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-139",
      "source": "artifact://automation/helper_driver_139.so",
      "target": "/opt/configflux/drivers/helper_driver_139.so",
      "doc": "Helper cluster driver 139"
    },
    "helper_leaf_driver_139": {
      "name": "helper_leaf_driver_139",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-139",
      "source": "artifact://automation/helper_leaf_driver_139.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_139.so",
      "doc": "Helper leaf driver 139"
    },
    "helper_driver_140": {
      "name": "helper_driver_140",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-140",
      "source": "artifact://automation/helper_driver_140.so",
      "target": "/opt/configflux/drivers/helper_driver_140.so",
      "doc": "Helper cluster driver 140"
    },
    "helper_leaf_driver_140": {
      "name": "helper_leaf_driver_140",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-140",
      "source": "artifact://automation/helper_leaf_driver_140.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_140.so",
      "doc": "Helper leaf driver 140"
    },
    "helper_driver_141": {
      "name": "helper_driver_141",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-141",
      "source": "artifact://automation/helper_driver_141.so",
      "target": "/opt/configflux/drivers/helper_driver_141.so",
      "doc": "Helper cluster driver 141"
    },
    "helper_leaf_driver_141": {
      "name": "helper_leaf_driver_141",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-141",
      "source": "artifact://automation/helper_leaf_driver_141.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_141.so",
      "doc": "Helper leaf driver 141"
    },
    "helper_driver_142": {
      "name": "helper_driver_142",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-142",
      "source": "artifact://automation/helper_driver_142.so",
      "target": "/opt/configflux/drivers/helper_driver_142.so",
      "doc": "Helper cluster driver 142"
    },
    "helper_leaf_driver_142": {
      "name": "helper_leaf_driver_142",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-142",
      "source": "artifact://automation/helper_leaf_driver_142.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_142.so",
      "doc": "Helper leaf driver 142"
    },
    "helper_driver_143": {
      "name": "helper_driver_143",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-143",
      "source": "artifact://automation/helper_driver_143.so",
      "target": "/opt/configflux/drivers/helper_driver_143.so",
      "doc": "Helper cluster driver 143"
    },
    "helper_leaf_driver_143": {
      "name": "helper_leaf_driver_143",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-143",
      "source": "artifact://automation/helper_leaf_driver_143.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_143.so",
      "doc": "Helper leaf driver 143"
    },
    "helper_driver_144": {
      "name": "helper_driver_144",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-144",
      "source": "artifact://automation/helper_driver_144.so",
      "target": "/opt/configflux/drivers/helper_driver_144.so",
      "doc": "Helper cluster driver 144"
    },
    "helper_leaf_driver_144": {
      "name": "helper_leaf_driver_144",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-144",
      "source": "artifact://automation/helper_leaf_driver_144.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_144.so",
      "doc": "Helper leaf driver 144"
    },
    "helper_driver_145": {
      "name": "helper_driver_145",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-145",
      "source": "artifact://automation/helper_driver_145.so",
      "target": "/opt/configflux/drivers/helper_driver_145.so",
      "doc": "Helper cluster driver 145"
    },
    "helper_leaf_driver_145": {
      "name": "helper_leaf_driver_145",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-145",
      "source": "artifact://automation/helper_leaf_driver_145.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_145.so",
      "doc": "Helper leaf driver 145"
    },
    "helper_driver_146": {
      "name": "helper_driver_146",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-146",
      "source": "artifact://automation/helper_driver_146.so",
      "target": "/opt/configflux/drivers/helper_driver_146.so",
      "doc": "Helper cluster driver 146"
    },
    "helper_leaf_driver_146": {
      "name": "helper_leaf_driver_146",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-146",
      "source": "artifact://automation/helper_leaf_driver_146.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_146.so",
      "doc": "Helper leaf driver 146"
    },
    "helper_driver_147": {
      "name": "helper_driver_147",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-147",
      "source": "artifact://automation/helper_driver_147.so",
      "target": "/opt/configflux/drivers/helper_driver_147.so",
      "doc": "Helper cluster driver 147"
    },
    "helper_leaf_driver_147": {
      "name": "helper_leaf_driver_147",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-147",
      "source": "artifact://automation/helper_leaf_driver_147.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_147.so",
      "doc": "Helper leaf driver 147"
    },
    "helper_driver_148": {
      "name": "helper_driver_148",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-148",
      "source": "artifact://automation/helper_driver_148.so",
      "target": "/opt/configflux/drivers/helper_driver_148.so",
      "doc": "Helper cluster driver 148"
    },
    "helper_leaf_driver_148": {
      "name": "helper_leaf_driver_148",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-148",
      "source": "artifact://automation/helper_leaf_driver_148.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_148.so",
      "doc": "Helper leaf driver 148"
    },
    "helper_driver_149": {
      "name": "helper_driver_149",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-149",
      "source": "artifact://automation/helper_driver_149.so",
      "target": "/opt/configflux/drivers/helper_driver_149.so",
      "doc": "Helper cluster driver 149"
    },
    "helper_leaf_driver_149": {
      "name": "helper_leaf_driver_149",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-149",
      "source": "artifact://automation/helper_leaf_driver_149.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_149.so",
      "doc": "Helper leaf driver 149"
    },
    "helper_driver_150": {
      "name": "helper_driver_150",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-150",
      "source": "artifact://automation/helper_driver_150.so",
      "target": "/opt/configflux/drivers/helper_driver_150.so",
      "doc": "Helper cluster driver 150"
    },
    "helper_leaf_driver_150": {
      "name": "helper_leaf_driver_150",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-150",
      "source": "artifact://automation/helper_leaf_driver_150.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_150.so",
      "doc": "Helper leaf driver 150"
    },
    "helper_driver_151": {
      "name": "helper_driver_151",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-151",
      "source": "artifact://automation/helper_driver_151.so",
      "target": "/opt/configflux/drivers/helper_driver_151.so",
      "doc": "Helper cluster driver 151"
    },
    "helper_leaf_driver_151": {
      "name": "helper_leaf_driver_151",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-151",
      "source": "artifact://automation/helper_leaf_driver_151.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_151.so",
      "doc": "Helper leaf driver 151"
    },
    "helper_driver_152": {
      "name": "helper_driver_152",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-152",
      "source": "artifact://automation/helper_driver_152.so",
      "target": "/opt/configflux/drivers/helper_driver_152.so",
      "doc": "Helper cluster driver 152"
    },
    "helper_leaf_driver_152": {
      "name": "helper_leaf_driver_152",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-152",
      "source": "artifact://automation/helper_leaf_driver_152.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_152.so",
      "doc": "Helper leaf driver 152"
    },
    "helper_driver_153": {
      "name": "helper_driver_153",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-153",
      "source": "artifact://automation/helper_driver_153.so",
      "target": "/opt/configflux/drivers/helper_driver_153.so",
      "doc": "Helper cluster driver 153"
    },
    "helper_leaf_driver_153": {
      "name": "helper_leaf_driver_153",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-153",
      "source": "artifact://automation/helper_leaf_driver_153.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_153.so",
      "doc": "Helper leaf driver 153"
    },
    "helper_driver_154": {
      "name": "helper_driver_154",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-154",
      "source": "artifact://automation/helper_driver_154.so",
      "target": "/opt/configflux/drivers/helper_driver_154.so",
      "doc": "Helper cluster driver 154"
    },
    "helper_leaf_driver_154": {
      "name": "helper_leaf_driver_154",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-154",
      "source": "artifact://automation/helper_leaf_driver_154.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_154.so",
      "doc": "Helper leaf driver 154"
    },
    "helper_driver_155": {
      "name": "helper_driver_155",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-155",
      "source": "artifact://automation/helper_driver_155.so",
      "target": "/opt/configflux/drivers/helper_driver_155.so",
      "doc": "Helper cluster driver 155"
    },
    "helper_leaf_driver_155": {
      "name": "helper_leaf_driver_155",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-155",
      "source": "artifact://automation/helper_leaf_driver_155.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_155.so",
      "doc": "Helper leaf driver 155"
    },
    "helper_driver_156": {
      "name": "helper_driver_156",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-156",
      "source": "artifact://automation/helper_driver_156.so",
      "target": "/opt/configflux/drivers/helper_driver_156.so",
      "doc": "Helper cluster driver 156"
    },
    "helper_leaf_driver_156": {
      "name": "helper_leaf_driver_156",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-156",
      "source": "artifact://automation/helper_leaf_driver_156.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_156.so",
      "doc": "Helper leaf driver 156"
    },
    "helper_driver_157": {
      "name": "helper_driver_157",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-157",
      "source": "artifact://automation/helper_driver_157.so",
      "target": "/opt/configflux/drivers/helper_driver_157.so",
      "doc": "Helper cluster driver 157"
    },
    "helper_leaf_driver_157": {
      "name": "helper_leaf_driver_157",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-157",
      "source": "artifact://automation/helper_leaf_driver_157.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_157.so",
      "doc": "Helper leaf driver 157"
    },
    "helper_driver_158": {
      "name": "helper_driver_158",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-158",
      "source": "artifact://automation/helper_driver_158.so",
      "target": "/opt/configflux/drivers/helper_driver_158.so",
      "doc": "Helper cluster driver 158"
    },
    "helper_leaf_driver_158": {
      "name": "helper_leaf_driver_158",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-158",
      "source": "artifact://automation/helper_leaf_driver_158.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_158.so",
      "doc": "Helper leaf driver 158"
    },
    "helper_driver_159": {
      "name": "helper_driver_159",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-159",
      "source": "artifact://automation/helper_driver_159.so",
      "target": "/opt/configflux/drivers/helper_driver_159.so",
      "doc": "Helper cluster driver 159"
    },
    "helper_leaf_driver_159": {
      "name": "helper_leaf_driver_159",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-159",
      "source": "artifact://automation/helper_leaf_driver_159.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_159.so",
      "doc": "Helper leaf driver 159"
    },
    "helper_driver_160": {
      "name": "helper_driver_160",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-160",
      "source": "artifact://automation/helper_driver_160.so",
      "target": "/opt/configflux/drivers/helper_driver_160.so",
      "doc": "Helper cluster driver 160"
    },
    "helper_leaf_driver_160": {
      "name": "helper_leaf_driver_160",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-160",
      "source": "artifact://automation/helper_leaf_driver_160.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_160.so",
      "doc": "Helper leaf driver 160"
    },
    "helper_driver_161": {
      "name": "helper_driver_161",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-161",
      "source": "artifact://automation/helper_driver_161.so",
      "target": "/opt/configflux/drivers/helper_driver_161.so",
      "doc": "Helper cluster driver 161"
    },
    "helper_leaf_driver_161": {
      "name": "helper_leaf_driver_161",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-161",
      "source": "artifact://automation/helper_leaf_driver_161.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_161.so",
      "doc": "Helper leaf driver 161"
    },
    "helper_driver_162": {
      "name": "helper_driver_162",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-162",
      "source": "artifact://automation/helper_driver_162.so",
      "target": "/opt/configflux/drivers/helper_driver_162.so",
      "doc": "Helper cluster driver 162"
    },
    "helper_leaf_driver_162": {
      "name": "helper_leaf_driver_162",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-162",
      "source": "artifact://automation/helper_leaf_driver_162.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_162.so",
      "doc": "Helper leaf driver 162"
    },
    "helper_driver_163": {
      "name": "helper_driver_163",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-163",
      "source": "artifact://automation/helper_driver_163.so",
      "target": "/opt/configflux/drivers/helper_driver_163.so",
      "doc": "Helper cluster driver 163"
    },
    "helper_leaf_driver_163": {
      "name": "helper_leaf_driver_163",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-163",
      "source": "artifact://automation/helper_leaf_driver_163.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_163.so",
      "doc": "Helper leaf driver 163"
    },
    "helper_driver_164": {
      "name": "helper_driver_164",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-164",
      "source": "artifact://automation/helper_driver_164.so",
      "target": "/opt/configflux/drivers/helper_driver_164.so",
      "doc": "Helper cluster driver 164"
    },
    "helper_leaf_driver_164": {
      "name": "helper_leaf_driver_164",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-164",
      "source": "artifact://automation/helper_leaf_driver_164.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_164.so",
      "doc": "Helper leaf driver 164"
    },
    "helper_driver_165": {
      "name": "helper_driver_165",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-165",
      "source": "artifact://automation/helper_driver_165.so",
      "target": "/opt/configflux/drivers/helper_driver_165.so",
      "doc": "Helper cluster driver 165"
    },
    "helper_leaf_driver_165": {
      "name": "helper_leaf_driver_165",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-165",
      "source": "artifact://automation/helper_leaf_driver_165.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_165.so",
      "doc": "Helper leaf driver 165"
    },
    "helper_driver_166": {
      "name": "helper_driver_166",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-166",
      "source": "artifact://automation/helper_driver_166.so",
      "target": "/opt/configflux/drivers/helper_driver_166.so",
      "doc": "Helper cluster driver 166"
    },
    "helper_leaf_driver_166": {
      "name": "helper_leaf_driver_166",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-166",
      "source": "artifact://automation/helper_leaf_driver_166.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_166.so",
      "doc": "Helper leaf driver 166"
    },
    "helper_driver_167": {
      "name": "helper_driver_167",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-167",
      "source": "artifact://automation/helper_driver_167.so",
      "target": "/opt/configflux/drivers/helper_driver_167.so",
      "doc": "Helper cluster driver 167"
    },
    "helper_leaf_driver_167": {
      "name": "helper_leaf_driver_167",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-167",
      "source": "artifact://automation/helper_leaf_driver_167.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_167.so",
      "doc": "Helper leaf driver 167"
    },
    "helper_driver_168": {
      "name": "helper_driver_168",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-168",
      "source": "artifact://automation/helper_driver_168.so",
      "target": "/opt/configflux/drivers/helper_driver_168.so",
      "doc": "Helper cluster driver 168"
    },
    "helper_leaf_driver_168": {
      "name": "helper_leaf_driver_168",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-168",
      "source": "artifact://automation/helper_leaf_driver_168.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_168.so",
      "doc": "Helper leaf driver 168"
    },
    "helper_driver_169": {
      "name": "helper_driver_169",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-169",
      "source": "artifact://automation/helper_driver_169.so",
      "target": "/opt/configflux/drivers/helper_driver_169.so",
      "doc": "Helper cluster driver 169"
    },
    "helper_leaf_driver_169": {
      "name": "helper_leaf_driver_169",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-169",
      "source": "artifact://automation/helper_leaf_driver_169.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_169.so",
      "doc": "Helper leaf driver 169"
    },
    "helper_driver_170": {
      "name": "helper_driver_170",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-170",
      "source": "artifact://automation/helper_driver_170.so",
      "target": "/opt/configflux/drivers/helper_driver_170.so",
      "doc": "Helper cluster driver 170"
    },
    "helper_leaf_driver_170": {
      "name": "helper_leaf_driver_170",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-170",
      "source": "artifact://automation/helper_leaf_driver_170.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_170.so",
      "doc": "Helper leaf driver 170"
    },
    "helper_driver_171": {
      "name": "helper_driver_171",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-171",
      "source": "artifact://automation/helper_driver_171.so",
      "target": "/opt/configflux/drivers/helper_driver_171.so",
      "doc": "Helper cluster driver 171"
    },
    "helper_leaf_driver_171": {
      "name": "helper_leaf_driver_171",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-171",
      "source": "artifact://automation/helper_leaf_driver_171.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_171.so",
      "doc": "Helper leaf driver 171"
    },
    "helper_driver_172": {
      "name": "helper_driver_172",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-172",
      "source": "artifact://automation/helper_driver_172.so",
      "target": "/opt/configflux/drivers/helper_driver_172.so",
      "doc": "Helper cluster driver 172"
    },
    "helper_leaf_driver_172": {
      "name": "helper_leaf_driver_172",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-172",
      "source": "artifact://automation/helper_leaf_driver_172.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_172.so",
      "doc": "Helper leaf driver 172"
    },
    "helper_driver_173": {
      "name": "helper_driver_173",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-173",
      "source": "artifact://automation/helper_driver_173.so",
      "target": "/opt/configflux/drivers/helper_driver_173.so",
      "doc": "Helper cluster driver 173"
    },
    "helper_leaf_driver_173": {
      "name": "helper_leaf_driver_173",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-173",
      "source": "artifact://automation/helper_leaf_driver_173.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_173.so",
      "doc": "Helper leaf driver 173"
    },
    "helper_driver_174": {
      "name": "helper_driver_174",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-174",
      "source": "artifact://automation/helper_driver_174.so",
      "target": "/opt/configflux/drivers/helper_driver_174.so",
      "doc": "Helper cluster driver 174"
    },
    "helper_leaf_driver_174": {
      "name": "helper_leaf_driver_174",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-174",
      "source": "artifact://automation/helper_leaf_driver_174.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_174.so",
      "doc": "Helper leaf driver 174"
    },
    "helper_driver_175": {
      "name": "helper_driver_175",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-175",
      "source": "artifact://automation/helper_driver_175.so",
      "target": "/opt/configflux/drivers/helper_driver_175.so",
      "doc": "Helper cluster driver 175"
    },
    "helper_leaf_driver_175": {
      "name": "helper_leaf_driver_175",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-175",
      "source": "artifact://automation/helper_leaf_driver_175.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_175.so",
      "doc": "Helper leaf driver 175"
    },
    "helper_driver_176": {
      "name": "helper_driver_176",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-176",
      "source": "artifact://automation/helper_driver_176.so",
      "target": "/opt/configflux/drivers/helper_driver_176.so",
      "doc": "Helper cluster driver 176"
    },
    "helper_leaf_driver_176": {
      "name": "helper_leaf_driver_176",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-176",
      "source": "artifact://automation/helper_leaf_driver_176.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_176.so",
      "doc": "Helper leaf driver 176"
    },
    "helper_driver_177": {
      "name": "helper_driver_177",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-177",
      "source": "artifact://automation/helper_driver_177.so",
      "target": "/opt/configflux/drivers/helper_driver_177.so",
      "doc": "Helper cluster driver 177"
    },
    "helper_leaf_driver_177": {
      "name": "helper_leaf_driver_177",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-177",
      "source": "artifact://automation/helper_leaf_driver_177.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_177.so",
      "doc": "Helper leaf driver 177"
    },
    "helper_driver_178": {
      "name": "helper_driver_178",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-178",
      "source": "artifact://automation/helper_driver_178.so",
      "target": "/opt/configflux/drivers/helper_driver_178.so",
      "doc": "Helper cluster driver 178"
    },
    "helper_leaf_driver_178": {
      "name": "helper_leaf_driver_178",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-178",
      "source": "artifact://automation/helper_leaf_driver_178.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_178.so",
      "doc": "Helper leaf driver 178"
    },
    "helper_driver_179": {
      "name": "helper_driver_179",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-179",
      "source": "artifact://automation/helper_driver_179.so",
      "target": "/opt/configflux/drivers/helper_driver_179.so",
      "doc": "Helper cluster driver 179"
    },
    "helper_leaf_driver_179": {
      "name": "helper_leaf_driver_179",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-179",
      "source": "artifact://automation/helper_leaf_driver_179.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_179.so",
      "doc": "Helper leaf driver 179"
    },
    "helper_driver_180": {
      "name": "helper_driver_180",
      "version": "1.0.0",
      "hash": "sha256-helper-driver-180",
      "source": "artifact://automation/helper_driver_180.so",
      "target": "/opt/configflux/drivers/helper_driver_180.so",
      "doc": "Helper cluster driver 180"
    },
    "helper_leaf_driver_180": {
      "name": "helper_leaf_driver_180",
      "version": "1.0.0",
      "hash": "sha256-helper-leaf-driver-180",
      "source": "artifact://automation/helper_leaf_driver_180.so",
      "target": "/opt/configflux/drivers/helper_leaf_driver_180.so",
      "doc": "Helper leaf driver 180"
    }
  },
  "components": {
    "cell_root": {
      "type": "cell"
    },
    "swift_ring_standard": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'swiftmove' && vision_stack == 'opticore' && safety_mode == 'pl_d' && network_topology == 'ring'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "swift_ring_standard"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "swift_ring_driver"
        }
      }
    },
    "swift_ring_high_safety": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'swiftmove' && vision_stack == 'opticore' && safety_mode == 'pl_e' && network_topology == 'ring'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "swift_ring_high_safety"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "swift_ring_safety_driver"
        }
      }
    },
    "belt_star_standard": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'beltmax' && vision_stack == 'camplus' && safety_mode == 'pl_d' && network_topology == 'star'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "belt_star_standard"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "belt_star_driver"
        }
      }
    },
    "belt_star_high_safety": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'beltmax' && vision_stack == 'camplus' && safety_mode == 'pl_e' && network_topology == 'star'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "belt_star_high_safety"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "belt_star_safety_driver"
        }
      }
    },
    "large_cell_orchestrator": {
      "type": "controller",
      "condition": "conveyor_brand == 'swiftmove' && vision_stack == 'opticore' && safety_mode == 'pl_d' && network_topology == 'ring'",
      "depends_on": [
        "swift_ring_standard",
        "helper_cluster_001",
        "helper_cluster_002",
        "helper_cluster_003",
        "helper_cluster_004",
        "helper_cluster_005",
        "helper_cluster_006",
        "helper_cluster_007",
        "helper_cluster_008",
        "helper_cluster_009",
        "helper_cluster_010",
        "helper_cluster_011",
        "helper_cluster_012",
        "helper_cluster_013",
        "helper_cluster_014",
        "helper_cluster_015",
        "helper_cluster_016",
        "helper_cluster_017",
        "helper_cluster_018",
        "helper_cluster_019",
        "helper_cluster_020",
        "helper_cluster_021",
        "helper_cluster_022",
        "helper_cluster_023",
        "helper_cluster_024",
        "helper_cluster_025",
        "helper_cluster_026",
        "helper_cluster_027",
        "helper_cluster_028",
        "helper_cluster_029",
        "helper_cluster_030",
        "helper_cluster_031",
        "helper_cluster_032",
        "helper_cluster_033",
        "helper_cluster_034",
        "helper_cluster_035",
        "helper_cluster_036",
        "helper_cluster_037",
        "helper_cluster_038",
        "helper_cluster_039",
        "helper_cluster_040",
        "helper_cluster_041",
        "helper_cluster_042",
        "helper_cluster_043",
        "helper_cluster_044",
        "helper_cluster_045",
        "helper_cluster_046",
        "helper_cluster_047",
        "helper_cluster_048",
        "helper_cluster_049",
        "helper_cluster_050",
        "helper_cluster_051",
        "helper_cluster_052",
        "helper_cluster_053",
        "helper_cluster_054",
        "helper_cluster_055",
        "helper_cluster_056",
        "helper_cluster_057",
        "helper_cluster_058",
        "helper_cluster_059",
        "helper_cluster_060",
        "helper_cluster_061",
        "helper_cluster_062",
        "helper_cluster_063",
        "helper_cluster_064",
        "helper_cluster_065",
        "helper_cluster_066",
        "helper_cluster_067",
        "helper_cluster_068",
        "helper_cluster_069",
        "helper_cluster_070",
        "helper_cluster_071",
        "helper_cluster_072",
        "helper_cluster_073",
        "helper_cluster_074",
        "helper_cluster_075",
        "helper_cluster_076",
        "helper_cluster_077",
        "helper_cluster_078",
        "helper_cluster_079",
        "helper_cluster_080",
        "helper_cluster_081",
        "helper_cluster_082",
        "helper_cluster_083",
        "helper_cluster_084",
        "helper_cluster_085",
        "helper_cluster_086",
        "helper_cluster_087",
        "helper_cluster_088",
        "helper_cluster_089",
        "helper_cluster_090",
        "helper_cluster_091",
        "helper_cluster_092",
        "helper_cluster_093",
        "helper_cluster_094",
        "helper_cluster_095",
        "helper_cluster_096",
        "helper_cluster_097",
        "helper_cluster_098",
        "helper_cluster_099",
        "helper_cluster_100",
        "helper_cluster_101",
        "helper_cluster_102",
        "helper_cluster_103",
        "helper_cluster_104",
        "helper_cluster_105",
        "helper_cluster_106",
        "helper_cluster_107",
        "helper_cluster_108",
        "helper_cluster_109",
        "helper_cluster_110",
        "helper_cluster_111",
        "helper_cluster_112",
        "helper_cluster_113",
        "helper_cluster_114",
        "helper_cluster_115",
        "helper_cluster_116",
        "helper_cluster_117",
        "helper_cluster_118",
        "helper_cluster_119",
        "helper_cluster_120",
        "helper_cluster_121",
        "helper_cluster_122",
        "helper_cluster_123",
        "helper_cluster_124",
        "helper_cluster_125",
        "helper_cluster_126",
        "helper_cluster_127",
        "helper_cluster_128",
        "helper_cluster_129",
        "helper_cluster_130",
        "helper_cluster_131",
        "helper_cluster_132",
        "helper_cluster_133",
        "helper_cluster_134",
        "helper_cluster_135",
        "helper_cluster_136",
        "helper_cluster_137",
        "helper_cluster_138",
        "helper_cluster_139",
        "helper_cluster_140",
        "helper_cluster_141",
        "helper_cluster_142",
        "helper_cluster_143",
        "helper_cluster_144",
        "helper_cluster_145",
        "helper_cluster_146",
        "helper_cluster_147",
        "helper_cluster_148",
        "helper_cluster_149",
        "helper_cluster_150",
        "helper_cluster_151",
        "helper_cluster_152",
        "helper_cluster_153",
        "helper_cluster_154",
        "helper_cluster_155",
        "helper_cluster_156",
        "helper_cluster_157",
        "helper_cluster_158",
        "helper_cluster_159",
        "helper_cluster_160",
        "helper_cluster_161",
        "helper_cluster_162",
        "helper_cluster_163",
        "helper_cluster_164",
        "helper_cluster_165",
        "helper_cluster_166",
        "helper_cluster_167",
        "helper_cluster_168",
        "helper_cluster_169",
        "helper_cluster_170",
        "helper_cluster_171",
        "helper_cluster_172",
        "helper_cluster_173",
        "helper_cluster_174",
        "helper_cluster_175",
        "helper_cluster_176",
        "helper_cluster_177",
        "helper_cluster_178",
        "helper_cluster_179",
        "helper_cluster_180"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "large_cell_orchestrator"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "swift_ring_driver"
        }
      }
    },
    "helper_leaf_001": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_001"
        }
      }
    },
    "helper_cluster_001": {
      "type": "module",
      "depends_on": [
        "helper_leaf_001"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_001"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_001"
        }
      }
    },
    "helper_leaf_002": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_002"
        }
      }
    },
    "helper_cluster_002": {
      "type": "module",
      "depends_on": [
        "helper_leaf_002"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_002"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_002"
        }
      }
    },
    "helper_leaf_003": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_003"
        }
      }
    },
    "helper_cluster_003": {
      "type": "module",
      "depends_on": [
        "helper_leaf_003"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_003"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_003"
        }
      }
    },
    "helper_leaf_004": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_004"
        }
      }
    },
    "helper_cluster_004": {
      "type": "module",
      "depends_on": [
        "helper_leaf_004"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_004"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_004"
        }
      }
    },
    "helper_leaf_005": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_005"
        }
      }
    },
    "helper_cluster_005": {
      "type": "module",
      "depends_on": [
        "helper_leaf_005"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_005"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_005"
        }
      }
    },
    "helper_leaf_006": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_006"
        }
      }
    },
    "helper_cluster_006": {
      "type": "module",
      "depends_on": [
        "helper_leaf_006"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_006"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_006"
        }
      }
    },
    "helper_leaf_007": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_007"
        }
      }
    },
    "helper_cluster_007": {
      "type": "module",
      "depends_on": [
        "helper_leaf_007"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_007"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_007"
        }
      }
    },
    "helper_leaf_008": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_008"
        }
      }
    },
    "helper_cluster_008": {
      "type": "module",
      "depends_on": [
        "helper_leaf_008"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_008"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_008"
        }
      }
    },
    "helper_leaf_009": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_009"
        }
      }
    },
    "helper_cluster_009": {
      "type": "module",
      "depends_on": [
        "helper_leaf_009"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_009"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_009"
        }
      }
    },
    "helper_leaf_010": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_010"
        }
      }
    },
    "helper_cluster_010": {
      "type": "module",
      "depends_on": [
        "helper_leaf_010"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_010"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_010"
        }
      }
    },
    "helper_leaf_011": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_011"
        }
      }
    },
    "helper_cluster_011": {
      "type": "module",
      "depends_on": [
        "helper_leaf_011"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_011"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_011"
        }
      }
    },
    "helper_leaf_012": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_012"
        }
      }
    },
    "helper_cluster_012": {
      "type": "module",
      "depends_on": [
        "helper_leaf_012"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_012"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_012"
        }
      }
    },
    "helper_leaf_013": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_013"
        }
      }
    },
    "helper_cluster_013": {
      "type": "module",
      "depends_on": [
        "helper_leaf_013"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_013"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_013"
        }
      }
    },
    "helper_leaf_014": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_014"
        }
      }
    },
    "helper_cluster_014": {
      "type": "module",
      "depends_on": [
        "helper_leaf_014"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_014"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_014"
        }
      }
    },
    "helper_leaf_015": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_015"
        }
      }
    },
    "helper_cluster_015": {
      "type": "module",
      "depends_on": [
        "helper_leaf_015"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_015"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_015"
        }
      }
    },
    "helper_leaf_016": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_016"
        }
      }
    },
    "helper_cluster_016": {
      "type": "module",
      "depends_on": [
        "helper_leaf_016"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_016"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_016"
        }
      }
    },
    "helper_leaf_017": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_017"
        }
      }
    },
    "helper_cluster_017": {
      "type": "module",
      "depends_on": [
        "helper_leaf_017"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_017"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_017"
        }
      }
    },
    "helper_leaf_018": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_018"
        }
      }
    },
    "helper_cluster_018": {
      "type": "module",
      "depends_on": [
        "helper_leaf_018"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_018"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_018"
        }
      }
    },
    "helper_leaf_019": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_019"
        }
      }
    },
    "helper_cluster_019": {
      "type": "module",
      "depends_on": [
        "helper_leaf_019"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_019"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_019"
        }
      }
    },
    "helper_leaf_020": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_020"
        }
      }
    },
    "helper_cluster_020": {
      "type": "module",
      "depends_on": [
        "helper_leaf_020"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_020"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_020"
        }
      }
    },
    "helper_leaf_021": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_021"
        }
      }
    },
    "helper_cluster_021": {
      "type": "module",
      "depends_on": [
        "helper_leaf_021"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_021"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_021"
        }
      }
    },
    "helper_leaf_022": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_022"
        }
      }
    },
    "helper_cluster_022": {
      "type": "module",
      "depends_on": [
        "helper_leaf_022"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_022"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_022"
        }
      }
    },
    "helper_leaf_023": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_023"
        }
      }
    },
    "helper_cluster_023": {
      "type": "module",
      "depends_on": [
        "helper_leaf_023"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_023"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_023"
        }
      }
    },
    "helper_leaf_024": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_024"
        }
      }
    },
    "helper_cluster_024": {
      "type": "module",
      "depends_on": [
        "helper_leaf_024"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_024"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_024"
        }
      }
    },
    "helper_leaf_025": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_025"
        }
      }
    },
    "helper_cluster_025": {
      "type": "module",
      "depends_on": [
        "helper_leaf_025"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_025"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_025"
        }
      }
    },
    "helper_leaf_026": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_026"
        }
      }
    },
    "helper_cluster_026": {
      "type": "module",
      "depends_on": [
        "helper_leaf_026"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_026"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_026"
        }
      }
    },
    "helper_leaf_027": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_027"
        }
      }
    },
    "helper_cluster_027": {
      "type": "module",
      "depends_on": [
        "helper_leaf_027"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_027"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_027"
        }
      }
    },
    "helper_leaf_028": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_028"
        }
      }
    },
    "helper_cluster_028": {
      "type": "module",
      "depends_on": [
        "helper_leaf_028"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_028"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_028"
        }
      }
    },
    "helper_leaf_029": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_029"
        }
      }
    },
    "helper_cluster_029": {
      "type": "module",
      "depends_on": [
        "helper_leaf_029"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_029"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_029"
        }
      }
    },
    "helper_leaf_030": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_030"
        }
      }
    },
    "helper_cluster_030": {
      "type": "module",
      "depends_on": [
        "helper_leaf_030"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_030"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_030"
        }
      }
    },
    "helper_leaf_031": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_031"
        }
      }
    },
    "helper_cluster_031": {
      "type": "module",
      "depends_on": [
        "helper_leaf_031"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_031"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_031"
        }
      }
    },
    "helper_leaf_032": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_032"
        }
      }
    },
    "helper_cluster_032": {
      "type": "module",
      "depends_on": [
        "helper_leaf_032"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_032"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_032"
        }
      }
    },
    "helper_leaf_033": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_033"
        }
      }
    },
    "helper_cluster_033": {
      "type": "module",
      "depends_on": [
        "helper_leaf_033"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_033"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_033"
        }
      }
    },
    "helper_leaf_034": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_034"
        }
      }
    },
    "helper_cluster_034": {
      "type": "module",
      "depends_on": [
        "helper_leaf_034"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_034"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_034"
        }
      }
    },
    "helper_leaf_035": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_035"
        }
      }
    },
    "helper_cluster_035": {
      "type": "module",
      "depends_on": [
        "helper_leaf_035"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_035"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_035"
        }
      }
    },
    "helper_leaf_036": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_036"
        }
      }
    },
    "helper_cluster_036": {
      "type": "module",
      "depends_on": [
        "helper_leaf_036"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_036"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_036"
        }
      }
    },
    "helper_leaf_037": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_037"
        }
      }
    },
    "helper_cluster_037": {
      "type": "module",
      "depends_on": [
        "helper_leaf_037"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_037"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_037"
        }
      }
    },
    "helper_leaf_038": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_038"
        }
      }
    },
    "helper_cluster_038": {
      "type": "module",
      "depends_on": [
        "helper_leaf_038"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_038"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_038"
        }
      }
    },
    "helper_leaf_039": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_039"
        }
      }
    },
    "helper_cluster_039": {
      "type": "module",
      "depends_on": [
        "helper_leaf_039"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_039"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_039"
        }
      }
    },
    "helper_leaf_040": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_040"
        }
      }
    },
    "helper_cluster_040": {
      "type": "module",
      "depends_on": [
        "helper_leaf_040"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_040"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_040"
        }
      }
    },
    "helper_leaf_041": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_041"
        }
      }
    },
    "helper_cluster_041": {
      "type": "module",
      "depends_on": [
        "helper_leaf_041"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_041"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_041"
        }
      }
    },
    "helper_leaf_042": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_042"
        }
      }
    },
    "helper_cluster_042": {
      "type": "module",
      "depends_on": [
        "helper_leaf_042"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_042"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_042"
        }
      }
    },
    "helper_leaf_043": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_043"
        }
      }
    },
    "helper_cluster_043": {
      "type": "module",
      "depends_on": [
        "helper_leaf_043"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_043"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_043"
        }
      }
    },
    "helper_leaf_044": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_044"
        }
      }
    },
    "helper_cluster_044": {
      "type": "module",
      "depends_on": [
        "helper_leaf_044"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_044"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_044"
        }
      }
    },
    "helper_leaf_045": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_045"
        }
      }
    },
    "helper_cluster_045": {
      "type": "module",
      "depends_on": [
        "helper_leaf_045"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_045"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_045"
        }
      }
    },
    "helper_leaf_046": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_046"
        }
      }
    },
    "helper_cluster_046": {
      "type": "module",
      "depends_on": [
        "helper_leaf_046"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_046"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_046"
        }
      }
    },
    "helper_leaf_047": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_047"
        }
      }
    },
    "helper_cluster_047": {
      "type": "module",
      "depends_on": [
        "helper_leaf_047"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_047"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_047"
        }
      }
    },
    "helper_leaf_048": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_048"
        }
      }
    },
    "helper_cluster_048": {
      "type": "module",
      "depends_on": [
        "helper_leaf_048"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_048"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_048"
        }
      }
    },
    "helper_leaf_049": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_049"
        }
      }
    },
    "helper_cluster_049": {
      "type": "module",
      "depends_on": [
        "helper_leaf_049"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_049"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_049"
        }
      }
    },
    "helper_leaf_050": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_050"
        }
      }
    },
    "helper_cluster_050": {
      "type": "module",
      "depends_on": [
        "helper_leaf_050"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_050"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_050"
        }
      }
    },
    "helper_leaf_051": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_051"
        }
      }
    },
    "helper_cluster_051": {
      "type": "module",
      "depends_on": [
        "helper_leaf_051"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_051"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_051"
        }
      }
    },
    "helper_leaf_052": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_052"
        }
      }
    },
    "helper_cluster_052": {
      "type": "module",
      "depends_on": [
        "helper_leaf_052"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_052"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_052"
        }
      }
    },
    "helper_leaf_053": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_053"
        }
      }
    },
    "helper_cluster_053": {
      "type": "module",
      "depends_on": [
        "helper_leaf_053"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_053"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_053"
        }
      }
    },
    "helper_leaf_054": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_054"
        }
      }
    },
    "helper_cluster_054": {
      "type": "module",
      "depends_on": [
        "helper_leaf_054"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_054"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_054"
        }
      }
    },
    "helper_leaf_055": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_055"
        }
      }
    },
    "helper_cluster_055": {
      "type": "module",
      "depends_on": [
        "helper_leaf_055"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_055"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_055"
        }
      }
    },
    "helper_leaf_056": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_056"
        }
      }
    },
    "helper_cluster_056": {
      "type": "module",
      "depends_on": [
        "helper_leaf_056"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_056"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_056"
        }
      }
    },
    "helper_leaf_057": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_057"
        }
      }
    },
    "helper_cluster_057": {
      "type": "module",
      "depends_on": [
        "helper_leaf_057"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_057"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_057"
        }
      }
    },
    "helper_leaf_058": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_058"
        }
      }
    },
    "helper_cluster_058": {
      "type": "module",
      "depends_on": [
        "helper_leaf_058"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_058"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_058"
        }
      }
    },
    "helper_leaf_059": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_059"
        }
      }
    },
    "helper_cluster_059": {
      "type": "module",
      "depends_on": [
        "helper_leaf_059"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_059"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_059"
        }
      }
    },
    "helper_leaf_060": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_060"
        }
      }
    },
    "helper_cluster_060": {
      "type": "module",
      "depends_on": [
        "helper_leaf_060"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_060"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_060"
        }
      }
    },
    "helper_leaf_061": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_061"
        }
      }
    },
    "helper_cluster_061": {
      "type": "module",
      "depends_on": [
        "helper_leaf_061"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_061"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_061"
        }
      }
    },
    "helper_leaf_062": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_062"
        }
      }
    },
    "helper_cluster_062": {
      "type": "module",
      "depends_on": [
        "helper_leaf_062"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_062"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_062"
        }
      }
    },
    "helper_leaf_063": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_063"
        }
      }
    },
    "helper_cluster_063": {
      "type": "module",
      "depends_on": [
        "helper_leaf_063"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_063"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_063"
        }
      }
    },
    "helper_leaf_064": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_064"
        }
      }
    },
    "helper_cluster_064": {
      "type": "module",
      "depends_on": [
        "helper_leaf_064"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_064"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_064"
        }
      }
    },
    "helper_leaf_065": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_065"
        }
      }
    },
    "helper_cluster_065": {
      "type": "module",
      "depends_on": [
        "helper_leaf_065"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_065"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_065"
        }
      }
    },
    "helper_leaf_066": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_066"
        }
      }
    },
    "helper_cluster_066": {
      "type": "module",
      "depends_on": [
        "helper_leaf_066"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_066"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_066"
        }
      }
    },
    "helper_leaf_067": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_067"
        }
      }
    },
    "helper_cluster_067": {
      "type": "module",
      "depends_on": [
        "helper_leaf_067"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_067"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_067"
        }
      }
    },
    "helper_leaf_068": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_068"
        }
      }
    },
    "helper_cluster_068": {
      "type": "module",
      "depends_on": [
        "helper_leaf_068"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_068"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_068"
        }
      }
    },
    "helper_leaf_069": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_069"
        }
      }
    },
    "helper_cluster_069": {
      "type": "module",
      "depends_on": [
        "helper_leaf_069"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_069"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_069"
        }
      }
    },
    "helper_leaf_070": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_070"
        }
      }
    },
    "helper_cluster_070": {
      "type": "module",
      "depends_on": [
        "helper_leaf_070"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_070"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_070"
        }
      }
    },
    "helper_leaf_071": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_071"
        }
      }
    },
    "helper_cluster_071": {
      "type": "module",
      "depends_on": [
        "helper_leaf_071"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_071"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_071"
        }
      }
    },
    "helper_leaf_072": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_072"
        }
      }
    },
    "helper_cluster_072": {
      "type": "module",
      "depends_on": [
        "helper_leaf_072"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_072"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_072"
        }
      }
    },
    "helper_leaf_073": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_073"
        }
      }
    },
    "helper_cluster_073": {
      "type": "module",
      "depends_on": [
        "helper_leaf_073"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_073"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_073"
        }
      }
    },
    "helper_leaf_074": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_074"
        }
      }
    },
    "helper_cluster_074": {
      "type": "module",
      "depends_on": [
        "helper_leaf_074"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_074"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_074"
        }
      }
    },
    "helper_leaf_075": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_075"
        }
      }
    },
    "helper_cluster_075": {
      "type": "module",
      "depends_on": [
        "helper_leaf_075"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_075"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_075"
        }
      }
    },
    "helper_leaf_076": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_076"
        }
      }
    },
    "helper_cluster_076": {
      "type": "module",
      "depends_on": [
        "helper_leaf_076"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_076"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_076"
        }
      }
    },
    "helper_leaf_077": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_077"
        }
      }
    },
    "helper_cluster_077": {
      "type": "module",
      "depends_on": [
        "helper_leaf_077"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_077"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_077"
        }
      }
    },
    "helper_leaf_078": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_078"
        }
      }
    },
    "helper_cluster_078": {
      "type": "module",
      "depends_on": [
        "helper_leaf_078"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_078"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_078"
        }
      }
    },
    "helper_leaf_079": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_079"
        }
      }
    },
    "helper_cluster_079": {
      "type": "module",
      "depends_on": [
        "helper_leaf_079"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_079"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_079"
        }
      }
    },
    "helper_leaf_080": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_080"
        }
      }
    },
    "helper_cluster_080": {
      "type": "module",
      "depends_on": [
        "helper_leaf_080"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_080"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_080"
        }
      }
    },
    "helper_leaf_081": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_081"
        }
      }
    },
    "helper_cluster_081": {
      "type": "module",
      "depends_on": [
        "helper_leaf_081"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_081"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_081"
        }
      }
    },
    "helper_leaf_082": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_082"
        }
      }
    },
    "helper_cluster_082": {
      "type": "module",
      "depends_on": [
        "helper_leaf_082"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_082"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_082"
        }
      }
    },
    "helper_leaf_083": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_083"
        }
      }
    },
    "helper_cluster_083": {
      "type": "module",
      "depends_on": [
        "helper_leaf_083"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_083"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_083"
        }
      }
    },
    "helper_leaf_084": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_084"
        }
      }
    },
    "helper_cluster_084": {
      "type": "module",
      "depends_on": [
        "helper_leaf_084"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_084"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_084"
        }
      }
    },
    "helper_leaf_085": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_085"
        }
      }
    },
    "helper_cluster_085": {
      "type": "module",
      "depends_on": [
        "helper_leaf_085"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_085"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_085"
        }
      }
    },
    "helper_leaf_086": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_086"
        }
      }
    },
    "helper_cluster_086": {
      "type": "module",
      "depends_on": [
        "helper_leaf_086"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_086"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_086"
        }
      }
    },
    "helper_leaf_087": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_087"
        }
      }
    },
    "helper_cluster_087": {
      "type": "module",
      "depends_on": [
        "helper_leaf_087"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_087"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_087"
        }
      }
    },
    "helper_leaf_088": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_088"
        }
      }
    },
    "helper_cluster_088": {
      "type": "module",
      "depends_on": [
        "helper_leaf_088"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_088"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_088"
        }
      }
    },
    "helper_leaf_089": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_089"
        }
      }
    },
    "helper_cluster_089": {
      "type": "module",
      "depends_on": [
        "helper_leaf_089"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_089"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_089"
        }
      }
    },
    "helper_leaf_090": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_090"
        }
      }
    },
    "helper_cluster_090": {
      "type": "module",
      "depends_on": [
        "helper_leaf_090"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_090"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_090"
        }
      }
    },
    "helper_leaf_091": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_091"
        }
      }
    },
    "helper_cluster_091": {
      "type": "module",
      "depends_on": [
        "helper_leaf_091"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_091"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_091"
        }
      }
    },
    "helper_leaf_092": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_092"
        }
      }
    },
    "helper_cluster_092": {
      "type": "module",
      "depends_on": [
        "helper_leaf_092"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_092"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_092"
        }
      }
    },
    "helper_leaf_093": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_093"
        }
      }
    },
    "helper_cluster_093": {
      "type": "module",
      "depends_on": [
        "helper_leaf_093"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_093"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_093"
        }
      }
    },
    "helper_leaf_094": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_094"
        }
      }
    },
    "helper_cluster_094": {
      "type": "module",
      "depends_on": [
        "helper_leaf_094"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_094"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_094"
        }
      }
    },
    "helper_leaf_095": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_095"
        }
      }
    },
    "helper_cluster_095": {
      "type": "module",
      "depends_on": [
        "helper_leaf_095"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_095"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_095"
        }
      }
    },
    "helper_leaf_096": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_096"
        }
      }
    },
    "helper_cluster_096": {
      "type": "module",
      "depends_on": [
        "helper_leaf_096"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_096"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_096"
        }
      }
    },
    "helper_leaf_097": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_097"
        }
      }
    },
    "helper_cluster_097": {
      "type": "module",
      "depends_on": [
        "helper_leaf_097"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_097"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_097"
        }
      }
    },
    "helper_leaf_098": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_098"
        }
      }
    },
    "helper_cluster_098": {
      "type": "module",
      "depends_on": [
        "helper_leaf_098"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_098"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_098"
        }
      }
    },
    "helper_leaf_099": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_099"
        }
      }
    },
    "helper_cluster_099": {
      "type": "module",
      "depends_on": [
        "helper_leaf_099"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_099"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_099"
        }
      }
    },
    "helper_leaf_100": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_100"
        }
      }
    },
    "helper_cluster_100": {
      "type": "module",
      "depends_on": [
        "helper_leaf_100"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_100"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_100"
        }
      }
    },
    "helper_leaf_101": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_101"
        }
      }
    },
    "helper_cluster_101": {
      "type": "module",
      "depends_on": [
        "helper_leaf_101"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_101"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_101"
        }
      }
    },
    "helper_leaf_102": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_102"
        }
      }
    },
    "helper_cluster_102": {
      "type": "module",
      "depends_on": [
        "helper_leaf_102"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_102"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_102"
        }
      }
    },
    "helper_leaf_103": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_103"
        }
      }
    },
    "helper_cluster_103": {
      "type": "module",
      "depends_on": [
        "helper_leaf_103"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_103"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_103"
        }
      }
    },
    "helper_leaf_104": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_104"
        }
      }
    },
    "helper_cluster_104": {
      "type": "module",
      "depends_on": [
        "helper_leaf_104"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_104"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_104"
        }
      }
    },
    "helper_leaf_105": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_105"
        }
      }
    },
    "helper_cluster_105": {
      "type": "module",
      "depends_on": [
        "helper_leaf_105"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_105"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_105"
        }
      }
    },
    "helper_leaf_106": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_106"
        }
      }
    },
    "helper_cluster_106": {
      "type": "module",
      "depends_on": [
        "helper_leaf_106"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_106"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_106"
        }
      }
    },
    "helper_leaf_107": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_107"
        }
      }
    },
    "helper_cluster_107": {
      "type": "module",
      "depends_on": [
        "helper_leaf_107"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_107"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_107"
        }
      }
    },
    "helper_leaf_108": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_108"
        }
      }
    },
    "helper_cluster_108": {
      "type": "module",
      "depends_on": [
        "helper_leaf_108"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_108"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_108"
        }
      }
    },
    "helper_leaf_109": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_109"
        }
      }
    },
    "helper_cluster_109": {
      "type": "module",
      "depends_on": [
        "helper_leaf_109"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_109"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_109"
        }
      }
    },
    "helper_leaf_110": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_110"
        }
      }
    },
    "helper_cluster_110": {
      "type": "module",
      "depends_on": [
        "helper_leaf_110"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_110"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_110"
        }
      }
    },
    "helper_leaf_111": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_111"
        }
      }
    },
    "helper_cluster_111": {
      "type": "module",
      "depends_on": [
        "helper_leaf_111"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_111"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_111"
        }
      }
    },
    "helper_leaf_112": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_112"
        }
      }
    },
    "helper_cluster_112": {
      "type": "module",
      "depends_on": [
        "helper_leaf_112"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_112"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_112"
        }
      }
    },
    "helper_leaf_113": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_113"
        }
      }
    },
    "helper_cluster_113": {
      "type": "module",
      "depends_on": [
        "helper_leaf_113"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_113"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_113"
        }
      }
    },
    "helper_leaf_114": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_114"
        }
      }
    },
    "helper_cluster_114": {
      "type": "module",
      "depends_on": [
        "helper_leaf_114"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_114"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_114"
        }
      }
    },
    "helper_leaf_115": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_115"
        }
      }
    },
    "helper_cluster_115": {
      "type": "module",
      "depends_on": [
        "helper_leaf_115"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_115"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_115"
        }
      }
    },
    "helper_leaf_116": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_116"
        }
      }
    },
    "helper_cluster_116": {
      "type": "module",
      "depends_on": [
        "helper_leaf_116"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_116"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_116"
        }
      }
    },
    "helper_leaf_117": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_117"
        }
      }
    },
    "helper_cluster_117": {
      "type": "module",
      "depends_on": [
        "helper_leaf_117"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_117"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_117"
        }
      }
    },
    "helper_leaf_118": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_118"
        }
      }
    },
    "helper_cluster_118": {
      "type": "module",
      "depends_on": [
        "helper_leaf_118"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_118"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_118"
        }
      }
    },
    "helper_leaf_119": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_119"
        }
      }
    },
    "helper_cluster_119": {
      "type": "module",
      "depends_on": [
        "helper_leaf_119"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_119"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_119"
        }
      }
    },
    "helper_leaf_120": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_120"
        }
      }
    },
    "helper_cluster_120": {
      "type": "module",
      "depends_on": [
        "helper_leaf_120"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_120"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_120"
        }
      }
    },
    "helper_leaf_121": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_121"
        }
      }
    },
    "helper_cluster_121": {
      "type": "module",
      "depends_on": [
        "helper_leaf_121"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_121"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_121"
        }
      }
    },
    "helper_leaf_122": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_122"
        }
      }
    },
    "helper_cluster_122": {
      "type": "module",
      "depends_on": [
        "helper_leaf_122"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_122"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_122"
        }
      }
    },
    "helper_leaf_123": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_123"
        }
      }
    },
    "helper_cluster_123": {
      "type": "module",
      "depends_on": [
        "helper_leaf_123"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_123"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_123"
        }
      }
    },
    "helper_leaf_124": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_124"
        }
      }
    },
    "helper_cluster_124": {
      "type": "module",
      "depends_on": [
        "helper_leaf_124"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_124"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_124"
        }
      }
    },
    "helper_leaf_125": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_125"
        }
      }
    },
    "helper_cluster_125": {
      "type": "module",
      "depends_on": [
        "helper_leaf_125"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_125"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_125"
        }
      }
    },
    "helper_leaf_126": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_126"
        }
      }
    },
    "helper_cluster_126": {
      "type": "module",
      "depends_on": [
        "helper_leaf_126"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_126"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_126"
        }
      }
    },
    "helper_leaf_127": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_127"
        }
      }
    },
    "helper_cluster_127": {
      "type": "module",
      "depends_on": [
        "helper_leaf_127"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_127"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_127"
        }
      }
    },
    "helper_leaf_128": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_128"
        }
      }
    },
    "helper_cluster_128": {
      "type": "module",
      "depends_on": [
        "helper_leaf_128"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_128"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_128"
        }
      }
    },
    "helper_leaf_129": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_129"
        }
      }
    },
    "helper_cluster_129": {
      "type": "module",
      "depends_on": [
        "helper_leaf_129"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_129"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_129"
        }
      }
    },
    "helper_leaf_130": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_130"
        }
      }
    },
    "helper_cluster_130": {
      "type": "module",
      "depends_on": [
        "helper_leaf_130"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_130"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_130"
        }
      }
    },
    "helper_leaf_131": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_131"
        }
      }
    },
    "helper_cluster_131": {
      "type": "module",
      "depends_on": [
        "helper_leaf_131"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_131"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_131"
        }
      }
    },
    "helper_leaf_132": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_132"
        }
      }
    },
    "helper_cluster_132": {
      "type": "module",
      "depends_on": [
        "helper_leaf_132"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_132"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_132"
        }
      }
    },
    "helper_leaf_133": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_133"
        }
      }
    },
    "helper_cluster_133": {
      "type": "module",
      "depends_on": [
        "helper_leaf_133"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_133"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_133"
        }
      }
    },
    "helper_leaf_134": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_134"
        }
      }
    },
    "helper_cluster_134": {
      "type": "module",
      "depends_on": [
        "helper_leaf_134"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_134"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_134"
        }
      }
    },
    "helper_leaf_135": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_135"
        }
      }
    },
    "helper_cluster_135": {
      "type": "module",
      "depends_on": [
        "helper_leaf_135"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_135"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_135"
        }
      }
    },
    "helper_leaf_136": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_136"
        }
      }
    },
    "helper_cluster_136": {
      "type": "module",
      "depends_on": [
        "helper_leaf_136"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_136"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_136"
        }
      }
    },
    "helper_leaf_137": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_137"
        }
      }
    },
    "helper_cluster_137": {
      "type": "module",
      "depends_on": [
        "helper_leaf_137"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_137"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_137"
        }
      }
    },
    "helper_leaf_138": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_138"
        }
      }
    },
    "helper_cluster_138": {
      "type": "module",
      "depends_on": [
        "helper_leaf_138"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_138"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_138"
        }
      }
    },
    "helper_leaf_139": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_139"
        }
      }
    },
    "helper_cluster_139": {
      "type": "module",
      "depends_on": [
        "helper_leaf_139"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_139"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_139"
        }
      }
    },
    "helper_leaf_140": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_140"
        }
      }
    },
    "helper_cluster_140": {
      "type": "module",
      "depends_on": [
        "helper_leaf_140"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_140"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_140"
        }
      }
    },
    "helper_leaf_141": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_141"
        }
      }
    },
    "helper_cluster_141": {
      "type": "module",
      "depends_on": [
        "helper_leaf_141"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_141"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_141"
        }
      }
    },
    "helper_leaf_142": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_142"
        }
      }
    },
    "helper_cluster_142": {
      "type": "module",
      "depends_on": [
        "helper_leaf_142"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_142"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_142"
        }
      }
    },
    "helper_leaf_143": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_143"
        }
      }
    },
    "helper_cluster_143": {
      "type": "module",
      "depends_on": [
        "helper_leaf_143"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_143"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_143"
        }
      }
    },
    "helper_leaf_144": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_144"
        }
      }
    },
    "helper_cluster_144": {
      "type": "module",
      "depends_on": [
        "helper_leaf_144"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_144"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_144"
        }
      }
    },
    "helper_leaf_145": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_145"
        }
      }
    },
    "helper_cluster_145": {
      "type": "module",
      "depends_on": [
        "helper_leaf_145"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_145"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_145"
        }
      }
    },
    "helper_leaf_146": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_146"
        }
      }
    },
    "helper_cluster_146": {
      "type": "module",
      "depends_on": [
        "helper_leaf_146"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_146"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_146"
        }
      }
    },
    "helper_leaf_147": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_147"
        }
      }
    },
    "helper_cluster_147": {
      "type": "module",
      "depends_on": [
        "helper_leaf_147"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_147"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_147"
        }
      }
    },
    "helper_leaf_148": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_148"
        }
      }
    },
    "helper_cluster_148": {
      "type": "module",
      "depends_on": [
        "helper_leaf_148"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_148"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_148"
        }
      }
    },
    "helper_leaf_149": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_149"
        }
      }
    },
    "helper_cluster_149": {
      "type": "module",
      "depends_on": [
        "helper_leaf_149"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_149"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_149"
        }
      }
    },
    "helper_leaf_150": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_150"
        }
      }
    },
    "helper_cluster_150": {
      "type": "module",
      "depends_on": [
        "helper_leaf_150"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_150"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_150"
        }
      }
    },
    "helper_leaf_151": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_151"
        }
      }
    },
    "helper_cluster_151": {
      "type": "module",
      "depends_on": [
        "helper_leaf_151"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_151"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_151"
        }
      }
    },
    "helper_leaf_152": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_152"
        }
      }
    },
    "helper_cluster_152": {
      "type": "module",
      "depends_on": [
        "helper_leaf_152"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_152"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_152"
        }
      }
    },
    "helper_leaf_153": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_153"
        }
      }
    },
    "helper_cluster_153": {
      "type": "module",
      "depends_on": [
        "helper_leaf_153"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_153"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_153"
        }
      }
    },
    "helper_leaf_154": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_154"
        }
      }
    },
    "helper_cluster_154": {
      "type": "module",
      "depends_on": [
        "helper_leaf_154"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_154"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_154"
        }
      }
    },
    "helper_leaf_155": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_155"
        }
      }
    },
    "helper_cluster_155": {
      "type": "module",
      "depends_on": [
        "helper_leaf_155"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_155"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_155"
        }
      }
    },
    "helper_leaf_156": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_156"
        }
      }
    },
    "helper_cluster_156": {
      "type": "module",
      "depends_on": [
        "helper_leaf_156"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_156"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_156"
        }
      }
    },
    "helper_leaf_157": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_157"
        }
      }
    },
    "helper_cluster_157": {
      "type": "module",
      "depends_on": [
        "helper_leaf_157"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_157"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_157"
        }
      }
    },
    "helper_leaf_158": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_158"
        }
      }
    },
    "helper_cluster_158": {
      "type": "module",
      "depends_on": [
        "helper_leaf_158"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_158"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_158"
        }
      }
    },
    "helper_leaf_159": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_159"
        }
      }
    },
    "helper_cluster_159": {
      "type": "module",
      "depends_on": [
        "helper_leaf_159"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_159"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_159"
        }
      }
    },
    "helper_leaf_160": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_160"
        }
      }
    },
    "helper_cluster_160": {
      "type": "module",
      "depends_on": [
        "helper_leaf_160"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_160"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_160"
        }
      }
    },
    "helper_leaf_161": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_161"
        }
      }
    },
    "helper_cluster_161": {
      "type": "module",
      "depends_on": [
        "helper_leaf_161"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_161"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_161"
        }
      }
    },
    "helper_leaf_162": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_162"
        }
      }
    },
    "helper_cluster_162": {
      "type": "module",
      "depends_on": [
        "helper_leaf_162"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_162"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_162"
        }
      }
    },
    "helper_leaf_163": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_163"
        }
      }
    },
    "helper_cluster_163": {
      "type": "module",
      "depends_on": [
        "helper_leaf_163"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_163"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_163"
        }
      }
    },
    "helper_leaf_164": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_164"
        }
      }
    },
    "helper_cluster_164": {
      "type": "module",
      "depends_on": [
        "helper_leaf_164"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_164"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_164"
        }
      }
    },
    "helper_leaf_165": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_165"
        }
      }
    },
    "helper_cluster_165": {
      "type": "module",
      "depends_on": [
        "helper_leaf_165"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_165"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_165"
        }
      }
    },
    "helper_leaf_166": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_166"
        }
      }
    },
    "helper_cluster_166": {
      "type": "module",
      "depends_on": [
        "helper_leaf_166"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_166"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_166"
        }
      }
    },
    "helper_leaf_167": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_167"
        }
      }
    },
    "helper_cluster_167": {
      "type": "module",
      "depends_on": [
        "helper_leaf_167"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_167"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_167"
        }
      }
    },
    "helper_leaf_168": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_168"
        }
      }
    },
    "helper_cluster_168": {
      "type": "module",
      "depends_on": [
        "helper_leaf_168"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_168"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_168"
        }
      }
    },
    "helper_leaf_169": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_169"
        }
      }
    },
    "helper_cluster_169": {
      "type": "module",
      "depends_on": [
        "helper_leaf_169"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_169"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_169"
        }
      }
    },
    "helper_leaf_170": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_170"
        }
      }
    },
    "helper_cluster_170": {
      "type": "module",
      "depends_on": [
        "helper_leaf_170"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_170"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_170"
        }
      }
    },
    "helper_leaf_171": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_171"
        }
      }
    },
    "helper_cluster_171": {
      "type": "module",
      "depends_on": [
        "helper_leaf_171"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_171"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_171"
        }
      }
    },
    "helper_leaf_172": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_172"
        }
      }
    },
    "helper_cluster_172": {
      "type": "module",
      "depends_on": [
        "helper_leaf_172"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_172"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_172"
        }
      }
    },
    "helper_leaf_173": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_173"
        }
      }
    },
    "helper_cluster_173": {
      "type": "module",
      "depends_on": [
        "helper_leaf_173"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_173"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_173"
        }
      }
    },
    "helper_leaf_174": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_174"
        }
      }
    },
    "helper_cluster_174": {
      "type": "module",
      "depends_on": [
        "helper_leaf_174"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_174"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_174"
        }
      }
    },
    "helper_leaf_175": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_175"
        }
      }
    },
    "helper_cluster_175": {
      "type": "module",
      "depends_on": [
        "helper_leaf_175"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_175"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_175"
        }
      }
    },
    "helper_leaf_176": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_176"
        }
      }
    },
    "helper_cluster_176": {
      "type": "module",
      "depends_on": [
        "helper_leaf_176"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_176"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_176"
        }
      }
    },
    "helper_leaf_177": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_177"
        }
      }
    },
    "helper_cluster_177": {
      "type": "module",
      "depends_on": [
        "helper_leaf_177"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_177"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_177"
        }
      }
    },
    "helper_leaf_178": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_178"
        }
      }
    },
    "helper_cluster_178": {
      "type": "module",
      "depends_on": [
        "helper_leaf_178"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_178"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_178"
        }
      }
    },
    "helper_leaf_179": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_179"
        }
      }
    },
    "helper_cluster_179": {
      "type": "module",
      "depends_on": [
        "helper_leaf_179"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_179"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_179"
        }
      }
    },
    "helper_leaf_180": {
      "type": "module",
      "params": {
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_leaf_driver_180"
        }
      }
    },
    "helper_cluster_180": {
      "type": "module",
      "depends_on": [
        "helper_leaf_180"
      ],
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "helper_cluster_180"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "helper_driver_180"
        }
      }
    }
  }
}
