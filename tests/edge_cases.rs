use dtcg_validator::{ValidationReport, validate};
use serde_json::{Value, json};

// Value-module unit tests already exercise every type, all 14 color-space
// boundaries, literal enums, primitive reference positions and composite field
// shapes through validate. These tests cover public-pipeline interactions and
// rules whose existing focused tests only exercise individual internal stages.
fn accepts(input: &str) {
    let report = validate(input);
    assert!(report.valid, "expected valid input:\n{input}\n{report:#?}");
    assert!(report.diagnostics.is_empty(), "{report:#?}");
}

fn rejects(input: &str) -> ValidationReport {
    let report = validate(input);
    assert!(!report.valid, "expected invalid input:\n{input}");
    assert!(!report.diagnostics.is_empty(), "{report:#?}");
    report
}

fn has_diagnostic(report: &ValidationReport, path: &str, code: &str) {
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == path && diagnostic.code == code),
        "expected {code} at {path}, got {report:#?}"
    );
}

fn rejects_at(input: &str, path: &str, code: &str) -> ValidationReport {
    let report = rejects(input);
    has_diagnostic(&report, path, code);
    report
}

#[test]
fn required_positive_verification_probes_use_the_public_core() {
    for input in [
        r#"{"x":{"$type":"number","$value":3},"y":{"$value":"{x}"}}"#,
        r#"{"g":{"$type":"number","$root":{"$value":1}},"x":{"$value":"{g.$root}"}}"#,
        r#"{"n":{"$type":"number","$value":1},"g":{"$type":"color","a":{"$value":"{n}"}}}"#,
        r##"{"a":{"$type":"fontFamily","$value":"Inter"},"b":{"$type":"fontFamily","$ref":"#/a/$type"}}"##,
        r##"{"a/b":{"$type":"number","$value":7},"x":{"$type":"number","$ref":"#/a~1b/$value"}}"##,
        r#"{"n":{"$type":"number","$value":1e1000}}"#,
        r#"{"n":{"$type":"fontWeight","$value":999.5}}"#,
        r#"{"g":{"$type":"gradient","$value":[{"color":{"colorSpace":"srgb","components":[0,"none",1]},"position":42}]}}"#,
    ] {
        accepts(input);
    }
}

#[test]
fn required_negative_probes_keep_original_source_paths() {
    for (input, path, code) in [
        (
            r#"{"n":{"$type":"number","$value":"bad"}}"#,
            "/n/$value",
            "value",
        ),
        (
            r#"{"n":{"$type":"number","$value":"{missing}"}}"#,
            "/n/$value",
            "reference",
        ),
        (
            r#"{"n":{"$type":"fontWeight","$value":1000.0000000000000001}}"#,
            "/n/$value",
            "value",
        ),
        (
            r#"{"c":{"$type":"color","$value":{"colorSpace":"hsl","components":[360,0,0]}}}"#,
            "/c/$value/components/0",
            "value",
        ),
    ] {
        rejects_at(input, path, code);
    }
}

#[test]
fn unknown_metadata_and_vendor_extensions_stay_opaque_at_every_level() {
    let opaque = json!({
        "$schema": "https://example.invalid/tokens.schema.json",
        "$description": "Use {missing} instead",
        "$deprecated": "Use {also.missing}",
        "$customProp": "{missing}",
        "$vendor.with{punctuation}": true,
        "$empty": {},
        "$metadata": {"$type": 42, "$description": false, "label": "{missing}"},
        "$array": [{"$value": 1}, {"$ref": "#/missing"}],
        "$extensions": {
            "plainKey": null,
            "scalar": 1,
            "array": [true, "{missing}"],
            "$value": {"$type": false, "bad.name": {"$ref": "#/missing"}},
            "vendor": {"$extends": "{missing}", "$root": []}
        }
    });
    let mut token = opaque.clone();
    token["$type"] = json!("number");
    token["$value"] = json!(1);
    let mut group = opaque.clone();
    group["token"] = token;
    let mut document = opaque;
    document["group"] = group;
    accepts(&document.to_string());

    for deprecated in [json!(true), json!(false), json!(""), json!("{missing}")] {
        accepts(
            &json!({"$deprecated": deprecated, "g": {
                "$deprecated": deprecated,
                "n": {"$type": "number", "$value": 1, "$deprecated": deprecated}
            }})
            .to_string(),
        );
    }
}

#[test]
fn known_metadata_shapes_are_enforced_on_root_groups_and_tokens() {
    for (property, invalid_values) in [
        ("$description", vec![json!(null), json!(1), json!({})]),
        ("$type", vec![json!(null), json!(1), json!([])]),
        ("$deprecated", vec![json!(null), json!(1), json!({})]),
        (
            "$extensions",
            vec![json!(null), json!(false), json!([]), json!("vendor")],
        ),
    ] {
        for invalid in invalid_values {
            for level in ["root", "group", "token"] {
                let mut definition = match level {
                    "token" => json!({"$type": "number", "$value": 1}),
                    _ => json!({}),
                };
                definition[property] = invalid.clone();
                let (document, path) = if level == "root" {
                    (definition, format!("/{property}"))
                } else {
                    (json!({"entry": definition}), format!("/entry/{property}"))
                };
                rejects_at(&document.to_string(), &path, "metadata");
            }
        }
    }
    for unknown_type in ["", "Number", "COLOR", "string", "unknown"] {
        for value in [json!({}), json!({"$value": 1})] {
            let mut definition = value;
            definition["$type"] = json!(unknown_type);
            rejects_at(
                &json!({"entry": definition}).to_string(),
                "/entry/$type",
                "type",
            );
        }
    }
}

#[test]
fn name_restrictions_apply_to_groups_and_tokens_without_normalizing_keys() {
    for name in ["a.b", "a{b", "a}b", ".", "{", "}"] {
        for definition in [json!({}), json!({"$type": "number", "$value": 1})] {
            let mut children = json!({});
            children[name] = definition;
            rejects_at(&children.to_string(), &format!("/{name}"), "structure");
            rejects_at(
                &json!({"nested": children}).to_string(),
                &format!("/nested/{name}"),
                "structure",
            );
        }
    }
    for name in ["", "a/b", "~", "a$b", "é🚲", "A", "a", "01", "100%"] {
        let mut document = json!({});
        document[name] = json!({(name): {"$type": "number", "$value": 1}});
        accepts(&document.to_string());
    }
    accepts(r#"{"A":{"$type":"number","$value":1},"a":{"$type":"number","$value":2}}"#);
}

#[test]
fn unknown_dollar_prefixed_definitions_are_not_accepted_as_metadata() {
    for definition in [
        json!({"$value": 1}),
        json!({"$ref": "#/base"}),
        json!({"$extends": "{base}"}),
        json!({"child": {}}),
        json!({"n": {"$type": "number", "$value": 1}}),
    ] {
        for level in ["root", "group", "token"] {
            let mut document = json!({"base": {}});
            let path = match level {
                "root" => {
                    document["$bad"] = definition.clone();
                    "/$bad"
                }
                "group" => {
                    document["g"] = json!({"$bad": definition});
                    "/g/$bad"
                }
                _ => {
                    document["n"] = json!({
                        "$type": "number", "$value": 1, "$bad": definition
                    });
                    "/n/$bad"
                }
            };
            rejects_at(&document.to_string(), path, "structure");
        }
    }
}

#[test]
fn token_group_and_root_classification_rules_survive_the_full_pipeline() {
    for child in [json!({}), json!({"$type": "number", "$value": 2}), json!(3)] {
        rejects_at(
            &json!({"n": {"$type": "number", "$value": 1, "child": child}}).to_string(),
            "/n/child",
            "structure",
        );
    }
    for root in [json!({}), json!([]), json!({"child": {}})] {
        rejects_at(
            &json!({"g": {"$root": root}}).to_string(),
            "/g/$root",
            "structure",
        );
    }
    rejects_at(
        r##"{"base":{},"g":{"$root":{"$ref":"#/base"}}}"##,
        "/g/$root",
        "structure",
    );
    accepts(
        r##"{"n":{"$type":"number","$value":1},"g":{"$root":{"$ref":"#/n/$value"}},"x":{"$value":"{g.$root}"}}"##,
    );
    accepts(r#"{"$type":"number","$root":{"$value":1},"empty":{}}"#);
    for input in [
        r#"{"number":{"$value":1}}"#,
        r#"{"dimension":{"$value":{"value":1,"unit":"px"}}}"#,
    ] {
        let report = rejects(input);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "type")
        );
    }
}

#[test]
fn duplicate_names_are_valid_json_but_every_definition_is_validated() {
    accepts(
        r#"{"n":{"$type":"number","$value":1},"n":{"$type":"number","$value":2},"g":{},"g":{"n":{"$type":"number","$value":3}}}"#,
    );
    accepts(
        r#"{"n":{"$value":1,"$value":1,"$type":"number","$type":"number","$description":"x","$description":"x","$deprecated":true,"$deprecated":true,"$extensions":{},"$extensions":{}},"$custom":1,"$custom":2}"#,
    );
    accepts(
        r##"{"base":{},"n":{"$type":"number","$value":1},"alias":{"$ref":"#/n/$value","$ref":"#/n/$value"},"g":{"$extends":"{base}","$extends":"{base}"}}"##,
    );
    for input in [
        r#"{"x":{"$description":false},"x":{"$type":"number","$value":2}}"#,
        r#"{"x":{"$type":"number","$value":2},"x":{"$description":false}}"#,
    ] {
        let report = rejects_at(input, "/x/$description", "metadata");
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "json")
        );
    }
    rejects_at(
        r#"{"x":{"$type":"number","$value":1},"x":{"$type":"number","$value":"bad"}}"#,
        "/x/$value",
        "value",
    );
}

#[test]
fn conflicting_duplicate_structural_metadata_is_never_selected_arbitrarily() {
    for (property, first, second) in [
        ("$value", "1", "2"),
        ("$type", "\"number\"", "\"color\""),
        ("$description", "\"text\"", "false"),
        ("$deprecated", "true", "false"),
        ("$extensions", "{}", "null"),
        ("$ref", "\"#/a\"", "\"#/b\""),
        ("$extends", "\"{a}\"", "\"{b}\""),
        ("$root", r#"{"$value":1}"#, r#"{"$value":2}"#),
    ] {
        for (first, second) in [(first, second), (second, first)] {
            let input = format!(
                r#"{{"$type":"number","entry":{{"{property}":{first},"{property}":{second}}}}}"#
            );
            rejects_at(&input, &format!("/entry/{property}"), "structure");
        }
    }
    rejects_at(
        r#"{"n":{"$type":"number","\u0024type":"color","$value":1}}"#,
        "/n/$type",
        "structure",
    );
}

#[test]
fn pointer_lookup_requires_unique_decoded_members_even_for_equal_values() {
    for input in [
        r##"{"n":{"$type":"number","$value":1},"n":{"$type":"number","$value":2},"x":{"$type":"number","$ref":"#/n/$value"}}"##,
        r##"{"n":{"$type":"number","$value":1,"$value":1},"x":{"$type":"number","$ref":"#/n/$value"}}"##,
        r##"{"n":{"$type":"fontFamily","$type":"fontFamily","$value":"Inter"},"x":{"$type":"fontFamily","$ref":"#/n/$type"}}"##,
        r##"{"$extensions":{"vendor":{"n":1,"\u006e":1}},"x":{"$type":"number","$ref":"#/$extensions/vendor/n"}}"##,
        r##"{"$extensions":{"vendor":{"n":1,"n":2}},"x":{"$type":"number","$ref":"#/$extensions/vendor/n"}}"##,
    ] {
        rejects_at(input, "/x/$ref", "reference");
    }
}

#[test]
fn closest_effective_group_defaults_are_validated_without_overriding_alias_types() {
    accepts(
        r#"{"outer":{"$type":"color","inner":{"$type":"number","n":{"$value":1},"explicit":{"$type":"fontFamily","$value":"Inter"}}},"alias":{"$value":"{outer.inner.n}"}}"#,
    );
    rejects_at(
        r#"{"outer":{"$type":"number","inner":{"$type":"fontWeight","n":{"$value":1001}}}}"#,
        "/outer/inner/n/$value",
        "value",
    );
    rejects_at(
        r#"{"base":{"$type":"number","n":{"$value":1001}},"derived":{"$extends":"{base}","$type":"fontWeight"}}"#,
        "/base/n/$value",
        "value",
    );
}

#[test]
fn deep_extensions_merge_groups_and_replace_complete_tokens_in_both_directions() {
    accepts(
        r##"{
          "derived":{"$extends":"{base}","$type":"fontWeight",
            "nested":{"local":{"$value":4}},
            "replace":{"$value":999.5},
            "becomesGroup":{"child":{"$value":6}},
            "becomesToken":{"$value":7}},
          "pointer":{"$extends":"#/derived"},
          "base":{"$type":"number",
            "nested":{"keep":{"$value":1}},
            "replace":{"$type":"number","$description":"obsolete","$value":2},
            "becomesGroup":{"$type":"number","$value":3},
            "becomesToken":{"old":{"$value":8}}},
          "checkReplacementType":{"$type":"fontWeight","$value":"{derived.replace}"},
          "checkNestedKeep":{"$type":"fontWeight","$value":"{pointer.nested.keep}"},
          "checkNestedLocal":{"$type":"fontWeight","$value":"{pointer.nested.local}"},
          "checkGroup":{"$type":"fontWeight","$value":"{pointer.becomesGroup.child}"},
          "checkToken":{"$type":"fontWeight","$value":"{pointer.becomesToken}"}
        }"##,
    );
    rejects_at(
        r##"{"base":{"n":{"$type":"number","$description":"old","$value":1}},"derived":{"$extends":"{base}","n":{"$type":"number","$value":2}},"probe":{"$type":"fontFamily","$ref":"#/derived/n/$description"}}"##,
        "/probe/$ref",
        "reference",
    );
    rejects_at(
        r##"{"base":{"slot":{"child":{"$type":"number","$value":1}}},"derived":{"$extends":"{base}","slot":{"$type":"number","$value":2}},"probe":{"$type":"number","$ref":"#/derived/slot/child/$value"}}"##,
        "/probe/$ref",
        "reference",
    );
}

#[test]
fn token_value_objects_are_replaced_instead_of_deep_merged() {
    rejects_at(
        r#"{
          "base":{"b":{"$type":"border","$value":{
            "color":{"colorSpace":"srgb","components":[0,0,0]},
            "width":{"value":1,"unit":"px"},"style":"solid"}}},
          "derived":{"$extends":"{base}","b":{"$type":"border","$value":{
            "width":{"value":2,"unit":"px"}}}}
        }"#,
        "/derived/b/$value",
        "value",
    );
}

#[test]
fn local_group_metadata_replaces_inherited_metadata_without_recursive_merging() {
    let prefix = r##"{"base":{"$extensions":{"vendor":{"old":1}}},"derived":{"$extends":"{base}","$extensions":{"vendor":{"new":2}}}"##;
    accepts(&format!(
        r##"{prefix},"probe":{{"$type":"number","$ref":"#/derived/$extensions/vendor/new"}}}}"##
    ));
    rejects_at(
        &format!(
            r##"{prefix},"probe":{{"$type":"number","$ref":"#/derived/$extensions/vendor/old"}}}}"##
        ),
        "/probe/$ref",
        "reference",
    );
}

#[test]
fn ref_only_classification_depends_on_children_target_and_explicit_type() {
    accepts(
        r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$ref":"#/base"},"probe":{"$value":"{derived.n}"}}"##,
    );
    rejects_at(
        r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$type":"color","$ref":"#/base"}}"##,
        "/derived/$ref",
        "value",
    );
    accepts(
        r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$type":"number","$ref":"#/base","local":{"$value":2}},"probe":{"$value":"{derived.local}"}}"##,
    );
    accepts(
        r##"{"base":{"candidate":{"old":{"$type":"number","$value":1}}},"derived":{"$extends":"{base}","candidate":{"$ref":"#/derived/sibling"},"sibling":{"fresh":{"$type":"number","$value":2}}},"probe":{"$value":"{derived.candidate.fresh}"}}"##,
    );
}

#[test]
fn ambiguous_reference_envelopes_are_structure_errors() {
    for input in [
        r##"{"x":{"$type":"number","$value":1,"$ref":"#/missing"}}"##,
        r##"{"x":{"$extends":"{missing}","$ref":"#/also-missing"}}"##,
    ] {
        rejects_at(input, "/x/$ref", "structure");
    }
    rejects_at(
        r##"{"x":{"$type":"number","$value":{"$ref":"#/a","$ref":"#/b"}}}"##,
        "/x/$value/$ref",
        "structure",
    );
}

#[test]
fn missing_malformed_or_wrong_kind_extension_targets_do_not_fall_back() {
    for reference in [
        "{missing}",
        "#/missing",
        "https://example.invalid/tokens",
        "bad",
        "#/x/~2",
    ] {
        rejects_at(
            &json!({"g": {"$extends": reference}}).to_string(),
            "/g/$extends",
            "reference",
        );
    }
    for reference in ["{n}", "#/n"] {
        rejects_at(
            &json!({"n": {"$type": "number", "$value": 1}, "g": {"$extends": reference}})
                .to_string(),
            "/g/$extends",
            "reference",
        );
    }
    rejects_at(r#"{"g":{"$extends":42}}"#, "/g/$extends", "reference");
    rejects_at(
        r##"{"g":{"$ref":"#/missing","child":{}}}"##,
        "/g/$ref",
        "reference",
    );
}

#[test]
fn pointer_escaping_utf8_percent_decoding_and_empty_names_are_end_to_end() {
    accepts(
        r##"{
          "":{"":{"":{"$type":"number","$value":1}}},
          "a/b":{"$type":"number","$value":2},
          "~1":{"$type":"number","$value":3},
          "é🚲":{"$type":"number","$value":4},
          "01":{"$type":"number","$value":5},
          "%2F":{"$type":"number","$value":6},
          "a+b":{"$type":"number","$value":7},
          "with space":{"$type":"number","$value":8},
          "empty":{"$type":"number","$ref":"#////$value"},
          "slash":{"$type":"number","$ref":"#/a~1b/$value"},
          "tilde":{"$type":"number","$ref":"#/~01/$value"},
          "unicode":{"$type":"number","$ref":"#/%C3%A9%F0%9F%9A%B2/$value"},
          "numericKey":{"$type":"number","$ref":"#/01/$value"},
          "decodeOnce":{"$type":"number","$ref":"#/%252F/$value"},
          "plus":{"$type":"number","$ref":"#/a+b/$value"},
          "space":{"$type":"number","$ref":"#/with%20space/$value"},
          "encodedSeparators":{"$type":"number","$ref":"#%2Fa~1b%2F%24value"}
        }"##,
    );
    accepts(r#"{"":{"":{"$type":"number","$value":1}},"x":{"$value":"{.}"}}"#);
    rejects_at(
        r#"{"a/b":{"~key":{"$description":false}}}"#,
        "/a~1b/~0key/$description",
        "metadata",
    );
    rejects_at(
        r#"{"":{"":{"$description":false}}}"#,
        "///$description",
        "metadata",
    );
}

#[test]
fn rfc6901_root_and_empty_member_are_distinct_and_never_implicit_values() {
    // If #/ were incorrectly treated as root, copying it would form a cycle.
    accepts(
        r##"{"":{"n":{"$type":"number","$value":1}},"copy":{"$ref":"#/"},"probe":{"$value":"{copy.n}"}}"##,
    );
    for reference in ["#", "#/"] {
        let report = rejects_at(
            &json!({"": {"$type": "number", "$value": 1}, "x": {"$type": "number", "$ref": reference}}).to_string(),
            "/x/$ref",
            "value",
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "reference")
        );
    }
    rejects_at(
        r##"{"x":{"$type":"number","$ref":"#/"}}"##,
        "/x/$ref",
        "reference",
    );
    rejects_at(
        r##"{"n":{"$type":"number","$value":1},"x":{"$type":"number","$ref":"#/n"}}"##,
        "/x/$ref",
        "value",
    );
    rejects_at(
        r##"{"n":{"$type":"number","$value":1},"alias":{"$type":"number","$ref":"#/n/$value"},"x":{"$type":"number","$ref":"#/alias/$value"}}"##,
        "/x/$ref",
        "reference",
    );
}

#[test]
fn malformed_pointer_fragments_and_array_indices_are_public_reference_errors() {
    for reference in [
        "",
        "/a",
        "https://example.invalid/#/a",
        "#a",
        "#/%",
        "#/%0",
        "#/%GG",
        "#/%FF",
        "#/%C3%28",
        "#/a~",
        "#/a~2",
    ] {
        rejects_at(
            &json!({"x": {"$type": "number", "$ref": reference}}).to_string(),
            "/x/$ref",
            "reference",
        );
    }
    for index in [
        "-",
        "00",
        "01",
        "+0",
        "-0",
        "-1",
        "1",
        "184467440737095516160",
    ] {
        rejects_at(
            &json!({"a": {"$type": "fontFamily", "$value": ["Inter"]}, "x": {
                "$type": "fontFamily", "$ref": format!("#/a/$value/{index}")
            }})
            .to_string(),
            "/x/$ref",
            "reference",
        );
    }
    accepts(
        r##"{"a":{"$type":"fontFamily","$value":["Inter"]},"x":{"$type":"fontFamily","$ref":"#/a/$value/0"},"matrix":{"$type":"number","$ref":"#/$extensions/matrix/0/1"},"$extensions":{"matrix":[[0,2]]}}"##,
    );
}

#[test]
fn pointers_traverse_reference_intermediates_in_the_expanded_document() {
    accepts(
        r##"{
          "base":{"c":{"$type":"color","$value":{"colorSpace":"srgb","components":[0.25,0.5,1]}}},
          "derived":{"$extends":"{base}"},
          "alias":{"$value":"{derived.c}"},
          "wrapper":{"$type":"color","$value":{"$ref":"#/alias/$value"}},
          "component":{"$type":"number","$ref":"#/wrapper/$value/components/0"},
          "bezier":{"$type":"cubicBezier","$value":[{"$ref":"#/wrapper/$value/components/0"},0,1,1]}
        }"##,
    );
}

#[test]
fn metadata_pointer_targets_are_literal_not_implicitly_evaluated() {
    accepts(
        r##"{"a":{"$type":"fontFamily","$description":"{missing}","$value":"Inter","$extensions":{"vendor":{"$ref":"#/missing"}}},"description":{"$type":"fontFamily","$ref":"#/a/$description"},"selfType":{"$type":"fontFamily","$ref":"#/selfType/$type"}}"##,
    );
    let report = rejects_at(
        r##"{"$extensions":{"vendor":{"$ref":"#/missing"}},"x":{"$type":"number","$ref":"#/$extensions/vendor"}}"##,
        "/x/$ref",
        "value",
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "reference")
    );
}

#[test]
fn all_alias_cycle_members_are_reported_but_downstream_dependents_are_not_members() {
    for (input, paths) in [
        (
            r#"{"a":{"$type":"number","$value":"{b}"},"b":{"$type":"number","$value":"{a}"}}"#,
            vec!["/a/$value", "/b/$value"],
        ),
        (
            r##"{"a":{"$type":"number","$value":"{b}"},"b":{"$type":"number","$ref":"#/c/$value"},"c":{"$type":"number","$value":"{a}"},"dependent":{"$type":"number","$value":"{a}"}}"##,
            vec!["/a/$value", "/b/$ref", "/c/$value"],
        ),
        (
            r##"{"a":{"$ref":"#/b"},"b":{"$ref":"#/a"}}"##,
            vec!["/a/$ref", "/b/$ref"],
        ),
    ] {
        let report = rejects(input);
        for path in paths {
            has_diagnostic(&report, path, "cycle");
        }
        assert!(
            report.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "cycle" || !diagnostic.path.starts_with("/dependent")
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn group_extension_cycles_include_direct_and_containment_participants() {
    for (input, paths) in [
        (
            r#"{"a":{"$extends":"{b}"},"b":{"$extends":"{c}"},"c":{"$extends":"{a}"}}"#,
            vec!["/a/$extends", "/b/$extends", "/c/$extends"],
        ),
        (
            r#"{"a":{"b":{"$extends":"{a}"}}}"#,
            vec!["/a", "/a/b/$extends"],
        ),
        (
            r##"{"a":{"middle":{"b":{"$ref":"#/a"}}}}"##,
            vec!["/a", "/a/middle", "/a/middle/b/$ref"],
        ),
    ] {
        let report = rejects(input);
        for path in paths {
            has_diagnostic(&report, path, "cycle");
        }
    }
    accepts(r#"{"a":{"$extends":"{a.b}","b":{"n":{"$type":"number","$value":1}}}}"#);
}

#[test]
fn inherited_errors_use_source_paths_and_do_not_hide_independent_failures() {
    let report = rejects(
        r#"{"base":{"n":{"$type":"number","$value":"{missing}"}},"derived":{"$extends":"{base}"},"a/b":{"$type":"number","$value":"bad"}}"#,
    );
    has_diagnostic(&report, "/base/n/$value", "reference");
    has_diagnostic(&report, "/a~1b/$value", "value");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.path.starts_with("/derived/n"))
    );
    assert!(report.diagnostics.windows(2).all(|pair| {
        (&pair[0].path, &pair[0].code, &pair[0].message)
            <= (&pair[1].path, &pair[1].code, &pair[1].message)
    }));
    assert_eq!(
        report,
        validate(
            r#"{"base":{"n":{"$type":"number","$value":"{missing}"}},"derived":{"$extends":"{base}"},"a/b":{"$type":"number","$value":"bad"}}"#
        )
    );
}

#[test]
fn referenced_composites_reject_extra_fields_while_singular_extras_remain_opaque() {
    let color = json!({"colorSpace": "srgb", "components": [0, "none", 1]});
    let dimension = json!({"value": -1.5, "unit": "rem"});
    let shadow = json!({"color": color, "offsetX": dimension, "offsetY": dimension,
        "blur": dimension, "spread": dimension, "inset": false});
    let cases: [(&str, Value); 6] = [
        ("strokeStyle", json!({"dashArray": [], "lineCap": "round"})),
        (
            "border",
            json!({"color": color, "width": dimension, "style": "solid"}),
        ),
        (
            "transition",
            json!({"duration": {"value": -1, "unit": "s"},
            "delay": {"value": 0, "unit": "ms"}, "timingFunction": [0, -2, 1, 2]}),
        ),
        ("shadow", shadow),
        ("gradient", json!([{"color": color, "position": 42}])),
        (
            "typography",
            json!({"fontFamily": [], "fontSize": dimension,
            "fontWeight": 999.5, "letterSpacing": dimension, "lineHeight": -1}),
        ),
    ];
    for (token_type, mut value) in cases {
        if token_type == "gradient" {
            value[0]["extra"] = json!({"$ref": "#/missing"});
        } else {
            value["extra"] = json!({"$ref": "#/missing"});
        }
        let input = json!({"$extensions": {"data": value}, "alias": {
            "$type": token_type, "$ref": "#/$extensions/data"
        }})
        .to_string();
        let report = rejects_at(&input, "/alias/$ref", "value");
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "reference")
        );
    }
    accepts(
        r##"{"$extensions":{"data":{"value":1,"unit":"px","extra":{"$ref":"#/missing"}}},"alias":{"$type":"dimension","$ref":"#/$extensions/data"}}"##,
    );
}
