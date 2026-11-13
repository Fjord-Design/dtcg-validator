mod decimal;
mod json;
mod resolver;
mod structure;
mod types;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(target_arch = "wasm32")]
pub use wasm::validate_wasm;

use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Returns the validator crate version, callable from JavaScript.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// A validation result shared by native and WebAssembly consumers.
///
/// Reports returned by [`validate`] are valid exactly when their diagnostics are
/// empty. Diagnostics are sorted by path, code and message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// One validation failure at an RFC 6901 JSON Pointer path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// An empty string identifies the document root.
    pub path: String,
    /// The diagnostic category: json, structure, metadata, type, value,
    /// reference or cycle.
    pub code: String,
    pub message: String,
}

impl ValidationReport {
    pub(crate) fn from_diagnostics(mut diagnostics: Vec<Diagnostic>) -> Self {
        diagnostics.sort_unstable_by(|left, right| {
            (&left.path, &left.code, &left.message).cmp(&(&right.path, &right.code, &right.message))
        });
        Self {
            valid: diagnostics.is_empty(),
            diagnostics,
        }
    }
}

/// Checks a complete JSON document without filesystem, network or JavaScript I/O.
///
/// Validates structure, metadata, inheritance, references and all token values.
/// Malformed input produces a root-level `json` diagnostic with line and column.
pub fn validate(input: &str) -> ValidationReport {
    let diagnostics = match json::parse(input) {
        Ok(document) => {
            let mut diagnostics = structure::validate(&document);
            let mut resolver = resolver::Resolver::new(&document);
            resolver.prepare();
            diagnostics.extend(types::validate(&mut resolver));
            diagnostics.extend(resolver.into_diagnostics());
            diagnostics
        }
        Err(error) => vec![Diagnostic {
            path: String::new(),
            code: "json".to_owned(),
            message: error.to_string(),
        }],
    };
    ValidationReport::from_diagnostics(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, ValidationReport, validate};

    #[test]
    fn foundation_accepts_arbitrary_precision_json_numbers() {
        for number in [
            "1e1000",
            "1e9999999999999999999999999999999999999999999999999999999999",
            "1e-9999999999999999999999999999999999999999999999999999999999",
        ] {
            let input = format!(r#"{{"n":{{"$type":"number","$value":{number}}}}}"#);
            let report = validate(&input);
            assert!(report.valid, "{number}: {:?}", report.diagnostics);
            assert!(report.diagnostics.is_empty());
        }
    }

    #[test]
    fn malformed_json_reports_root_line_and_column() {
        let report = validate("{\n  \"x\": }\n");
        assert!(!report.valid);
        assert_eq!(report.diagnostics.len(), 1);
        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.path, "");
        assert_eq!(diagnostic.code, "json");
        assert!(
            diagnostic.message.contains("line 2"),
            "{}",
            diagnostic.message
        );
        assert!(
            diagnostic.message.contains("column"),
            "{}",
            diagnostic.message
        );
    }

    #[test]
    fn unicode_decoding_errors_keep_complete_document_locations() {
        for input in [" \n {\n  \"x\": \"\\uD800\"\n }", "\n {\"\\uDC00\": 1}"] {
            let expected = serde_json::from_str::<serde_json::Value>(input).unwrap_err();
            let report = validate(input);
            assert!(!report.valid);
            assert_eq!(report.diagnostics.len(), 1);
            assert_eq!(report.diagnostics[0].code, "json");
            assert_eq!(report.diagnostics[0].message, expected.to_string());
        }
    }

    #[test]
    fn rejects_trailing_content_instead_of_accepting_a_prefix() {
        let report = validate("{} {}");
        assert!(!report.valid);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, "json");
    }

    #[test]
    fn validates_structure_after_parsing_json() {
        for (input, path, code) in [
            ("[]", "", "structure"),
            (r#"{"$value":1}"#, "/$value", "structure"),
            (r#"{"child":1}"#, "/child", "structure"),
            (
                r#"{"g":{"$description":false}}"#,
                "/g/$description",
                "metadata",
            ),
        ] {
            let report = validate(input);
            assert!(!report.valid, "{input}");
            assert_eq!(report.diagnostics.len(), 1, "{input}");
            assert_eq!(report.diagnostics[0].path, path, "{input}");
            assert_eq!(report.diagnostics[0].code, code, "{input}");
        }
    }

    #[test]
    fn report_serializes_to_the_shared_contract() {
        let report = validate(r#"{"n":{"$type":"number","$value":1}}"#);
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"valid":true,"diagnostics":[]}"#
        );
    }

    #[test]
    fn diagnostics_sort_by_path_then_code_then_message() {
        let diagnostic = |path: &str, code: &str, message: &str| Diagnostic {
            path: path.to_owned(),
            code: code.to_owned(),
            message: message.to_owned(),
        };
        let ordered = vec![
            diagnostic("", "json", "root"),
            diagnostic("/a", "type", "alpha"),
            diagnostic("/a", "type", "zeta"),
            diagnostic("/a", "value", "value"),
            diagnostic("/b", "type", "other path"),
        ];
        let mut shuffled = ordered.clone();
        shuffled.reverse();
        let report = ValidationReport::from_diagnostics(shuffled);
        assert_eq!(report.diagnostics, ordered);
        assert!(!report.valid);
        assert!(ValidationReport::from_diagnostics(Vec::new()).valid);
    }
}
