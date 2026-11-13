use crate::{ValidationReport, validate};

const COLOR: &str = r#"{"colorSpace":"srgb","components":[0,0.5,1]}"#;
const DIMENSION: &str = r#"{"value":2,"unit":"px"}"#;
const DURATION: &str = r#"{"value":-0.5,"unit":"s"}"#;

fn document(name: &str, value: &str) -> String {
    format!(r#"{{"x":{{"$type":"{name}","$value":{value}}}}}"#)
}

fn accepts(input: &str) {
    let report = validate(input);
    assert!(report.valid, "{input}: {:?}", report.diagnostics);
}

fn rejects(input: &str) -> ValidationReport {
    let report = validate(input);
    assert!(!report.valid, "expected invalid: {input}");
    report
}

fn rejects_at(input: &str, path: &str, code: &str) {
    let report = rejects(input);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.path == path && diagnostic.code == code }),
        "{input}: expected {code} at {path}, got {:?}",
        report.diagnostics
    );
}

fn shadow() -> String {
    format!(
        r#"{{"color":{COLOR},"offsetX":{DIMENSION},"offsetY":{DIMENSION},"blur":{DIMENSION},"spread":{DIMENSION}}}"#
    )
}

#[test]
fn validates_every_primitive_and_composite_type() {
    for (name, value) in [
        ("number", "1e1000".to_owned()),
        ("dimension", DIMENSION.to_owned()),
        ("duration", DURATION.to_owned()),
        ("fontFamily", r#"["Inter","sans-serif"]"#.to_owned()),
        ("fontWeight", "999.5".to_owned()),
        ("cubicBezier", "[0,-2,1,1e1000]".to_owned()),
        ("color", COLOR.to_owned()),
        (
            "strokeStyle",
            format!(r#"{{"dashArray":[{DIMENSION}],"lineCap":"round"}}"#),
        ),
        (
            "border",
            format!(r#"{{"color":{COLOR},"width":{DIMENSION},"style":"solid"}}"#),
        ),
        (
            "transition",
            format!(r#"{{"duration":{DURATION},"delay":{DURATION},"timingFunction":[0,0,1,1]}}"#),
        ),
        ("shadow", shadow()),
        (
            "gradient",
            format!(r#"[{{"color":{COLOR},"position":42}}]"#),
        ),
        (
            "typography",
            format!(
                r#"{{"fontFamily":"Inter","fontSize":{DIMENSION},"fontWeight":"bold","letterSpacing":{DIMENSION},"lineHeight":1.5}}"#
            ),
        ),
    ] {
        accepts(&document(name, &value));
        rejects_at(&document(name, "null"), "/x/$value", "value");
    }
}

#[test]
fn numbers_and_bounded_values_never_pass_through_floating_point() {
    for number in [
        "-1e1000",
        "1e1000",
        "0",
        "-0",
        "1e9999999999999999999999999999999999999999999999999999999999",
        "1e-9999999999999999999999999999999999999999999999999999999999",
    ] {
        accepts(&document("number", number));
    }
    for number in ["1", "1000", "999.5", "1e3", "1000.0000000000000000"] {
        accepts(&document("fontWeight", number));
    }
    for number in [
        "0.9999999999999999999999999999999",
        "1000.0000000000000001",
        "-1",
        "1e1000",
        "1e9999999999999999999999999999999999999999999999999999999999",
        "1e-9999999999999999999999999999999999999999999999999999999999",
    ] {
        rejects_at(&document("fontWeight", number), "/x/$value", "value");
    }
    for value in ["true", "false", "null", "\"1\"", "[]", "{}"] {
        rejects_at(&document("number", value), "/x/$value", "value");
    }
}

#[test]
fn quantity_units_are_exact_and_values_may_be_negative_or_fractional() {
    for (name, units) in [("dimension", ["px", "rem"]), ("duration", ["ms", "s"])] {
        for unit in units {
            accepts(&document(
                name,
                &format!(r#"{{"value":-0.25,"unit":"{unit}"}}"#),
            ));
        }
        for unit in ["PX", "em", "seconds", "", "percent"] {
            rejects_at(
                &document(name, &format!(r#"{{"value":1,"unit":"{unit}"}}"#)),
                "/x/$value/unit",
                "value",
            );
        }
        for value in ["{}", r#"{"value":1}"#, r#"{"unit":"px"}"#, "1", "[]"] {
            rejects(&document(name, value));
        }
        rejects_at(
            &document(name, r#"{"value":"1","unit":"px"}"#),
            "/x/$value/value",
            "value",
        );
    }
}

#[test]
fn font_family_arrays_are_literal_strings_and_accept_empty_arrays() {
    for value in [
        r#""Inter""#,
        r#""""#,
        "[]",
        r#"["Inter","{missing}","sans-serif"]"#,
    ] {
        accepts(&document("fontFamily", value));
    }
    for value in ["[1]", "[[]]", "[{}]", "[false]", "[null]"] {
        rejects_at(&document("fontFamily", value), "/x/$value/0", "value");
    }
}

#[test]
fn accepts_only_published_font_weight_names() {
    for weight in super::FONT_WEIGHTS {
        accepts(&document("fontWeight", &format!("\"{weight}\"")));
    }
    for weight in [
        "Bold", "bolder", "lighter", "semibold", "400", "", "normal ",
    ] {
        rejects_at(
            &document("fontWeight", &format!("\"{weight}\"")),
            "/x/$value",
            "value",
        );
    }
}

#[test]
fn bezier_x_coordinates_are_exactly_bounded_but_y_values_are_unbounded() {
    for value in ["[0,-1e1000,1,1e1000]", "[0.5,0,0.5,0]", "[-0,0,1.000,0]"] {
        accepts(&document("cubicBezier", value));
    }
    for value in [
        "[]",
        "[0,0,1]",
        "[0,0,1,1,2]",
        "[-0.00000000000000000001,0,1,0]",
        "[0,0,1.0000000000000000001,0]",
        "[0,\"{missing}\",1,0]",
    ] {
        let report = rejects(&document("cubicBezier", value));
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "reference")
        );
    }
}

#[test]
fn all_rgb_and_xyz_spaces_have_exact_unit_component_bounds() {
    for space in [
        "srgb",
        "srgb-linear",
        "display-p3",
        "a98-rgb",
        "prophoto-rgb",
        "rec2020",
        "xyz-d65",
        "xyz-d50",
    ] {
        for components in ["[0,0.5,1]", r#"["none","none","none"]"#] {
            accepts(&document(
                "color",
                &format!(r#"{{"colorSpace":"{space}","components":{components}}}"#),
            ));
        }
        for index in 0..3 {
            for invalid in ["-0.0000000000000000001", "1.0000000000000000001", "1e1000"] {
                let mut components = ["0", "0", "0"];
                components[index] = invalid;
                rejects_at(
                    &document(
                        "color",
                        &format!(
                            r#"{{"colorSpace":"{space}","components":[{}]}}"#,
                            components.join(",")
                        ),
                    ),
                    &format!("/x/$value/components/{index}"),
                    "value",
                );
            }
        }
    }
}

#[test]
fn hsl_and_hwb_use_open_hue_and_percentage_ranges() {
    for space in ["hsl", "hwb"] {
        for components in [
            "[0,0,100]",
            "[359.9999999999999999999,100,0]",
            r#"["none","none","none"]"#,
        ] {
            accepts(&document(
                "color",
                &format!(r#"{{"colorSpace":"{space}","components":{components}}}"#),
            ));
        }
        for components in [
            "[360,0,0]",
            "[-1,0,0]",
            "[0,-1,0]",
            "[0,0,100.0000000000000001]",
            "[0,101,0]",
        ] {
            rejects(&document(
                "color",
                &format!(r#"{{"colorSpace":"{space}","components":{components}}}"#),
            ));
        }
    }
}

#[test]
fn lab_and_oklab_do_not_impose_practical_gamut_bounds() {
    for (space, maximum) in [("lab", "100"), ("oklab", "1")] {
        for lightness in ["0", maximum] {
            accepts(&document(
                "color",
                &format!(r#"{{"colorSpace":"{space}","components":[{lightness},-1e1000,1e1000]}}"#),
            ));
        }
        accepts(&document(
            "color",
            &format!(r#"{{"colorSpace":"{space}","components":["none","none","none"]}}"#),
        ));
        for lightness in [
            "-0.00000000000000001",
            if space == "lab" {
                "100.0000000000000001"
            } else {
                "1.0000000000000001"
            },
        ] {
            rejects_at(
                &document(
                    "color",
                    &format!(r#"{{"colorSpace":"{space}","components":[{lightness},0,0]}}"#),
                ),
                "/x/$value/components/0",
                "value",
            );
        }
    }
}

#[test]
fn lch_and_oklch_enforce_nonnegative_chroma_and_open_hue() {
    for (space, maximum) in [("lch", "100"), ("oklch", "1")] {
        for components in [
            format!("[{maximum},1e1000,359.99999999999999999999]"),
            "[0,0,0]".to_owned(),
            r#"["none","none","none"]"#.to_owned(),
        ] {
            accepts(&document(
                "color",
                &format!(r#"{{"colorSpace":"{space}","components":{components}}}"#),
            ));
        }
        for components in [
            "[-1,0,0]",
            "[0,-1e-1000,0]",
            "[0,0,-1]",
            "[0,0,360]",
            if space == "lch" {
                "[100.0000000000000001,0,0]"
            } else {
                "[1.0000000000000001,0,0]"
            },
        ] {
            rejects(&document(
                "color",
                &format!(r#"{{"colorSpace":"{space}","components":{components}}}"#),
            ));
        }
    }
}

#[test]
fn color_shape_alpha_and_hex_are_exact() {
    accepts(&document(
        "color",
        r##"{"colorSpace":"srgb","components":[0,"none",1],"alpha":0,"hex":"#aB09fF"}"##,
    ));
    accepts(&document(
        "color",
        r##"{"colorSpace":"srgb","components":[0,0,1],"alpha":1,"hex":"#000000"}"##,
    ));
    for value in [
        r#"{"colorSpace":"rgb","components":[0,0,0]}"#,
        r#"{"colorSpace":"SRGB","components":[0,0,0]}"#,
        r#"{"colorSpace":"srgb","components":[]}"#,
        r#"{"colorSpace":"srgb","components":[0,0,0,0]}"#,
        r#"{"colorSpace":"srgb","components":[0,"None",0]}"#,
        r#"{"colorSpace":"srgb","components":[0,0,0],"alpha":"none"}"#,
        r#"{"colorSpace":"srgb","components":[0,0,0],"alpha":1.0000000000000001}"#,
        r#"{"colorSpace":"srgb","components":[0,0,0],"alpha":-1e-1000}"#,
        r##"{"colorSpace":"srgb","components":[0,0,0],"hex":"#fff"}"##,
        r##"{"colorSpace":"srgb","components":[0,0,0],"hex":"#000000ff"}"##,
        r##"{"colorSpace":"srgb","components":[0,0,0],"hex":"#gg0000"}"##,
        r#"{"colorSpace":"srgb","components":[0,0,0],"hex":"000000"}"#,
        r#"{"components":[0,0,0]}"#,
        r#"{"colorSpace":"srgb"}"#,
    ] {
        rejects(&document("color", value));
    }
}

#[test]
fn stroke_style_names_caps_and_dash_arrays_follow_the_report() {
    for style in super::STROKE_STYLES {
        accepts(&document("strokeStyle", &format!("\"{style}\"")));
    }
    for cap in ["round", "butt", "square"] {
        for dash in [
            "[]".to_owned(),
            format!("[{DIMENSION}]"),
            format!("[{DIMENSION},{DIMENSION},{DIMENSION}]"),
        ] {
            accepts(&document(
                "strokeStyle",
                &format!(r#"{{"dashArray":{dash},"lineCap":"{cap}"}}"#),
            ));
        }
    }
    for value in [
        r#""none""#,
        r#""Solid""#,
        r#"{"dashArray":[],"lineCap":"flat"}"#,
        r#"{"dashArray":[1],"lineCap":"round"}"#,
        r#"{"dashArray":{},"lineCap":"round"}"#,
        r#"{"dashArray":[]}"#,
    ] {
        rejects(&document("strokeStyle", value));
    }
}

#[test]
fn composite_fields_are_required_and_unknown_subvalues_are_rejected() {
    let composites = [
        (
            "strokeStyle",
            r#"{"dashArray":[],"lineCap":"round"}"#.to_owned(),
        ),
        (
            "border",
            format!(r#"{{"color":{COLOR},"width":{DIMENSION},"style":"solid"}}"#),
        ),
        (
            "transition",
            format!(r#"{{"duration":{DURATION},"delay":{DURATION},"timingFunction":[0,0,1,1]}}"#),
        ),
        ("shadow", shadow()),
        (
            "typography",
            format!(
                r#"{{"fontFamily":"Inter","fontSize":{DIMENSION},"fontWeight":400,"letterSpacing":{DIMENSION},"lineHeight":1}}"#
            ),
        ),
    ];
    for (name, value) in composites {
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        for key in value.as_object().unwrap().keys() {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            rejects(&document(name, &missing.to_string()));
            let mut wrong = value.clone();
            wrong[key] = serde_json::Value::Null;
            rejects(&document(name, &wrong.to_string()));
        }
        let mut extra = value;
        extra["unexpected"] = serde_json::json!({"$ref":"#/missing"});
        let report = rejects(&document(name, &extra.to_string()));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "value"
                    && diagnostic.path == "/x/$value/unexpected")
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "reference")
        );
    }
    rejects_at(
        &document(
            "gradient",
            &format!(r#"[{{"color":{COLOR},"position":0,"extra":1}}]"#),
        ),
        "/x/$value/0/extra",
        "value",
    );
    rejects(&document("gradient", &format!(r#"[{{"color":{COLOR}}}]"#)));
    rejects(&document("gradient", "[{\"position\":0}]"));
}

#[test]
fn singular_value_extra_members_and_all_metadata_stay_opaque() {
    accepts(
        r##"{
      "$custom":"{missing}","$extensions":{"vendor":{"$ref":"#/missing"}},
      "d":{"$type":"dimension","$description":"{missing}","$value":{"value":1,"unit":"px","extra":{"$ref":"#/missing"},"$ref":"#/missing"}},
      "t":{"$type":"duration","$value":{"value":1,"unit":"s","extra":"{missing}"}},
      "c":{"$type":"color","$value":{"colorSpace":"srgb","components":[0,0,0],"extra":{"$ref":"#/missing"}}}
    }"##,
    );
}

#[test]
fn shadows_allow_optional_boolean_inset_and_unflattened_arrays() {
    let single = shadow();
    accepts(&document("shadow", &single));
    accepts(&document("shadow", &format!("[{single},{single}]")));
    accepts(&document("shadow", "[]"));
    for inset in ["true", "false"] {
        accepts(&document(
            "shadow",
            &format!("{},\"inset\":{inset}}}", &single[..single.len() - 1]),
        ));
    }
    for inset in ["0", "null", "\"true\"", "[]", "{}"] {
        rejects_at(
            &document(
                "shadow",
                &format!("{},\"inset\":{inset}}}", &single[..single.len() - 1]),
            ),
            "/x/$value/inset",
            "value",
        );
    }
    rejects_at(&document("shadow", "[[]]"), "/x/$value/0", "value");
}

#[test]
fn gradient_positions_are_not_rejected_instead_of_clamped() {
    for position in ["-1e1000", "0", "1", "42", "1e1000"] {
        accepts(&document(
            "gradient",
            &format!(r#"[{{"color":{COLOR},"position":{position}}}]"#),
        ));
    }
    accepts(&document("gradient", "[]"));
    for value in ["[[]]", "[1]", "{}", "[{}]"] {
        rejects(&document("gradient", value));
    }
}

#[test]
fn type_precedence_is_explicit_then_alias_then_effective_group_default() {
    accepts(r#"{"g":{"$type":"color","n":{"$type":"number","$value":1}},"x":{"$value":"{g.n}"}}"#);
    accepts(r#"{"n":{"$type":"number","$value":1},"g":{"$type":"color","x":{"$value":"{n}"}}}"#);
    accepts(r#"{"g":{"$type":"number","$root":{"$value":1}},"x":{"$value":"{g.$root}"}}"#);
    accepts(
        r#"{"base":{"$type":"number","n":{"$value":400}},"derived":{"$extends":"{base}","$type":"fontWeight"}}"#,
    );
    rejects_at(
        r#"{"base":{"$type":"number","n":{"$value":1001}},"derived":{"$extends":"{base}","$type":"fontWeight"}}"#,
        "/base/n/$value",
        "value",
    );
    rejects_at(
        r#"{"named-like-number":{"$value":1}}"#,
        "/named-like-number",
        "type",
    );
    rejects_at(
        r#"{"g":{"$type":"unknown","x":{"$value":1}}}"#,
        "/g/$type",
        "type",
    );
    rejects_at(
        r#"{"x":{"$type":"unknown","$value":1}}"#,
        "/x/$type",
        "type",
    );
}

#[test]
fn curly_aliases_require_identical_types_even_when_json_shapes_match() {
    rejects_at(
        r#"{"n":{"$type":"number","$value":400},"w":{"$type":"fontWeight","$value":"{n}"}}"#,
        "/w/$value",
        "reference",
    );
    accepts(
        r##"{"n":{"$type":"number","$value":400},"w":{"$type":"fontWeight","$ref":"#/n/$value"}}"##,
    );
    accepts(
        r##"{"n":{"$type":"number","$value":400},"w":{"$type":"fontWeight","$value":{"$ref":"#/n/$value"}}}"##,
    );
    let input = format!(
        r#"{{"n":{{"$type":"fontWeight","$value":1}},"t":{{"$type":"typography","$value":{{"fontFamily":"Inter","fontSize":{DIMENSION},"fontWeight":400,"letterSpacing":{DIMENSION},"lineHeight":"{{n}}"}}}}}}"#
    );
    rejects_at(&input, "/t/$value/lineHeight", "reference");
    let input = format!(
        r#"{{"d":{{"$type":"duration","$value":{DURATION}}},"b":{{"$type":"border","$value":{{"color":{COLOR},"width":"{{d}}","style":"solid"}}}}}}"#
    );
    rejects_at(&input, "/b/$value/width", "reference");
}

#[test]
fn pointers_work_at_every_primitive_inner_value_position() {
    accepts(
        r##"{
      "$extensions":{"data":{"n":0.5,"zero":0,"one":1,"px":"px","s":"s","family":"{literal}","space":"srgb","none":"none","hex":"#aB1234","cap":"round","inset":true}},
      "d":{"$type":"dimension","$value":{"value":{"$ref":"#/$extensions/data/n"},"unit":{"$ref":"#/$extensions/data/px"}}},
      "t":{"$type":"duration","$value":{"value":{"$ref":"#/$extensions/data/n"},"unit":{"$ref":"#/$extensions/data/s"}}},
      "f":{"$type":"fontFamily","$value":[{"$ref":"#/$extensions/data/family"}]},
      "b":{"$type":"cubicBezier","$value":[{"$ref":"#/$extensions/data/zero"},{"$ref":"#/$extensions/data/n"},{"$ref":"#/$extensions/data/one"},{"$ref":"#/$extensions/data/n"}]},
      "c":{"$type":"color","$value":{"colorSpace":{"$ref":"#/$extensions/data/space"},"components":[{"$ref":"#/$extensions/data/zero"},{"$ref":"#/$extensions/data/none"},{"$ref":"#/$extensions/data/one"}],"alpha":{"$ref":"#/$extensions/data/n"},"hex":{"$ref":"#/$extensions/data/hex"}}},
      "stroke":{"$type":"strokeStyle","$value":{"dashArray":["{d}"],"lineCap":{"$ref":"#/$extensions/data/cap"}}},
      "shadow":{"$type":"shadow","$value":{"color":"{c}","offsetX":"{d}","offsetY":"{d}","blur":"{d}","spread":"{d}","inset":{"$ref":"#/$extensions/data/inset"}}}
    }"##,
    );
    accepts(
        r##"{"f":{"$type":"fontFamily","$value":["{missing}"]},"literal":{"$type":"fontFamily","$ref":"#/f/$value/0"},"metadata":{"$type":"fontFamily","$ref":"#/f/$type"}}"##,
    );
}

#[test]
fn pointer_arrays_and_mixed_composite_references_are_not_flattened() {
    let input = format!(
        r##"{{"c":{{"$type":"color","$value":{COLOR}}},"d":{{"$type":"dimension","$value":{DIMENSION}}},"s":{{"$type":"shadow","$value":{}}},"g":{{"$type":"gradient","$value":[{{"color":"{{c}}","position":0}}]}},"shadows":{{"$type":"shadow","$value":["{{s}}",{{"$ref":"#/s/$value"}}]}},"stops":{{"$type":"gradient","$value":[{{"$ref":"#/g/$value/0"}},{{"color":{{"$ref":"#/c/$value"}},"position":42}}]}}}}"##,
        shadow()
    );
    accepts(&input);
    for (name, reference) in [("shadow", "{s}"), ("gradient", "{g}")] {
        let base = format!(
            r#"{{"s":{{"$type":"shadow","$value":[]}},"g":{{"$type":"gradient","$value":[]}},"x":{{"$type":"{name}","$value":["{reference}"]}}}}"#
        );
        rejects_at(&base, "/x/$value/0", "value");
        let pointer = format!(
            r##"{{"base":{{"$type":"{name}","$value":[]}},"x":{{"$type":"{name}","$value":[{{"$ref":"#/base/$value"}}]}}}}"##
        );
        rejects_at(&pointer, "/x/$value/0/$ref", "value");
    }
    accepts(
        r##"{"$extensions":{"data":{"components":[0,0,1],"families":["Inter"],"bezier":[0,0,1,1],"dashes":[]}},"c":{"$type":"color","$value":{"colorSpace":"srgb","components":{"$ref":"#/$extensions/data/components"}}},"f":{"$type":"fontFamily","$value":{"$ref":"#/$extensions/data/families"}},"b":{"$type":"cubicBezier","$value":{"$ref":"#/$extensions/data/bezier"}},"s":{"$type":"strokeStyle","$value":{"dashArray":{"$ref":"#/$extensions/data/dashes"},"lineCap":"butt"}}}"##,
    );
}

#[test]
fn primitive_inner_curly_strings_are_invalid_literals_not_failed_references() {
    for (name, value, path) in [
        (
            "dimension",
            r#"{"value":"{missing}","unit":"px"}"#,
            "/x/$value/value",
        ),
        (
            "dimension",
            r#"{"value":1,"unit":"{missing}"}"#,
            "/x/$value/unit",
        ),
        (
            "duration",
            r#"{"value":"{missing}","unit":"s"}"#,
            "/x/$value/value",
        ),
        (
            "duration",
            r#"{"value":1,"unit":"{missing}"}"#,
            "/x/$value/unit",
        ),
        (
            "color",
            r#"{"colorSpace":"{missing}","components":[0,0,0]}"#,
            "/x/$value/colorSpace",
        ),
        (
            "color",
            r#"{"colorSpace":"srgb","components":["{missing}",0,0]}"#,
            "/x/$value/components/0",
        ),
        (
            "color",
            r#"{"colorSpace":"srgb","components":[0,0,0],"alpha":"{missing}"}"#,
            "/x/$value/alpha",
        ),
        ("cubicBezier", r#"[0,"{missing}",1,0]"#, "/x/$value/1"),
    ] {
        let report = rejects(&document(name, value));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.path == path && diagnostic.code == "value"),
            "{:?}",
            report.diagnostics
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "reference"),
            "{:?}",
            report.diagnostics
        );
    }
}

#[test]
fn incompatible_pointer_targets_are_diagnosed_at_original_aliases() {
    rejects_at(
        r##"{"n":{"$type":"number","$value":42},"c":{"$type":"color","$ref":"#/n/$value"}}"##,
        "/c/$ref",
        "value",
    );
    rejects_at(
        r##"{"base":{"n":{"$type":"number","$value":1}},"derived":{"$type":"color","$ref":"#/base"}}"##,
        "/derived/$ref",
        "value",
    );
    rejects_at(
        r##"{"n":{"$type":"fontFamily","$value":"bad"},"x":{"$type":"number","$value":{"$ref":"#/n/$value"}}}"##,
        "/x/$value",
        "value",
    );
    rejects_at(
        r##"{"$extensions":{"n":"bad"},"x":{"$type":"dimension","$value":{"value":{"$ref":"#/$extensions/n"},"unit":"px"}}}"##,
        "/x/$value/value/$ref",
        "value",
    );
    rejects_at(
        r##"{"n":{"$type":"number","$value":1},"x":{"$type":"number","$ref":"#/n"}}"##,
        "/x/$ref",
        "value",
    );
}

#[test]
fn invalid_reference_branches_do_not_hide_independent_value_errors() {
    let report = rejects(
        r##"{"x":{"$type":"dimension","$value":{"value":{"$ref":"#/missing"},"unit":"bad"}}}"##,
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == "/x/$value/value/$ref"
                && diagnostic.code == "reference")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == "/x/$value/unit" && diagnostic.code == "value")
    );
    let report = rejects(
        r##"{"a":{"$type":"gradient","$value":[{"$ref":"#/b/$value/0"}]},"b":{"$type":"gradient","$value":[{"$ref":"#/a/$value/0"}]}}"##,
    );
    for prefix in ["/a/$value", "/b/$value"] {
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic.path.starts_with(prefix) && diagnostic.code == "cycle"), "{:?}", report.diagnostics);
    }
}
