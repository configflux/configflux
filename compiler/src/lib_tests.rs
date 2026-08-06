// SPDX-License-Identifier: BUSL-1.1

    use super::*;
    use crate::compiler_core::SourceChunk;
    use crate::ingest_merge::build_ir_index;
    use crate::resolver::{resolve, ResolutionContext};
    use crate::scenario_test_support::unique_temp_path;
    use crate::schema::{Component, Config, Parameter};
    use std::collections::HashMap;

    // --- Helper to create a context from a list of strings ---
    fn make_ctx(pairs: &[(&str, &str)]) -> ResolutionContext {
        let mut tags = HashMap::new();
        for (k, v) in pairs {
            tags.insert(k.to_string(), v.to_string());
        }
        ResolutionContext { tags }
    }

    fn empty_param() -> Parameter {
        Parameter {
            inherits: None,
            r#type: None,
            unit: None,
            doc: None,
            value: None,
            lifecycle: None,
            safety: None,
            access: None,
            limits: None,
            req_id: None,
            overrides: Vec::new(),
        }
    }

    fn config_with_component(name: &str) -> Config {
        let mut components = HashMap::new();
        components.insert(
            name.to_string(),
            Component {
                r#type: Some("actuator".to_string()),
                condition: None,
                depends_on: Vec::new(),
                params: HashMap::new(),
            },
        );

        Config {
            package: "p1".to_string(),
            version: "1.0".to_string(),
            definitions: HashMap::new(),
            components,
            artifacts: HashMap::new(),
            facets: Default::default(),
            constraints: Default::default(),
        }
    }

    fn config_with_definition(name: &str) -> Config {
        let mut definitions = HashMap::new();
        let mut param = empty_param();
        param.r#type = Some("float".to_string());
        definitions.insert(name.to_string(), param);

        Config {
            package: "p1".to_string(),
            version: "1.0".to_string(),
            definitions,
            components: HashMap::new(),
            artifacts: HashMap::new(),
            facets: Default::default(),
            constraints: Default::default(),
        }
    }

    // NOTE (B-5, ADR-0027 Track B, configflux-73fr): the Rust inheritance/merge
    // engine (`apply_inheritance`, the `merge_params`/`merge_components`
    // cross-file conflict logic, `validate_snake_case_ids`, and the
    // target-existence half of `validate_definition_inheritance`) was deleted
    // once CUE became the semantic owner of `inherits` resolution and cross-file
    // unification (proven byte-equal by the B-4 equivalence corpus). The unit
    // tests that exercised that engine via inline TOML overlays were removed
    // with it, along with the `Compiler::add_chunk` inline convenience wrapper
    // whose only callers they were. The tests that remain here cover the code
    // that stays Rust-owned: the resolve-time override/late-binding engine, the
    // component-dependency and definition-cycle detectors, `build_ir_index`'s
    // one-entity-one-chunk invariant, and IR emission. Surviving link-verify
    // tests feed their TOML through `add_chunk_auto` (the retained content
    // router), the same entry point the scenario mutation fixtures use.

    #[test]
    fn test_merge_duplicate_definition_id() {
        let mut compiler = Compiler::new();

        let chunk_a = r#"
            package = "p1"
            version = "1.0"

            [definitions.safe_speed]
            type = "float"
            unit = "m/s"
        "#;

        let chunk_b = r#"
            package = "p2"
            version = "1.0"

            [definitions.safe_speed]
            type = "float"
            unit = "km/h"
        "#;

        compiler.add_chunk_auto("a.toml", chunk_a).unwrap();
        let err = compiler.add_chunk_auto("b.toml", chunk_b).unwrap_err();
        assert!(
            format!("{err}").contains("Duplicate definition ID"),
            "err: {err}"
        );
    }

    #[test]
    fn test_merge_duplicate_component_id() {
        // Cross-file component overlap is now a duplicate error (ADR-0027
        // Track B): CUE owns cross-file unification, so two chunks must not both
        // carry the same component. This replaces the former
        // `merge_components`-based overlay-merge tests.
        let mut compiler = Compiler::new();

        let chunk_a = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
        "#;

        let chunk_b = r#"
            package = "p2"
            version = "1.0"

            [components.motor]
            type = "sensor"
        "#;

        compiler.add_chunk_auto("a.toml", chunk_a).unwrap();
        let err = compiler.add_chunk_auto("b.toml", chunk_b).unwrap_err();
        assert!(
            format!("{err}").contains("Duplicate component ID"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_unknown_dependency() {
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
            depends_on = ["missing_component"]
        "#;

        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("depends_on unknown component"),
            "err: {err}"
        );
    }

    #[test]
    fn test_duplicate_facet_across_chunks_is_rejected_at_ingest() {
        // ADR-0047 §2: a facet is a pack-global domain declared by at most one
        // chunk; the second declaration fails at ingest merge.
        let mut compiler = Compiler::new();
        let chunk_a = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "eu"
        "#;
        let chunk_b = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "apac"]
        "#;
        compiler.add_chunk_auto("00_defs.toml", chunk_a).unwrap();
        let err = compiler.add_chunk_auto("01_more.toml", chunk_b).unwrap_err();
        assert!(
            format!("{err}").contains("Facet 'region' is declared in more than one chunk"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_rejects_facet_default_not_in_values() {
        let mut compiler = Compiler::new();
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "mars"
        "#;
        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("default 'mars' is not one of"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_accepts_declared_facet_with_matching_condition() {
        let mut compiler = Compiler::new();
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "eu"

            [components.motor]
            type = "actuator"
            condition = "region == 'us'"
        "#;
        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        assert!(compiler.link_and_verify().is_ok());
    }

    #[test]
    fn test_link_and_verify_cycle() {
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.alpha]
            type = "actuator"
            depends_on = ["beta"]

            [components.beta]
            type = "actuator"
            depends_on = ["alpha"]
        "#;

        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("dependency cycle detected"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_diamond_dependency_is_accepted() {
        // ADR-0048: diamonds (here `shared` reached from `root` via both `left`
        // and `right`) are a permitted DAG shape. Link/verify accepts them;
        // acyclicity is still enforced by the cycle check.
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.root]
            type = "actuator"
            depends_on = ["left", "right"]

            [components.left]
            type = "actuator"
            depends_on = ["shared"]

            [components.right]
            type = "actuator"
            depends_on = ["shared"]

            [components.shared]
            type = "actuator"
        "#;

        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        assert!(
            compiler.link_and_verify().is_ok(),
            "a diamond dependency must be accepted after ADR-0048"
        );
    }

    #[test]
    fn test_link_and_verify_definition_inherits_unknown() {
        // Unknown inherit target on the definition `inherits` chain is still
        // caught by the retained `detect_definition_cycle` (ADR-0027 Decision 6
        // carve-out), which rejects an unknown parent while walking the chain.
        // Authored via the CUE-JSON ingest path: `inherits` pointers survive
        // verbatim in the emitted CUE-JSON, whereas authoring `inherits` in TOML
        // is rejected at ingest (configflux-qofj).
        let mut compiler = Compiler::new();

        let chunk = r#"{
            "package": "p1",
            "version": "1.0",
            "definitions": {
                "alpha": { "type": "float", "inherits": "missing_def" }
            }
        }"#;

        compiler.add_chunk_auto("chunk.json", chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("inherits unknown definition"),
            "err: {err}"
        );
    }

    #[test]
    fn test_add_chunk_with_source_duplicate_id() {
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
        "#;

        compiler
            .add_chunk_with_source("configs/motor.toml", chunk)
            .unwrap();
        let err = compiler
            .add_chunk_with_source("configs/motor.toml", chunk)
            .unwrap_err();
        assert!(
            format!("{err}").contains("Duplicate source_id"),
            "err: {err}"
        );
    }

    #[test]
    fn test_build_ir_index_duplicate_component() {
        let chunks = vec![
            SourceChunk {
                source_id: "a.toml".to_string(),
                chunk_hash: "hash_a".to_string(),
                config: config_with_component("motor"),
            },
            SourceChunk {
                source_id: "b.toml".to_string(),
                chunk_hash: "hash_b".to_string(),
                config: config_with_component("motor"),
            },
        ];

        let err = build_ir_index(&chunks).unwrap_err();
        assert!(
            format!("{err}").contains("Component 'motor' appears in multiple chunks"),
            "err: {err}"
        );
    }

    #[test]
    fn test_build_ir_index_duplicate_definition() {
        let chunks = vec![
            SourceChunk {
                source_id: "a.toml".to_string(),
                chunk_hash: "hash_a".to_string(),
                config: config_with_definition("speed"),
            },
            SourceChunk {
                source_id: "b.toml".to_string(),
                chunk_hash: "hash_b".to_string(),
                config: config_with_definition("speed"),
            },
        ];

        let err = build_ir_index(&chunks).unwrap_err();
        assert!(
            format!("{err}").contains("Definition 'speed' appears in multiple chunks"),
            "err: {err}"
        );
    }

    #[test]
    fn test_build_ir_index_duplicate_artifact() {
        let mut config_a = config_with_component("motor");
        config_a.artifacts.insert(
            "motor_driver".to_string(),
            schema::Artifact {
                name: "motor_driver".to_string(),
                version: None,
                hash: None,
                source: Some("artifact://motor_driver".to_string()),
                target: None,
                doc: None,
            },
        );

        let mut config_b = config_with_component("sensor");
        config_b.artifacts.insert(
            "motor_driver".to_string(),
            schema::Artifact {
                name: "motor_driver".to_string(),
                version: None,
                hash: None,
                source: Some("artifact://motor_driver".to_string()),
                target: None,
                doc: None,
            },
        );

        let chunks = vec![
            SourceChunk {
                source_id: "a.toml".to_string(),
                chunk_hash: "hash_a".to_string(),
                config: config_a,
            },
            SourceChunk {
                source_id: "b.toml".to_string(),
                chunk_hash: "hash_b".to_string(),
                config: config_b,
            },
        ];

        let err = build_ir_index(&chunks).unwrap_err();
        assert!(
            format!("{err}").contains("Artifact 'motor_driver' appears in multiple chunks"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_definition_inheritance_cycle() {
        // Cycle detection over the `inherits` string-pointer graph is retained in
        // Rust (ADR-0027 Decision 6 carve-out). Authored via CUE-JSON because
        // authoring `inherits` in TOML is rejected at ingest (configflux-qofj).
        let mut compiler = Compiler::new();

        let chunk = r#"{
            "package": "p1",
            "version": "1.0",
            "definitions": {
                "alpha": { "type": "float", "inherits": "beta" },
                "beta": { "type": "float", "inherits": "alpha" }
            }
        }"#;

        compiler.add_chunk_auto("chunk.json", chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("Definition inheritance cycle detected"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_definition_inheritance_depth_ceiling() {
        // configflux-xowl.5: a deeply nested ACYCLIC `inherits` chain must fail
        // closed with a diagnostic instead of overflowing the stack. 1_100
        // exceeds MAX_CHAIN_DEPTH (1_000) but stays far below any depth that
        // could overflow before the guard fires.
        let mut compiler = Compiler::new();
        let mut defs: Vec<String> = Vec::new();
        for i in 0..1_100 {
            if i + 1 < 1_100 {
                defs.push(format!(
                    "\"d{i:04}\": {{ \"type\": \"float\", \"inherits\": \"d{:04}\" }}",
                    i + 1
                ));
            } else {
                defs.push(format!("\"d{i:04}\": {{ \"type\": \"float\" }}"));
            }
        }
        let chunk = format!(
            "{{ \"package\": \"p1\", \"version\": \"1.0\", \"definitions\": {{ {} }} }}",
            defs.join(", ")
        );

        compiler.add_chunk_auto("chunk.json", &chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("exceeds the maximum supported depth"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_definition_inheritance_chain_at_limit_is_accepted() {
        // Boundary proof for the configflux-xowl.5 ceiling: a chain of exactly
        // MAX_CHAIN_DEPTH (1_000) definitions is legitimate and must link.
        let mut compiler = Compiler::new();
        let mut defs: Vec<String> = Vec::new();
        for i in 0..1_000 {
            if i + 1 < 1_000 {
                defs.push(format!(
                    "\"d{i:04}\": {{ \"type\": \"float\", \"inherits\": \"d{:04}\" }}",
                    i + 1
                ));
            } else {
                defs.push(format!("\"d{i:04}\": {{ \"type\": \"float\" }}"));
            }
        }
        let chunk = format!(
            "{{ \"package\": \"p1\", \"version\": \"1.0\", \"definitions\": {{ {} }} }}",
            defs.join(", ")
        );

        compiler.add_chunk_auto("chunk.json", &chunk).unwrap();
        compiler
            .link_and_verify()
            .expect("a chain at the depth ceiling must be accepted");
    }

    #[test]
    fn test_link_and_verify_component_dependency_depth_ceiling() {
        // configflux-xowl.5: same ceiling on the component `depends_on` DFS.
        let mut compiler = Compiler::new();
        let mut comps: Vec<String> = Vec::new();
        for i in 0..1_100 {
            if i + 1 < 1_100 {
                comps.push(format!(
                    "\"c{i:04}\": {{ \"type\": \"actuator\", \"depends_on\": [\"c{:04}\"] }}",
                    i + 1
                ));
            } else {
                comps.push(format!("\"c{i:04}\": {{ \"type\": \"actuator\" }}"));
            }
        }
        let chunk = format!(
            "{{ \"package\": \"p1\", \"version\": \"1.0\", \"components\": {{ {} }} }}",
            comps.join(", ")
        );

        compiler.add_chunk_auto("chunk.json", &chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("exceeds the maximum supported depth"),
            "err: {err}"
        );
    }

    #[test]
    fn test_link_and_verify_condition_compatibility_ok() {
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.root]
            type = "actuator"
            condition = "variant == 'heavy' && region == 'us'"
            depends_on = ["child"]

            [components.child]
            type = "actuator"
            condition = "variant == 'heavy'"
        "#;

        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        compiler.link_and_verify().unwrap();
    }

    #[test]
    fn test_link_and_verify_condition_compatibility_fail() {
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.root]
            type = "actuator"
            condition = "variant == 'heavy'"
            depends_on = ["child"]

            [components.child]
            type = "actuator"
            condition = "region == 'us'"
        "#;

        compiler.add_chunk_auto("chunk.toml", chunk).unwrap();
        let err = compiler.link_and_verify().unwrap_err();
        assert!(
            format!("{err}").contains("Condition incompatibility"),
            "err: {err}"
        );
    }

    #[test]
    fn test_emit_ir_writes_chunk_and_index() {
        let mut compiler = Compiler::new();

        // Authored via CUE-JSON (the `inherits` pointer is carried verbatim into
        // the emitted chunk per ADR-0027); authoring `inherits` directly in TOML
        // is rejected at ingest (configflux-qofj).
        let chunk = r#"{
            "package": "p1",
            "version": "1.0",
            "definitions": {
                "safe_speed": { "type": "float", "unit": "m/s" }
            },
            "components": {
                "motor": {
                    "type": "actuator",
                    "params": {
                        "speed": { "inherits": "safe_speed", "value": 1.0 }
                    }
                }
            }
        }"#;

        compiler
            .add_chunk_json_with_source("configs/motor.json", chunk)
            .unwrap();

        let temp_dir = unique_temp_path("configflux-ir", "test");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let index = compiler.emit_ir(&temp_dir).unwrap();

        assert_eq!(index.chunks.len(), 1);
        let chunk_path = temp_dir.join(format!("chunk-{}.cfir", index.chunks[0].chunk_hash));
        assert!(chunk_path.exists(), "chunk path missing: {:?}", chunk_path);
        let index_path = temp_dir.join("index.cfir.json");
        assert!(index_path.exists(), "index path missing: {:?}", index_path);

        let on_disk: crate::ir::IrIndex =
            serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
        assert_eq!(on_disk.config_hash, on_disk.compute_config_hash().unwrap());

        verify_ir_dir(&temp_dir).unwrap();

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_resolve_missing_component_type() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            condition = "variant == 'heavy'"
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[("variant", "heavy")]);
        let err = resolve(raw, &ctx).unwrap_err();
        assert!(
            format!("{err}").contains("missing required 'type'"),
            "err: {err}"
        );
    }

    #[test]
    fn test_resolve_missing_param_type() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            value = 1.0
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[]);
        let err = resolve(raw, &ctx).unwrap_err();
        assert!(
            format!("{err}").contains("Error resolving parameter"),
            "err: {err}"
        );
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Missing 'type' for parameter")),
            "err: {err}"
        );
    }

    #[test]
    fn test_resolve_unknown_inherits() {
        // Resolve-time validation of the `inherits` pointer is retained even
        // though gap-fill moved to CUE: an `inherits` that resolves to no known
        // definition is still rejected at resolve time.
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            inherits = "unknown_def"
            value = 1.0
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[]);
        let err = resolve(raw, &ctx).unwrap_err();
        assert!(
            format!("{err}").contains("Error resolving parameter"),
            "err: {err}"
        );
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("unknown definition")),
            "err: {err}"
        );
    }

    #[test]
    fn test_resolve_drops_param_without_value() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            type = "float"
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[]);
        let err = resolve(raw, &ctx).unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.to_string().contains("Missing 'value' for parameter")),
            "err: {err}"
        );
    }

    #[test]
    fn test_component_condition_filters_out() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
            condition = "variant == 'heavy'"
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[("variant", "light")]);
        let resolved = resolve(raw, &ctx).unwrap();
        assert!(!resolved.components.contains_key("motor"));
    }

    #[test]
    fn test_condition_parse_error() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
            condition = "variant =="
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[("variant", "heavy")]);
        let err = resolve(raw, &ctx).unwrap_err();
        assert!(
            format!("{err}").contains("Failed to evaluate condition"),
            "err: {err}"
        );
    }

    #[test]
    fn test_nested_override_applies_depth_first() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            type = "float"
            value = 10.0

            [[components.motor.params.speed.overrides]]
            condition = "variant == 'heavy'"
            value = 7.0

            [[components.motor.params.speed.overrides.overrides]]
            condition = "region == 'eu'"
            value = 6.0
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[("variant", "heavy"), ("region", "eu")]);
        let resolved = resolve(raw, &ctx).unwrap();
        let motor = &resolved.components["motor"];
        let speed = &motor.params["speed"];
        match &speed.value {
            schema::Value::Float(v) => assert_eq!(*v, 6.0),
            _ => panic!("Expected float"),
        }
    }

    #[test]
    fn test_multiple_overrides_last_wins() {
        let input = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            type = "float"
            value = 10.0

            [[components.motor.params.speed.overrides]]
            condition = "variant == 'heavy'"
            value = 7.0

            [[components.motor.params.speed.overrides]]
            condition = "variant == 'heavy'"
            value = 5.0
        "#;

        let raw = toml::from_str::<Config>(input).unwrap();
        let ctx = make_ctx(&[("variant", "heavy")]);
        let resolved = resolve(raw, &ctx).unwrap();
        let motor = &resolved.components["motor"];
        let speed = &motor.params["speed"];
        match &speed.value {
            schema::Value::Float(v) => assert_eq!(*v, 5.0),
            _ => panic!("Expected float"),
        }
    }

    // --- configflux-qofj: authored TOML carrying `inherits` is rejected at
    // ingest (parse) time, BEFORE resolve, so the post-L2 silent-default path
    // in `resolver.rs` (safety=QM / lifecycle=Runtime / access=Technician) is
    // unreachable via the public TOML surface. Post-ADR-0027, `inherits`
    // gap-fill happens only in CUE whole-pack export; CUE-emitted JSON carries
    // resolved `inherits` pointers verbatim and is NOT affected by this guard.
    // The retained TOML path (ADR-0027 Decision 9 erratum) exists only for the
    // inherits-free scenario mutation fixtures, which keep ingesting fine.

    #[test]
    fn test_toml_chunk_with_inherits_on_param_rejected_at_ingest() {
        // THE footgun: a TOML param authors its own type/value but relies on
        // inheriting safety/lifecycle/access from a SIL-rated definition. Pre-fix
        // this ingested cleanly and SILENTLY defaulted safety=QM at resolve — a
        // silent safety-level downgrade. It must now hard-fail at ingest.
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [definitions.brake_pressure]
            type = "float"
            safety = "sil3"
            lifecycle = "startup"
            access = "supervisor"

            [components.brakes]
            type = "actuator"

            [components.brakes.params.pressure]
            inherits = "brake_pressure"
            type = "float"
            value = 12.0
        "#;

        let err = compiler.add_chunk_auto("brakes.toml", chunk).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("inherits"),
            "error must name the `inherits` field: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("cue"),
            "error must point the author at CUE authoring: {msg}"
        );
        assert!(
            msg.contains("pressure"),
            "error must name the offending parameter path: {msg}"
        );
    }

    #[test]
    fn test_toml_chunk_with_inherits_value_only_rejected_at_ingest() {
        // The exact pattern the shipped getting-started guide taught: a TOML
        // param with `inherits` + `value` only. Pre-fix this hard-failed LATE at
        // resolve ("Missing type for parameter"); it must now fail at ingest with
        // a deterministic diagnostic that points at CUE.
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [definitions.safe_speed]
            type = "float"
            unit = "m/s"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            inherits = "safe_speed"
            value = 1.0
        "#;

        let err = compiler.add_chunk_auto("motor.toml", chunk).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("inherits") && msg.to_lowercase().contains("cue"),
            "ingest-time inherits rejection must point at CUE: {msg}"
        );
        // It must NOT be the late resolve-time "Missing 'type'" failure.
        assert!(
            !msg.contains("Missing 'type'"),
            "rejection must fire at ingest, before resolve: {msg}"
        );
    }

    #[test]
    fn test_toml_chunk_with_inherits_on_definition_rejected_at_ingest() {
        // `inherits` carried on a definition is equally rejected at ingest.
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [definitions.base]
            type = "float"

            [definitions.derived]
            type = "float"
            inherits = "base"
        "#;

        let err = compiler.add_chunk_auto("defs.toml", chunk).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("inherits") && msg.contains("derived"),
            "error must name the offending definition and the `inherits` field: {msg}"
        );
    }

    #[test]
    fn test_toml_chunk_with_inherits_in_override_payload_rejected_at_ingest() {
        // `inherits` is a `Parameter` field and `Parameter` nests via override
        // payloads (`ConditionalBlock.payload: Box<Parameter>`). A nested
        // `inherits` must also be rejected — the scan is recursive.
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "p1"
            version = "1.0"

            [definitions.safe_speed]
            type = "float"

            [components.motor]
            type = "actuator"

            [components.motor.params.speed]
            type = "float"
            value = 10.0

            [[components.motor.params.speed.overrides]]
            condition = "variant == 'heavy'"
            inherits = "safe_speed"
            value = 7.0
        "#;

        let err = compiler.add_chunk_auto("motor.toml", chunk).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("inherits") && msg.to_lowercase().contains("cue"),
            "nested override `inherits` must be rejected at ingest: {msg}"
        );
    }

    #[test]
    fn test_toml_chunk_without_inherits_still_ingests() {
        // Regression guard for the retained mutation-fixture surface (ADR-0027
        // Decision 9 erratum): an inherits-free TOML chunk shaped like the
        // scenario mutation fixtures must keep ingesting.
        let mut compiler = Compiler::new();

        let chunk = r#"
            package = "s1_water_pump_mutation"
            version = "1.0.0"

            [components.control_extension]
            type = "module"
            depends_on = ["power_bus"]

            [components.power_bus]
            type = "module"
        "#;

        compiler
            .add_chunk_auto("mutation.toml", chunk)
            .expect("inherits-free TOML must still ingest");
    }

    #[test]
    fn test_json_chunk_with_inherits_still_ingests() {
        // The guard is TOML-only: CUE-emitted JSON carries resolved `inherits`
        // pointers verbatim (ADR-0027) and must NOT be rejected. Ingesting the
        // same logical chunk as JSON succeeds.
        let mut compiler = Compiler::new();

        let chunk = r#"{
            "package": "p1",
            "version": "1.0",
            "definitions": {
                "safe_speed": { "type": "float", "unit": "m/s" }
            },
            "components": {
                "motor": {
                    "type": "actuator",
                    "params": {
                        "speed": { "inherits": "safe_speed", "type": "float", "value": 1.0 }
                    }
                }
            }
        }"#;

        compiler
            .add_chunk_auto("motor.json", chunk)
            .expect("CUE-emitted JSON carrying `inherits` must still ingest");
    }
