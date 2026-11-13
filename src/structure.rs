use crate::Diagnostic;
use crate::json::Value;

pub(crate) const SUPPORTED_TYPES: [&str; 13] = [
    "number",
    "dimension",
    "duration",
    "fontFamily",
    "fontWeight",
    "cubicBezier",
    "color",
    "strokeStyle",
    "border",
    "transition",
    "shadow",
    "gradient",
    "typography",
];

const PROPERTIES: [&str; 8] = [
    "$value",
    "$type",
    "$description",
    "$deprecated",
    "$extensions",
    "$ref",
    "$extends",
    "$root",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Document,
    Child,
    RootToken,
}

/// Inspects the original AST without merging definitions or resolving values.
/// Duplicate child names remain separate definitions for later pointer lookup.
pub(crate) fn validate(document: &Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    match document {
        Value::Object(members) => {
            validate_object(members, "", Context::Document, &mut diagnostics);
        }
        _ => report(
            &mut diagnostics,
            "".to_owned(),
            "structure",
            "The document root must be an object.",
        ),
    }
    diagnostics
}

fn validate_object(
    members: &[(String, Value)],
    path: &str,
    context: Context,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let has = |property| members.iter().any(|(name, _)| name == property);
    let has_value = has("$value");
    let has_ref = has("$ref");
    let has_children = members
        .iter()
        .any(|(name, _)| !name.starts_with('$') || name == "$root");
    // A childless, explicitly typed $ref is a token alias. With children it is
    // a group extension; without either clue its target decides in the resolver.
    let is_token = has_value || (has_ref && has("$type") && !has_children);

    if context == Context::Document && has_value {
        report(
            diagnostics,
            child_path(path, "$value"),
            "structure",
            "The document root must be a group and cannot contain $value.",
        );
    }
    if context == Context::RootToken && !has_value && (!has_ref || has_children) {
        report(
            diagnostics,
            path.to_owned(),
            "structure",
            "$root must be a token, not a group.",
        );
    }
    if has_ref && has_value {
        report(
            diagnostics,
            child_path(path, "$ref"),
            "structure",
            "An object cannot contain both $ref and $value.",
        );
    }
    if has_ref && has("$extends") {
        report(
            diagnostics,
            child_path(path, "$ref"),
            "structure",
            "An object cannot contain both $ref and $extends.",
        );
    }
    if has_value && has("$extends") {
        report(
            diagnostics,
            child_path(path, "$extends"),
            "structure",
            "$extends is only allowed on groups.",
        );
    }

    for property in PROPERTIES {
        let mut values = members
            .iter()
            .filter_map(|(name, value)| (name == property).then_some(value));
        let Some(value) = values.next() else {
            continue;
        };
        if values.any(|other| other != value) {
            report(
                diagnostics,
                child_path(path, property),
                "structure",
                &format!("Conflicting duplicate {property} properties are ambiguous."),
            );
            // Do not validate one arbitrarily selected metadata value.
            continue;
        }
        validate_metadata(property, value, path, diagnostics);
    }

    for (name, value) in members {
        if name.starts_with('$') && name != "$root" {
            if !PROPERTIES.contains(&name.as_str()) && looks_like_definition(value) {
                report(
                    diagnostics,
                    child_path(path, name),
                    "structure",
                    "Token and group names must not begin with $.",
                );
            }
            // Known metadata and unknown metadata that is not a forbidden
            // definition are opaque here, including all $extensions contents.
            continue;
        }

        let child_path = child_path(path, name);
        if name.contains(['{', '}', '.']) {
            report(
                diagnostics,
                child_path.clone(),
                "structure",
                "Token and group names must not contain {, } or . characters.",
            );
        }
        if is_token {
            report(
                diagnostics,
                child_path.clone(),
                "structure",
                "A token cannot contain child tokens or groups.",
            );
        }
        match value {
            Value::Object(children) => validate_object(
                children,
                &child_path,
                if name == "$root" {
                    Context::RootToken
                } else {
                    Context::Child
                },
                diagnostics,
            ),
            _ => report(
                diagnostics,
                child_path,
                "structure",
                "A child token or group must be an object.",
            ),
        }
    }
}

fn validate_metadata(property: &str, value: &Value, path: &str, diagnostics: &mut Vec<Diagnostic>) {
    let message = match property {
        "$description" if !matches!(value, Value::String(_)) => "$description must be a string.",
        "$type" => match value {
            Value::String(name) if SUPPORTED_TYPES.contains(&name.as_str()) => return,
            Value::String(_) => {
                report(
                    diagnostics,
                    child_path(path, property),
                    "type",
                    "$type must name a supported token type.",
                );
                return;
            }
            _ => "$type must be a string.",
        },
        "$deprecated" if !matches!(value, Value::Bool(_) | Value::String(_)) => {
            "$deprecated must be a boolean or a string."
        }
        "$extensions" if !matches!(value, Value::Object(_)) => "$extensions must be an object.",
        _ => return,
    };
    report(diagnostics, child_path(path, property), "metadata", message);
}

fn looks_like_definition(value: &Value) -> bool {
    let Value::Object(members) = value else {
        return false;
    };
    members.iter().any(|(name, value)| {
        matches!(name.as_str(), "$value" | "$ref" | "$extends")
            || (!name.starts_with('$') && matches!(value, Value::Object(_)))
    })
}

pub(crate) fn child_path(parent: &str, name: &str) -> String {
    let mut path = String::with_capacity(parent.len() + name.len() + 1);
    path.push_str(parent);
    path.push('/');
    for character in name.chars() {
        match character {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            _ => path.push(character),
        }
    }
    path
}

fn report(diagnostics: &mut Vec<Diagnostic>, path: String, code: &str, message: &str) {
    diagnostics.push(Diagnostic {
        path,
        code: code.to_owned(),
        message: message.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::{SUPPORTED_TYPES, validate};
    use crate::{ValidationReport, json};

    // Exercise only this stage: missing types, reference targets and value
    // compatibility belong to the subsequent resolver and value validator.
    fn check(input: &str) -> ValidationReport {
        ValidationReport::from_diagnostics(validate(&json::parse(input).unwrap()))
    }

    fn accepts(input: &str) {
        let report = check(input);
        assert!(report.valid, "{input}: {:?}", report.diagnostics);
    }

    fn rejects_at(input: &str, path: &str, code: &str) {
        let report = check(input);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.path == path && diagnostic.code == code),
            "{input}: expected {code} at {path:?}, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn requires_an_object_document_without_a_root_value() {
        for input in ["null", "false", "42", "\"tokens\"", "[]", "[{}]"] {
            rejects_at(input, "", "structure");
        }
        for value in ["null", "1", "{}", "[]", "\"{other}\""] {
            rejects_at(&format!(r#"{{"$value":{value}}}"#), "/$value", "structure");
        }
        accepts("{}");
        accepts(r#"{"empty":{},"outer":{"inner":{}}}"#);
    }

    #[test]
    fn requires_object_children_at_every_group_depth() {
        for value in ["null", "false", "1", "\"token\"", "[]"] {
            rejects_at(&format!(r#"{{"child":{value}}}"#), "/child", "structure");
            rejects_at(
                &format!(r#"{{"group":{{"child":{value}}}}}"#),
                "/group/child",
                "structure",
            );
        }
    }

    #[test]
    fn tokens_cannot_also_contain_children_or_extend_groups() {
        for child in [r#"{"$value":2}"#, "{}", "3"] {
            rejects_at(
                &format!(r#"{{"token":{{"$value":1,"child":{child}}}}}"#),
                "/token/child",
                "structure",
            );
        }
        rejects_at(
            r#"{"token":{"$value":1,"$root":{"$value":2}}}"#,
            "/token/$root",
            "structure",
        );
        rejects_at(
            r#"{"token":{"$value":1,"$extends":"{base}"}}"#,
            "/token/$extends",
            "structure",
        );
    }

    #[test]
    fn root_children_must_be_tokens_but_may_be_aliases() {
        accepts(r#"{"$root":{"$value":1},"g":{"$root":{"$value":2}}}"#);
        accepts(r##"{"g":{"$root":{"$ref":"#/base"}}}"##);
        accepts(r##"{"g":{"$root":{"$type":"number","$ref":"#/n/$value"}}}"##);
        for value in ["1", "[]", "{}", r#"{"$type":"number"}"#, r#"{"child":{}}"#] {
            rejects_at(
                &format!(r#"{{"g":{{"$root":{value}}}}}"#),
                "/g/$root",
                "structure",
            );
        }
        rejects_at(
            r##"{"g":{"$root":{"$ref":"#/base","child":{}}}}"##,
            "/g/$root",
            "structure",
        );
    }

    #[test]
    fn forbids_reference_envelopes_with_competing_definitions() {
        rejects_at(
            r##"{"token":{"$value":1,"$ref":"#/n/$value"}}"##,
            "/token/$ref",
            "structure",
        );
        rejects_at(
            r##"{"group":{"$extends":"{base}","$ref":"#/base"}}"##,
            "/group/$ref",
            "structure",
        );
    }

    #[test]
    fn defers_target_dependent_reference_classification() {
        accepts(r##"{"alias":{"$ref":"#/unknown"}}"##);
        accepts(r##"{"alias":{"$type":"number","$ref":"#/unknown"}}"##);
        accepts(r##"{"derived":{"$ref":"#/base","n":{"$value":1}}}"##);
        accepts(r##"{"derived":{"$type":"number","$ref":"#/base","n":{"$value":1}}}"##);
        accepts(r#"{"derived":{"$extends":"{base}","n":{"$value":1}}}"#);
    }

    #[test]
    fn restricts_names_for_both_tokens_and_groups() {
        for name in ["a.b", "a{b", "a}b", ".", "{", "}"] {
            for definition in ["{}", r#"{"$value":1}"#] {
                rejects_at(
                    &format!(r#"{{"{name}":{definition}}}"#),
                    &format!("/{name}"),
                    "structure",
                );
            }
        }
        accepts(r#"{"":{"$value":1},"a/b":{"$value":2},"~":{"$value":3},"a$b":{},"é🚲":{}}"#);
    }

    #[test]
    fn rejects_unknown_dollar_prefixed_definitions() {
        for definition in [
            r#"{"$value":1}"#,
            r##"{"$ref":"#/target"}"##,
            r#"{"$extends":"{base}"}"#,
            r#"{"child":{}}"#,
            r#"{"child":{"$value":1}}"#,
        ] {
            rejects_at(&format!(r#"{{"$bad":{definition}}}"#), "/$bad", "structure");
            rejects_at(
                &format!(r#"{{"token":{{"$value":1,"$bad":{definition}}}}}"#),
                "/token/$bad",
                "structure",
            );
        }
    }

    #[test]
    fn preserves_opaque_unknown_metadata() {
        for value in [
            "null",
            "false",
            "42",
            "\"{missing}\"",
            "[]",
            r#"[{"$value":1}]"#,
            "{}",
            r#"{"$type":42,"$description":false,"$extensions":null}"#,
            r#"{"label":"text","count":1}"#,
        ] {
            accepts(&format!(r#"{{"$custom":{value}}}"#));
            accepts(&format!(r#"{{"token":{{"$value":1,"$custom":{value}}}}}"#));
        }
        accepts(
            r#"{"$schema":"https://example.com/tokens.schema.json","$vendor.with{punctuation}":true}"#,
        );
    }

    #[test]
    fn validates_metadata_shapes_on_both_tokens_and_groups() {
        for (property, value) in [
            ("$description", "null"),
            ("$description", "1"),
            ("$description", "{}"),
            ("$type", "null"),
            ("$type", "1"),
            ("$type", "[]"),
            ("$deprecated", "null"),
            ("$deprecated", "1"),
            ("$deprecated", "{}"),
            ("$extensions", "null"),
            ("$extensions", "false"),
            ("$extensions", "[]"),
            ("$extensions", "\"vendor\""),
        ] {
            for token_property in ["", r#""$value":1,"#] {
                rejects_at(
                    &format!(r#"{{"entry":{{{token_property}"{property}":{value}}}}}"#),
                    &format!("/entry/{property}"),
                    "metadata",
                );
            }
        }
    }

    #[test]
    fn permits_all_supported_types_and_rejects_unknown_type_names() {
        for name in SUPPORTED_TYPES {
            accepts(&format!(r#"{{"group":{{"$type":"{name}"}}}}"#));
            accepts(&format!(
                r#"{{"token":{{"$type":"{name}","$value":null}}}}"#
            ));
        }
        for name in ["", "Number", "COLOR", "string", "unknown"] {
            for token_property in ["", r#""$value":1,"#] {
                rejects_at(
                    &format!(r#"{{"entry":{{{token_property}"$type":"{name}"}}}}"#),
                    "/entry/$type",
                    "type",
                );
            }
        }
    }

    #[test]
    fn accepts_valid_metadata_without_evaluating_embedded_references() {
        for deprecated in ["true", "false", "\"\"", "\"Use {missing} instead\""] {
            for token_property in ["", r#""$value":1,"#] {
                accepts(&format!(
                    r#"{{"entry":{{{token_property}"$description":"{{missing}}","$deprecated":{deprecated},"$extensions":{{}}}}}}"#
                ));
            }
        }
    }

    #[test]
    fn keeps_all_vendor_extension_values_opaque() {
        let extensions = r##"{"plainKey":null,"scalar":1,"array":[true,"{missing}"],"$value":{"$type":false,"bad.name":{"$ref":"#/missing"}},"vendor":{"$extends":"{missing}","$root":[]}}"##;
        accepts(&format!(r#"{{"$extensions":{extensions}}}"#));
        accepts(&format!(r#"{{"group":{{"$extensions":{extensions}}}}}"#));
        accepts(&format!(
            r#"{{"token":{{"$value":1,"$extensions":{extensions}}}}}"#
        ));
    }

    #[test]
    fn leaves_token_values_and_missing_type_decisions_to_later_stages() {
        accepts(
            r##"{"token":{"$value":{"bad.name":null,"$type":false,"nested":{"$ref":"#/missing"}}}}"##,
        );
        accepts(r#"{"token":{"$value":null},"color-named-group":{"$value":false}}"#);
    }

    #[test]
    fn accepts_duplicate_child_names_and_checks_each_definition() {
        accepts(r#"{"x":{"$value":1},"x":{"$value":2},"g":{},"g":{"n":{"$value":3}}}"#);
        for input in [
            r#"{"x":{"$description":false},"x":{"$value":2}}"#,
            r#"{"x":{"$value":2},"x":{"$description":false}}"#,
        ] {
            rejects_at(input, "/x/$description", "metadata");
        }
        rejects_at(r#"{"x":{},"x":1}"#, "/x", "structure");
    }

    #[test]
    fn rejects_conflicting_known_properties_without_selecting_one() {
        for (property, first, second) in [
            ("$value", "1", "2"),
            ("$type", "\"number\"", "false"),
            ("$description", "\"text\"", "false"),
            ("$deprecated", "true", "false"),
            ("$extensions", "{}", "null"),
            ("$ref", "\"#/a\"", "\"#/b\""),
            ("$extends", "\"{a}\"", "\"{b}\""),
            ("$root", r#"{"$value":1}"#, r#"{"$value":2}"#),
        ] {
            for (first, second) in [(first, second), (second, first)] {
                let input =
                    format!(r#"{{"entry":{{"{property}":{first},"{property}":{second}}}}}"#);
                let report = check(&input);
                assert_eq!(
                    report.diagnostics.len(),
                    1,
                    "{input}: {:?}",
                    report.diagnostics
                );
                assert_eq!(report.diagnostics[0].path, format!("/entry/{property}"));
                assert_eq!(report.diagnostics[0].code, "structure");
            }
        }
        rejects_at(
            r#"{"x":{"$type":"number","\u0024type":"color"}}"#,
            "/x/$type",
            "structure",
        );
    }

    #[test]
    fn accepts_identical_known_duplicates_and_opaque_metadata_duplicates() {
        accepts(
            r#"{"x":{"$value":1,"$value":1,"$type":"number","$type":"number","$description":"x","$description":"x","$deprecated":true,"$deprecated":true,"$extensions":{},"$extensions":{}}}"#,
        );
        accepts(
            r##"{"x":{"$ref":"#/a","$ref":"#/a"},"g":{"$extends":"{base}","$extends":"{base}"},"$root":{"$value":1},"$root":{"$value":1}}"##,
        );
        accepts(r#"{"$custom":1,"$custom":2,"$extensions":{"vendor":1,"vendor":2}}"#);
    }

    #[test]
    fn reports_original_paths_with_rfc6901_escaping_and_empty_segments() {
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
}
