//! Format 2025.10 value definitions and Color 2025.10 component bounds.
//!
//! References are resolved only at the slot being inspected. Primitive inner
//! slots use `Expected::Literal`: pointer wrappers still work, but literal
//! braces and opaque metadata never become aliases. Referenced arrays remain
//! arrays, including when a slot requires one shadow or one gradient stop.
use std::cmp::Ordering;

use crate::Diagnostic;
use crate::json::Value;
use crate::resolver::{Expected, Resolver, TokenId, ValueCursor};

#[derive(Clone, Copy)]
enum Site {
    Cursor(ValueCursor),
    Token(TokenId),
}

#[derive(Clone, Copy)]
struct At {
    cursor: ValueCursor,
    // Once redirected, diagnose incompatibilities at the original reference,
    // not at the target as though the alias contained that literal value.
    site: Option<Site>,
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Option<i32>,
    max: Option<i32>,
    exclusive_max: bool,
}

impl Bounds {
    const ANY: Self = Self {
        min: None,
        max: None,
        exclusive_max: false,
    };
    const UNIT: Self = Self::closed(0, 1);
    const HUE: Self = Self {
        min: Some(0),
        max: Some(360),
        exclusive_max: true,
    };
    const NONNEGATIVE: Self = Self {
        min: Some(0),
        max: None,
        exclusive_max: false,
    };

    const fn closed(min: i32, max: i32) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
            exclusive_max: false,
        }
    }
}

const FONT_WEIGHTS: &[&str] = &[
    "thin",
    "hairline",
    "extra-light",
    "ultra-light",
    "light",
    "normal",
    "regular",
    "book",
    "medium",
    "semi-bold",
    "demi-bold",
    "bold",
    "extra-bold",
    "ultra-bold",
    "black",
    "heavy",
    "extra-black",
    "ultra-black",
];
const STROKE_STYLES: &[&str] = &[
    "solid", "dashed", "dotted", "double", "groove", "ridge", "outset", "inset",
];
const SHADOW_FIELDS: &[(&str, &str)] = &[
    ("color", "color"),
    ("offsetX", "dimension"),
    ("offsetY", "dimension"),
    ("blur", "dimension"),
    ("spread", "dimension"),
];

/// Validates the prepared effective token namespace, sharing resolution/type
/// caches with the resolver rather than scanning every value a second time.
pub(crate) fn validate(resolver: &mut Resolver<'_>) -> Vec<Diagnostic> {
    let mut validator = Validator {
        resolver,
        diagnostics: Vec::new(),
    };
    // Resolution may grow the view table, but never changes this token order.
    for index in 0..validator.resolver.tokens().len() {
        let token = validator.resolver.tokens()[index];
        validator.token(token);
    }
    validator.diagnostics
}

struct Validator<'r, 'a> {
    resolver: &'r mut Resolver<'a>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Validator<'_, 'a> {
    fn token(&mut self, token: TokenId) {
        let value = self.resolver.token_value(token);
        let token_type = self.resolver.token_type(token);
        let (Ok(value), Ok(token_type)) = (value, token_type) else {
            return;
        };
        if let Some(alias) = value.alias.filter(|_| value.curly_alias) {
            let Ok(alias_type) = self.resolver.token_type(alias) else {
                return;
            };
            if alias_type.name != token_type.name {
                self.report(
                    At {
                        cursor: value.cursor,
                        site: Some(Site::Token(token)),
                    },
                    "reference",
                    "The referenced token has an incompatible type.",
                );
                return;
            }
        }
        let redirected =
            self.resolver.source_path(value.cursor) != self.resolver.token_reference_path(token);
        self.resolved_type(
            At {
                cursor: value.cursor,
                site: redirected.then_some(Site::Token(token)),
            },
            token_type.name,
        );
    }

    fn head(&mut self, at: At, expected: Expected<'_>) -> Option<At> {
        let value = self.resolver.resolve_head(at.cursor, expected).ok()?;
        let redirected =
            self.resolver.source_path(at.cursor) != self.resolver.source_path(value.cursor);
        Some(At {
            cursor: value.cursor,
            site: at
                .site
                .or_else(|| redirected.then_some(Site::Cursor(at.cursor))),
        })
    }

    fn child(&self, at: At, index: usize) -> At {
        At {
            cursor: self.resolver.child(at.cursor, index),
            site: at.site,
        }
    }

    fn value(&self, at: At) -> &'a Value {
        self.resolver.value(at.cursor)
    }

    fn report(&mut self, at: At, code: &str, message: impl Into<String>) {
        let path = match at.site {
            Some(Site::Token(token)) => self.resolver.token_reference_path(token),
            Some(Site::Cursor(cursor)) => self.reference_path(cursor),
            None => self.resolver.source_path(at.cursor),
        }
        .to_owned();
        let message = message.into();
        // Inherited placements can have different effective types, but a
        // repeated failure at the same original declaration is one diagnostic.
        if self.diagnostics.iter().any(|diagnostic| {
            diagnostic.path == path && diagnostic.code == code && diagnostic.message == message
        }) {
            return;
        }
        self.diagnostics.push(Diagnostic {
            path,
            code: code.to_owned(),
            message,
        });
    }

    fn reference_path(&self, cursor: ValueCursor) -> &str {
        if let Value::Object(members) = self.resolver.value(cursor) {
            if members.iter().all(|(name, _)| name == "$ref") {
                if let Some(index) = members.iter().position(|(name, _)| name == "$ref") {
                    return self
                        .resolver
                        .source_path(self.resolver.child(cursor, index));
                }
            }
        }
        self.resolver.source_path(cursor)
    }

    fn typed(&mut self, at: At, name: &'static str) {
        if let Some(at) = self.head(at, Expected::Type(name)) {
            self.resolved_type(at, name);
        }
    }

    fn resolved_type(&mut self, at: At, name: &'static str) {
        match name {
            "number" => self.number(at, Bounds::ANY),
            "dimension" => self.quantity(at, &["px", "rem"], "a dimension unit (px or rem)"),
            "duration" => self.quantity(at, &["ms", "s"], "a duration unit (ms or s)"),
            "fontFamily" => self.font_family(at),
            "fontWeight" => self.font_weight(at),
            "cubicBezier" => self.cubic_bezier(at),
            "color" => self.color(at),
            "strokeStyle" => self.stroke_style(at),
            "border" => self.composite(
                at,
                &[
                    ("color", "color"),
                    ("width", "dimension"),
                    ("style", "strokeStyle"),
                ],
                &[],
            ),
            "transition" => self.composite(
                at,
                &[
                    ("duration", "duration"),
                    ("delay", "duration"),
                    ("timingFunction", "cubicBezier"),
                ],
                &[],
            ),
            "shadow" => self.shadow(at),
            "gradient" => self.gradient(at),
            "typography" => self.composite(
                at,
                &[
                    ("fontFamily", "fontFamily"),
                    ("fontSize", "dimension"),
                    ("fontWeight", "fontWeight"),
                    ("letterSpacing", "dimension"),
                    ("lineHeight", "number"),
                ],
                &[],
            ),
            _ => unreachable!("token_type returns only supported types"),
        }
    }

    fn object(&mut self, at: At, required: &[&str], optional: &[&str], extras: bool) -> bool {
        let Value::Object(members) = self.value(at) else {
            self.report(at, "value", "Expected an object.");
            return false;
        };
        for name in required {
            if !members.iter().any(|(key, _)| key.as_str() == *name) {
                self.report(at, "value", format!("Missing required {name} field."));
            }
        }
        if !extras {
            for (index, (name, _)) in members.iter().enumerate() {
                if !required.contains(&name.as_str()) && !optional.contains(&name.as_str()) {
                    self.report(
                        self.child(at, index),
                        "value",
                        format!("Unknown composite field {name}."),
                    );
                }
            }
        }
        true
    }

    fn number(&mut self, at: At, bounds: Bounds) {
        let Value::Number(number) = self.value(at) else {
            self.report(at, "value", "Expected a JSON number.");
            return;
        };
        let decimal = number.decimal();
        if let Some(min) = bounds.min {
            if decimal.cmp_integer(min) == Ordering::Less {
                self.report(at, "value", format!("Number must be at least {min}."));
                return;
            }
        }
        if let Some(max) = bounds.max {
            let order = decimal.cmp_integer(max);
            if order == Ordering::Greater || (bounds.exclusive_max && order == Ordering::Equal) {
                let relation = if bounds.exclusive_max {
                    "less than"
                } else {
                    "at most"
                };
                self.report(at, "value", format!("Number must be {relation} {max}."));
            }
        }
    }

    fn literal_number(&mut self, at: At, bounds: Bounds) {
        if let Some(at) = self.head(at, Expected::Literal) {
            self.number(at, bounds);
        }
    }

    fn literal_string(&mut self, at: At) -> Option<&'a str> {
        let at = self.head(at, Expected::Literal)?;
        if let Value::String(text) = self.value(at) {
            Some(text)
        } else {
            self.report(at, "value", "Expected a string.");
            None
        }
    }

    fn literal_enum(&mut self, at: At, allowed: &[&str], description: &str) {
        let Some(at) = self.head(at, Expected::Literal) else {
            return;
        };
        if !matches!(self.value(at), Value::String(text) if allowed.contains(&text.as_str())) {
            self.report(at, "value", format!("Expected {description}."));
        }
    }

    fn quantity(&mut self, at: At, units: &[&str], description: &str) {
        if !self.object(at, &["value", "unit"], &[], true) {
            return;
        }
        let Value::Object(members) = self.value(at) else {
            unreachable!()
        };
        for (index, (key, _)) in members.iter().enumerate() {
            let child = self.child(at, index);
            match key.as_str() {
                "value" => self.literal_number(child, Bounds::ANY),
                "unit" => self.literal_enum(child, units, description),
                _ => {} // Singular-value extra members are opaque.
            }
        }
    }

    fn font_family(&mut self, at: At) {
        match self.value(at) {
            Value::String(_) => {}
            Value::Array(values) => {
                for index in 0..values.len() {
                    self.literal_string(self.child(at, index));
                }
            }
            _ => self.report(
                at,
                "value",
                "fontFamily must be a string or an array of strings.",
            ),
        }
    }

    fn font_weight(&mut self, at: At) {
        match self.value(at) {
            Value::Number(_) => self.number(at, Bounds::closed(1, 1000)),
            Value::String(name) if FONT_WEIGHTS.contains(&name.as_str()) => {}
            _ => self.report(
                at,
                "value",
                "fontWeight must be a number in [1,1000] or a specified weight name.",
            ),
        }
    }

    fn cubic_bezier(&mut self, at: At) {
        let Value::Array(values) = self.value(at) else {
            self.report(at, "value", "cubicBezier must be an array of four numbers.");
            return;
        };
        if values.len() != 4 {
            self.report(
                at,
                "value",
                "cubicBezier must contain exactly four numbers.",
            );
        }
        for index in 0..values.len() {
            let bounds = if index == 0 || index == 2 {
                Bounds::UNIT
            } else {
                Bounds::ANY
            };
            self.literal_number(self.child(at, index), bounds);
        }
    }

    fn color(&mut self, at: At) {
        if !self.object(at, &["colorSpace", "components"], &["alpha", "hex"], true) {
            return;
        }
        let Value::Object(members) = self.value(at) else {
            unreachable!()
        };
        // Each declared space is checked; every component must satisfy all
        // declared spaces if duplicate colorSpace members occur. No duplicate
        // member is arbitrarily selected as the sole authoritative value.
        let mut bounds = None;
        for (index, (key, _)) in members.iter().enumerate() {
            if key == "colorSpace" {
                let child = self.child(at, index);
                if let Some(space_at) = self.head(child, Expected::Literal) {
                    match self.value(space_at) {
                        Value::String(space) => match color_bounds(space) {
                            Some(next) => {
                                bounds = Some(match bounds {
                                    None => next,
                                    Some(previous) => intersect_bounds(previous, next),
                                });
                            }
                            None => {
                                self.report(space_at, "value", "Unknown Color 2025.10 color space.")
                            }
                        },
                        _ => self.report(space_at, "value", "colorSpace must be a string."),
                    }
                }
            }
        }
        for (index, (key, _)) in members.iter().enumerate() {
            let child = self.child(at, index);
            match key.as_str() {
                "components" => self.color_components(child, bounds),
                "alpha" => self.literal_number(child, Bounds::UNIT),
                "hex" => {
                    if let Some(hex_at) = self.head(child, Expected::Literal) {
                        if !matches!(self.value(hex_at), Value::String(hex) if hex.len() == 7 && hex.starts_with('#') && hex.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
                        {
                            self.report(
                                hex_at,
                                "value",
                                "hex must be # followed by exactly six hexadecimal digits.",
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn color_components(&mut self, at: At, bounds: Option<[Bounds; 3]>) {
        let Some(at) = self.head(at, Expected::Literal) else {
            return;
        };
        let Value::Array(values) = self.value(at) else {
            self.report(
                at,
                "value",
                "Color components must be an array of three numbers or none values.",
            );
            return;
        };
        if values.len() != 3 {
            self.report(
                at,
                "value",
                "Color components must contain exactly three entries.",
            );
        }
        for index in 0..values.len() {
            if let Some(child) = self.head(self.child(at, index), Expected::Literal) {
                if matches!(self.value(child), Value::String(text) if text == "none") {
                    continue;
                }
                let bound = bounds
                    .and_then(|bounds| bounds.get(index).copied())
                    .unwrap_or(Bounds::ANY);
                self.number(child, bound);
            }
        }
    }

    fn stroke_style(&mut self, at: At) {
        if let Value::String(style) = self.value(at) {
            if !STROKE_STYLES.contains(&style.as_str()) {
                self.report(at, "value", "Unknown strokeStyle name.");
            }
            return;
        }
        if !self.object(at, &["dashArray", "lineCap"], &[], false) {
            return;
        }
        let Value::Object(members) = self.value(at) else {
            unreachable!()
        };
        for (index, (key, _)) in members.iter().enumerate() {
            let child = self.child(at, index);
            match key.as_str() {
                "lineCap" => {
                    self.literal_enum(child, &["round", "butt", "square"], "round, butt or square")
                }
                "dashArray" => {
                    if let Some(array) = self.head(child, Expected::Literal) {
                        if let Value::Array(values) = self.value(array) {
                            for index in 0..values.len() {
                                self.typed(self.child(array, index), "dimension");
                            }
                        } else {
                            self.report(
                                array,
                                "value",
                                "dashArray must be an array of dimensions.",
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn composite(&mut self, at: At, fields: &[(&str, &'static str)], optional: &[&str]) {
        let Value::Object(members) = self.value(at) else {
            self.report(at, "value", "Expected a composite object.");
            return;
        };
        for (name, _) in fields {
            if !members.iter().any(|(key, _)| key.as_str() == *name) {
                self.report(at, "value", format!("Missing required {name} field."));
            }
        }
        for (index, (key, _)) in members.iter().enumerate() {
            let child = self.child(at, index);
            if let Some((_, name)) = fields.iter().find(|(name, _)| *name == key.as_str()) {
                self.typed(child, name);
            } else if optional.contains(&key.as_str()) {
                // The only optional composite field is shadow's boolean inset.
                if let Some(child) = self.head(child, Expected::Literal) {
                    if !matches!(self.value(child), Value::Bool(_)) {
                        self.report(child, "value", "inset must be a boolean.");
                    }
                }
            } else {
                self.report(child, "value", format!("Unknown composite field {key}."));
            }
        }
    }

    fn shadow(&mut self, at: At) {
        if let Value::Array(values) = self.value(at) {
            for index in 0..values.len() {
                if let Some(child) = self.head(self.child(at, index), Expected::Type("shadow")) {
                    self.composite(child, SHADOW_FIELDS, &["inset"]);
                }
            }
        } else {
            self.composite(at, SHADOW_FIELDS, &["inset"]);
        }
    }

    fn gradient(&mut self, at: At) {
        let Value::Array(values) = self.value(at) else {
            self.report(at, "value", "gradient must be an array of stop objects.");
            return;
        };
        for index in 0..values.len() {
            if let Some(child) = self.head(self.child(at, index), Expected::Any) {
                self.composite(child, &[("color", "color"), ("position", "number")], &[]);
            }
        }
    }
}

fn color_bounds(space: &str) -> Option<[Bounds; 3]> {
    let percent = Bounds::closed(0, 100);
    Some(match space {
        "srgb" | "srgb-linear" | "display-p3" | "a98-rgb" | "prophoto-rgb" | "rec2020"
        | "xyz-d65" | "xyz-d50" => [Bounds::UNIT; 3],
        "hsl" | "hwb" => [Bounds::HUE, percent, percent],
        "lab" => [percent, Bounds::ANY, Bounds::ANY],
        "lch" => [percent, Bounds::NONNEGATIVE, Bounds::HUE],
        "oklab" => [Bounds::UNIT, Bounds::ANY, Bounds::ANY],
        "oklch" => [Bounds::UNIT, Bounds::NONNEGATIVE, Bounds::HUE],
        _ => return None,
    })
}

fn intersect_bounds(left: [Bounds; 3], right: [Bounds; 3]) -> [Bounds; 3] {
    std::array::from_fn(|index| {
        let left = left[index];
        let right = right[index];
        let min = match (left.min, right.min) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
        let (max, exclusive_max) = match (left.max, right.max) {
            (Some(a), Some(b)) if a == b => (Some(a), left.exclusive_max || right.exclusive_max),
            (Some(a), Some(b)) if a < b => (Some(a), left.exclusive_max),
            (Some(_), Some(b)) => (Some(b), right.exclusive_max),
            (Some(a), None) => (Some(a), left.exclusive_max),
            (None, b) => (b, right.exclusive_max),
        };
        Bounds {
            min,
            max,
            exclusive_max,
        }
    })
}

#[cfg(test)]
mod tests;
