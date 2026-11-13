use super::{Expected, Kind, Location, Resolver, TypeOrigin, parse_fragment};
use crate::json::{self, Value};
use crate::{ValidationReport, validate};

fn with_resolver(input: &str, check: impl FnOnce(&mut Resolver<'_>)) {
    let document = json::parse(input).unwrap();
    let original = document.clone();
    let mut resolver = Resolver::new(&document);
    resolver.prepare();
    check(&mut resolver);
    assert_eq!(
        document, original,
        "resolution must not mutate the source AST"
    );
}

fn token(resolver: &Resolver<'_>, path: &str) -> usize {
    *resolver
        .tokens()
        .iter()
        .find(|&&id| resolver.effective_path(id) == path)
        .unwrap_or_else(|| panic!("missing effective token {path}"))
}

fn stage_report(input: &str) -> ValidationReport {
    let document = json::parse(input).unwrap();
    let mut diagnostics = crate::structure::validate(&document);
    let mut resolver = Resolver::new(&document);
    resolver.prepare();
    resolver.validate_references();
    diagnostics.extend(resolver.into_diagnostics());
    ValidationReport::from_diagnostics(diagnostics)
}

fn accepts(input: &str) {
    let report = stage_report(input);
    assert!(report.valid, "{input}: {:?}", report.diagnostics);
}

fn rejects_at(input: &str, path: &str, code: &str) {
    let report = stage_report(input);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == path && diagnostic.code == code),
        "{input}: expected {code} at {path}, got {:?}",
        report.diagnostics
    );
}

fn number(resolver: &mut Resolver<'_>, path: &str) -> String {
    let id = token(resolver, path);
    let value = resolver.token_value(id).unwrap();
    let Value::Number(number) = resolver.value(value.cursor) else {
        panic!("expected a number at {path}")
    };
    number.as_str().to_owned()
}

#[test]
fn classifies_ref_only_objects_by_target_and_explicit_type() {
    let input = r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$ref":"#/base"}}"##;
    with_resolver(input, |resolver| {
        assert_eq!(number(resolver, "/derived/n"), "1");
        assert_eq!(
            resolver.token_path(token(resolver, "/derived/n")),
            "/base/n"
        );
        assert!(
            !resolver
                .tokens()
                .iter()
                .any(|&id| resolver.effective_path(id) == "/derived")
        );
    });
    with_resolver(
        r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$type":"color","$ref":"#/base"}}"##,
        |resolver| {
            let derived = token(resolver, "/derived");
            assert_eq!(resolver.kind(derived), Ok(Kind::Token));
            assert_eq!(resolver.token_type(derived).unwrap().name, "color");
            let value = resolver.token_value(derived).unwrap();
            assert!(matches!(value.cursor.location, Location::Namespace(_)));
            assert!(matches!(resolver.value(value.cursor), Value::Object(_)));
            assert!(
                !resolver
                    .tokens()
                    .iter()
                    .any(|&id| resolver.effective_path(id) == "/derived/n")
            );
        },
    );
    accepts(
        r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$type":"number","$ref":"#/base","local":{"$value":2}}}"##,
    );
}

#[test]
fn resolves_ref_only_token_aliases_and_curly_alias_chains() {
    let input = r##"{"n":{"$type":"number","$value":3},"alias":{"$ref":"#/n/$value"},"last":{"$value":"{alias}"}}"##;
    accepts(input);
    with_resolver(input, |resolver| {
        let alias = token(resolver, "/alias");
        assert!(matches!(
            resolver.token_type(alias).unwrap().origin,
            TypeOrigin::Referenced(_)
        ));
        assert_eq!(number(resolver, "/alias"), "3");
        assert_eq!(number(resolver, "/last"), "3");
    });
    assert!(
        validate(input).valid,
        "the public pipeline must include the resolver"
    );
}

#[test]
fn rejects_ambiguous_envelopes_without_resolving_either_branch() {
    rejects_at(
        r##"{"x":{"$type":"number","$value":1,"$ref":"#/missing"}}"##,
        "/x/$ref",
        "structure",
    );
    rejects_at(
        r##"{"x":{"$extends":"{missing}","$ref":"#/also-missing"}}"##,
        "/x/$ref",
        "structure",
    );
}

#[test]
fn resolves_curly_and_pointer_group_extensions_with_deep_overrides() {
    let input = r##"{
      "derived":{"$extends":"{base}","$type":"fontWeight",
        "nested":{"local":{"$value":4}},
        "replace":{"$value":5},"becomesGroup":{"child":{"$value":6}}},
      "pointer":{"$extends":"#/derived"},
      "base":{"$type":"number","nested":{"keep":{"$value":1}},
        "replace":{"$type":"number","$description":"old","$value":2},
        "becomesGroup":{"$type":"number","$value":3}}
    }"##;
    accepts(input);
    with_resolver(input, |resolver| {
        for (path, expected) in [
            ("/derived/nested/keep", "1"),
            ("/derived/nested/local", "4"),
            ("/derived/replace", "5"),
            ("/derived/becomesGroup/child", "6"),
            ("/pointer/nested/keep", "1"),
        ] {
            assert_eq!(number(resolver, path), expected);
        }
        let replaced = token(resolver, "/derived/replace");
        assert_eq!(resolver.token_type(replaced).unwrap().name, "fontWeight");
        assert_eq!(
            resolver.token_type(replaced).unwrap().origin,
            TypeOrigin::Group
        );
        assert_eq!(resolver.token_path(replaced), "/derived/replace");
        assert_eq!(
            resolver.token_path(token(resolver, "/derived/nested/keep")),
            "/base/nested/keep"
        );
        assert!(
            !resolver
                .tokens()
                .iter()
                .any(|&id| resolver.effective_path(id) == "/derived/becomesGroup")
        );
    });
    accepts(
        r#"{"base":{"n":{"$type":"number","$value":1},"g":{"child":{"$type":"number","$value":2}}},"derived":{"$extends":"{base}","g":{"$type":"number","$value":3}}}"#,
    );
}

#[test]
fn local_group_metadata_replaces_inherited_metadata_without_recursive_merging() {
    rejects_at(
        r##"{"base":{"$extensions":{"vendor":{"old":1}}},"derived":{"$extends":"{base}","$extensions":{"vendor":{"new":2}}},"x":{"$type":"number","$ref":"#/derived/$extensions/vendor/old"}}"##,
        "/x/$ref",
        "reference",
    );
    accepts(
        r##"{"base":{"$extensions":{"vendor":{"old":1}}},"derived":{"$extends":"{base}","$extensions":{"vendor":{"new":2}}},"x":{"$type":"number","$ref":"#/derived/$extensions/vendor/new"}}"##,
    );
}

#[test]
fn uses_effective_parent_defaults_but_alias_types_take_precedence() {
    let input = r#"{"base":{"$type":"number","n":{"$value":1}},"derived":{"$extends":"{base}","$type":"fontWeight"},"g":{"$type":"color","a":{"$value":"{base.n}"}}}"#;
    accepts(input);
    with_resolver(input, |resolver| {
        let inherited = token(resolver, "/derived/n");
        assert_eq!(
            resolver.effective_parent_type(inherited),
            Ok(Some("fontWeight"))
        );
        assert_eq!(resolver.token_type(inherited).unwrap().name, "fontWeight");
        let alias = token(resolver, "/g/a");
        assert_eq!(resolver.effective_parent_type(alias), Ok(Some("color")));
        assert_eq!(resolver.token_type(alias).unwrap().name, "number");
        assert!(matches!(
            resolver.token_type(alias).unwrap().origin,
            TypeOrigin::Referenced(_)
        ));
    });
    accepts(r#"{"g":{"$type":"number","$root":{"$value":1}},"x":{"$value":"{g.$root}"}}"#);
}

#[test]
fn classifies_forward_sibling_references_during_a_deep_merge() {
    accepts(
        r##"{"base":{"candidate":{"old":{"$type":"number","$value":1}}},"derived":{"$extends":"{base}","candidate":{"$ref":"#/derived/sibling"},"sibling":{"fresh":{"$type":"number","$value":2}}}}"##,
    );
}

#[test]
fn rejects_missing_malformed_and_wrong_kind_group_references() {
    for reference in [
        "{missing}",
        "#/missing",
        "https://example.com/tokens",
        "bad",
        "#/x/~2",
    ] {
        rejects_at(
            &format!(r#"{{"g":{{"$extends":"{reference}"}}}}"#),
            "/g/$extends",
            "reference",
        );
    }
    rejects_at(
        r#"{"n":{"$type":"number","$value":1},"g":{"$extends":"{n}"}}"#,
        "/g/$extends",
        "reference",
    );
    rejects_at(
        r##"{"g":{"$ref":"#/missing","child":{}}}"##,
        "/g/$ref",
        "reference",
    );
    rejects_at(r#"{"g":{"$extends":42}}"#, "/g/$extends", "reference");
    rejects_at(
        r#"{"n":{"$type":"number","$ref":false}}"#,
        "/n/$ref",
        "reference",
    );
    rejects_at(
        r##"{"base":{},"g":{"$root":{"$ref":"#/base"}}}"##,
        "/g/$root",
        "structure",
    );
}

#[test]
fn diagnoses_every_direct_and_containment_cycle_participant() {
    let direct = r#"{"a":{"$extends":"{b}"},"b":{"$extends":"{c}"},"c":{"$extends":"{a}"}}"#;
    for path in ["/a/$extends", "/b/$extends", "/c/$extends"] {
        rejects_at(direct, path, "cycle");
    }
    let containment = r#"{"a":{"b":{"$extends":"{a}"}}}"#;
    rejects_at(containment, "/a", "cycle");
    rejects_at(containment, "/a/b/$extends", "cycle");
    let nested = r##"{"a":{"middle":{"b":{"$ref":"#/a"}}}}"##;
    for path in ["/a", "/a/middle", "/a/middle/b/$ref"] {
        rejects_at(nested, path, "cycle");
    }
    accepts(r#"{"a":{"$extends":"{a.b}","b":{"n":{"$type":"number","$value":1}}}}"#);
}

#[test]
fn diagnoses_all_alias_cycle_members_without_labeling_dependents() {
    let input = r##"{"a":{"$type":"number","$value":"{b}"},"b":{"$type":"number","$ref":"#/c/$value"},"c":{"$type":"number","$value":"{a}"},"dependent":{"$type":"number","$value":"{a}"}}"##;
    for path in ["/a/$value", "/b/$ref", "/c/$value"] {
        rejects_at(input, path, "cycle");
    }
    let report = stage_report(input);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "cycle"
                && diagnostic.path.starts_with("/dependent")),
        "{:?}",
        report.diagnostics
    );
    let classification = r##"{"a":{"$ref":"#/b"},"b":{"$ref":"#/a"}}"##;
    for path in ["/a/$ref", "/b/$ref"] {
        rejects_at(classification, path, "cycle");
    }
}

#[test]
fn diagnoses_cycles_in_nested_pointer_wrappers() {
    let input = r##"{"a":{"$type":"gradient","$value":[{"$ref":"#/b/$value/0"}]},"b":{"$type":"gradient","$value":[{"$ref":"#/a/$value/0"}]}}"##;
    let report = stage_report(input);
    for prefix in ["/a/$value", "/b/$value"] {
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic.code == "cycle" && diagnostic.path.starts_with(prefix)), "{:?}", report.diagnostics);
    }
}

#[test]
fn parses_rfc6901_fragments_without_losing_empty_segments() {
    for (fragment, expected) in [
        ("#", vec![]),
        ("#/", vec![""]),
        ("#//", vec!["", ""]),
        ("#/a//b/", vec!["a", "", "b", ""]),
        ("#/%C3%A9%F0%9F%9A%B2", vec!["é🚲"]),
        ("#/a~1b/~0key/~01", vec!["a/b", "~key", "~1"]),
        ("#%2Fa%2Fb", vec!["a", "b"]),
        ("#/%252F", vec!["%2F"]),
    ] {
        assert_eq!(parse_fragment(fragment).unwrap(), expected, "{fragment}");
    }
    for fragment in [
        "",
        "/a",
        "https://example.com/#/a",
        "#a",
        "#/%",
        "#/%0",
        "#/%GG",
        "#/%FF",
        "#/%C3%28",
        "#/a~",
        "#/a~2",
    ] {
        assert!(parse_fragment(fragment).is_err(), "{fragment}");
    }
}

#[test]
fn resolves_escaped_unicode_empty_and_numeric_object_names() {
    let input = r##"{"":{"":{"":{"$type":"number","$value":1}}},"a/b":{"$type":"number","$value":2},"~1":{"$type":"number","$value":3},"é🚲":{"$type":"number","$value":4},"01":{"$type":"number","$value":5},"a":{"$type":"number","$ref":"#////$value"},"b":{"$type":"number","$ref":"#/a~1b/$value"},"c":{"$type":"number","$ref":"#/~01/$value"},"d":{"$type":"number","$ref":"#/%C3%A9%F0%9F%9A%B2/$value"},"e":{"$type":"number","$ref":"#/01/$value"}}"##;
    // Three empty namespace segments require three slashes plus the slash
    // introducing $value. Exercise trailing and doubled separators exactly.
    accepts(input);
    with_resolver(input, |resolver| {
        for (path, expected) in [
            ("/a", "1"),
            ("/b", "2"),
            ("/c", "3"),
            ("/d", "4"),
            ("/e", "5"),
        ] {
            assert_eq!(number(resolver, path), expected);
        }
    });
    accepts(r##"{"":{"$type":"number","$value":1},"x":{"$type":"number","$ref":"#//$value"}}"##);
    accepts(r#"{"":{"":{"$type":"number","$value":1}},"x":{"$type":"number","$value":"{.}"}}"#);
}

#[test]
fn keeps_root_empty_member_and_token_envelope_targets_distinct() {
    with_resolver(
        r##"{"":{"$type":"number","$value":1},"root":{"$type":"number","$ref":"#"},"empty":{"$type":"number","$ref":"#/"},"value":{"$type":"number","$ref":"#//$value"}}"##,
        |resolver| {
            let root = resolver.token_value(token(resolver, "/root")).unwrap();
            let empty = resolver.token_value(token(resolver, "/empty")).unwrap();
            assert_eq!(root.cursor.location, Location::Namespace(0));
            assert!(matches!(empty.cursor.location, Location::Namespace(view) if view != 0));
            assert!(matches!(resolver.value(empty.cursor), Value::Object(_)));
            assert_eq!(number(resolver, "/value"), "1");
        },
    );
    rejects_at(
        r##"{"n":{"$type":"number","$value":1},"alias":{"$type":"number","$ref":"#/n/$value"},"x":{"$type":"number","$ref":"#/alias/$value"}}"##,
        "/x/$ref",
        "reference",
    );
}

#[test]
fn pointer_metadata_remains_literal_and_does_not_create_false_cycles() {
    accepts(r##"{"a":{"$type":"fontFamily","$ref":"#/a/$type"}}"##);
    accepts(
        r##"{"a":{"$type":"fontFamily","$value":"Inter"},"b":{"$type":"fontFamily","$ref":"#/a/$type"}}"##,
    );
    with_resolver(
        r##"{"a":{"$type":"fontFamily","$description":"{missing}","$value":"Inter","$extensions":{"vendor":{"$ref":"#/missing"}}},"literal":{"$type":"fontFamily","$ref":"#/a/$description"},"object":{"$type":"number","$ref":"#/a/$extensions/vendor"}}"##,
        |resolver| {
            let literal = resolver.token_value(token(resolver, "/literal")).unwrap();
            assert_eq!(
                resolver.value(literal.cursor),
                &Value::String("{missing}".to_owned())
            );
            let object = resolver.token_value(token(resolver, "/object")).unwrap();
            assert!(object.cursor.opaque);
            assert!(matches!(resolver.value(object.cursor), Value::Object(_)));
            resolver.validate_references();
            assert!(
                resolver.diagnostics.is_empty(),
                "{:?}",
                resolver.diagnostics
            );
        },
    );
}

#[test]
fn pointer_lookup_requires_unique_members_in_every_region() {
    for input in [
        r##"{"n":{"$type":"number","$value":1},"n":{"$type":"number","$value":2},"x":{"$type":"number","$ref":"#/n/$value"}}"##,
        r##"{"n":{"$type":"number","$value":1,"$value":1},"x":{"$type":"number","$ref":"#/n/$value"}}"##,
        r##"{"n":{"$type":"fontFamily","$type":"fontFamily","$value":"Inter"},"x":{"$type":"fontFamily","$ref":"#/n/$type"}}"##,
        r##"{"$extensions":{"vendor":{"n":1,"n":1}},"x":{"$type":"number","$ref":"#/$extensions/vendor/n"}}"##,
    ] {
        rejects_at(input, "/x/$ref", "reference");
    }
    accepts(r#"{"n":{"$type":"number","$value":1},"n":{"$type":"number","$value":2}}"#);
}

#[test]
fn enforces_array_index_syntax_without_applying_it_to_object_keys() {
    for index in ["-", "00", "01", "+0", "-0", "1", "184467440737095516160"] {
        rejects_at(
            &format!(
                r##"{{"a":{{"$type":"fontFamily","$value":["Inter"]}},"x":{{"$type":"fontFamily","$ref":"#/a/$value/{index}"}}}}"##
            ),
            "/x/$ref",
            "reference",
        );
    }
    accepts(
        r##"{"a":{"$type":"fontFamily","$value":["Inter"]},"x":{"$type":"fontFamily","$ref":"#/a/$value/0"}}"##,
    );
}

#[test]
fn traverses_reference_valued_intermediates_in_the_expanded_document() {
    let input = r##"{"base":{"c":{"$type":"color","$value":{"colorSpace":"srgb","components":[0.25,0.5,1]}}},"derived":{"$extends":"{base}"},"alias":{"$value":"{derived.c}"},"wrapper":{"$type":"color","$value":{"$ref":"#/alias/$value"}},"x":{"$type":"number","$ref":"#/wrapper/$value/components/0"}}"##;
    accepts(input);
    with_resolver(input, |resolver| assert_eq!(number(resolver, "/x"), "0.25"));
}

#[test]
fn resolves_nested_pointer_wrappers_at_primitive_and_composite_positions() {
    accepts(
        r##"{"$extensions":{"data":{"unit":"px","n":2,"name":"Inter","components":[0,0.5,1]}},"d":{"$type":"dimension","$value":{"value":{"$ref":"#/$extensions/data/n"},"unit":{"$ref":"#/$extensions/data/unit"}}},"f":{"$type":"fontFamily","$value":[{"$ref":"#/$extensions/data/name"}]},"c":{"$type":"color","$value":{"colorSpace":"srgb","components":{"$ref":"#/$extensions/data/components"}}},"b":{"$type":"border","$value":{"color":"{c}","width":{"$ref":"#/d/$value"},"style":"solid"}}}"##,
    );
    rejects_at(
        r##"{"x":{"$type":"dimension","$value":{"value":{"$ref":"#/missing"},"unit":"px"}}}"##,
        "/x/$value/value/$ref",
        "reference",
    );
    rejects_at(
        r##"{"x":{"$type":"color","$value":{"colorSpace":"srgb","components":[0,{"$ref":"#/missing"},1]}}}"##,
        "/x/$value/components/1/$ref",
        "reference",
    );
}

#[test]
fn primitive_inner_curly_strings_and_singular_extras_are_not_references() {
    accepts(
        r##"{"d":{"$type":"dimension","$value":{"value":"{missing}","unit":"{missing}","extra":{"$ref":"#/missing"}}},"t":{"$type":"duration","$value":{"value":"{missing}","unit":"{missing}","extra":{"$ref":"#/missing"}}},"c":{"$type":"color","$value":{"colorSpace":"{missing}","components":["{missing}","{missing}","{missing}"],"extra":{"$ref":"#/missing"}}},"f":{"$type":"fontFamily","$value":["{missing}"]},"b":{"$type":"cubicBezier","$value":["{missing}",0,1,1]}}"##,
    );
}

#[test]
fn array_element_references_preserve_arrays_instead_of_splicing_them() {
    with_resolver(
        r##"{"g":{"$type":"gradient","$value":[]},"nested":{"$type":"gradient","$value":[{"$ref":"#/g/$value"}]},"s":{"$type":"shadow","$value":[]},"nestedShadow":{"$type":"shadow","$value":[{"$ref":"#/s/$value"}]}}"##,
        |resolver| {
            for path in ["/nested", "/nestedShadow"] {
                let root = resolver.token_value(token(resolver, path)).unwrap();
                let Value::Array(values) = resolver.value(root.cursor) else {
                    panic!("expected array")
                };
                assert_eq!(values.len(), 1);
                let child = resolver.child(root.cursor, 0);
                let child = resolver.resolve_head(child, Expected::Literal).unwrap();
                assert_eq!(resolver.value(child.cursor), &Value::Array(Vec::new()));
            }
        },
    );
}

#[test]
fn only_curly_aliases_require_identical_token_types() {
    rejects_at(
        r#"{"n":{"$type":"number","$value":1},"w":{"$type":"fontWeight","$value":"{n}"}}"#,
        "/w/$value",
        "reference",
    );
    accepts(
        r##"{"n":{"$type":"number","$value":1},"w":{"$type":"fontWeight","$ref":"#/n/$value"}}"##,
    );
    rejects_at(
        r#"{"g":{},"n":{"$type":"number","$value":"{g}"}}"#,
        "/n/$value",
        "reference",
    );
    rejects_at(
        r#"{"n":{"$type":"number","$value":"{missing}"}}"#,
        "/n/$value",
        "reference",
    );
}

#[test]
fn reports_inherited_reference_errors_at_original_declarations() {
    let report = stage_report(
        r#"{"base":{"n":{"$type":"number","$value":"{missing}"}},"derived":{"$extends":"{base}"}}"#,
    );
    assert!(report.diagnostics.iter().any(|diagnostic| diagnostic.path == "/base/n/$value" && diagnostic.code == "reference"));
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path.starts_with("/derived/n")),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn missing_types_are_not_inferred_from_value_shapes_or_names() {
    rejects_at(r#"{"number":{"$value":1}}"#, "/number", "type");
    rejects_at(
        r#"{"dimension":{"$value":{"value":1,"unit":"px"}}}"#,
        "/dimension",
        "type",
    );
}

#[test]
fn distinguishes_pointer_wrappers_from_opaque_singular_extra_members() {
    accepts(
        r##"{"d":{"$type":"dimension","$value":{"value":1,"unit":"px","$ref":"#/missing"}},"c":{"$type":"color","$value":{"colorSpace":"srgb","components":[0,0,0],"$ref":"#/missing"}}}"##,
    );
    rejects_at(
        r##"{"x":{"$type":"number","$value":{"$ref":"#/a","$ref":"#/b"}}}"##,
        "/x/$value/$ref",
        "structure",
    );
}

#[test]
fn resolves_curly_references_in_composite_array_slots() {
    rejects_at(
        r#"{"s":{"$type":"shadow","$value":["{missing}"]}}"#,
        "/s/$value/0",
        "reference",
    );
    rejects_at(
        r#"{"g":{"$type":"gradient","$value":["{missing}"]}}"#,
        "/g/$value/0",
        "reference",
    );
    with_resolver(
        r#"{"s":{"$type":"shadow","$value":{}},"array":{"$type":"shadow","$value":["{s}"]}}"#,
        |resolver| {
            let array = resolver.token_value(token(resolver, "/array")).unwrap();
            let child = resolver.child(array.cursor, 0);
            let value = resolver
                .resolve_head(child, Expected::Type("shadow"))
                .unwrap();
            assert!(matches!(resolver.value(value.cursor), Value::Object(_)));
        },
    );
}

#[test]
fn pointers_preserve_literal_primitive_strings_when_reused_as_whole_values() {
    accepts(
        r##"{"family":{"$type":"fontFamily","$value":["{missing}"]},"literal":{"$type":"fontFamily","$ref":"#/family/$value/0"}}"##,
    );
}
