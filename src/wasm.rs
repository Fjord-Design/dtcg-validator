use wasm_bindgen::prelude::*;

/// Validates a JSON document and returns the shared report as a JavaScript object.
///
/// Throws a JavaScript error if report serialization fails.
#[wasm_bindgen(js_name = validate)]
pub fn validate_wasm(input: &str) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&crate::validate(input)).map_err(Into::into)
}
