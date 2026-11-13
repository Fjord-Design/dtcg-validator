use crate::decimal::Decimal;
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

/// Object members remain ordered and non-unique; later pointer resolution must
/// reject ambiguous lookups rather than silently selecting a duplicate member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Number {
    lexeme: String,
}

impl Number {
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.lexeme
    }

    pub(crate) fn decimal(&self) -> Decimal<'_> {
        Decimal::from_json_number(&self.lexeme)
    }
}

/// Keeps decoding-error positions relative to the complete input, including its
/// leading whitespace. RawValue's syntax check alone permits lone surrogates;
/// the cursor's string decoding must therefore also be fallible.
pub(crate) fn parse(input: &str) -> Result<Value, serde_json::Error> {
    let _: &RawValue = serde_json::from_str(input)?;
    Cursor { input, position: 0 }.parse()
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // deserialize_any exposes arbitrary-precision numbers as private maps,
        // indistinguishable from sentinel-shaped user objects, and normalizes
        // exponent spelling. RawValue validates JSON syntax while retaining its
        // bytes. The cursor then builds the AST in one linear pass, without
        // revalidating every nested RawValue or guessing sentinels.
        let raw = <&RawValue>::deserialize(deserializer)?;
        Cursor {
            input: raw.get(),
            position: 0,
        }
        .parse()
        .map_err(serde::de::Error::custom)
    }
}

enum Frame {
    Array(Vec<Value>),
    Object {
        members: Vec<(String, Value)>,
        key: String,
    },
}

/// This cursor only receives a complete, validated RawValue. Its byte-level
/// traversal never interprets UTF-8 bytes as string contents: serde_json handles
/// string decoding, and all slice boundaries are ASCII JSON delimiters.
struct Cursor<'a> {
    input: &'a str,
    position: usize,
}

impl Cursor<'_> {
    fn parse(mut self) -> Result<Value, serde_json::Error> {
        let mut frames = Vec::new();
        loop {
            self.whitespace();
            let mut value = match self.byte() {
                b'[' => {
                    self.position += 1;
                    self.whitespace();
                    if self.byte() == b']' {
                        self.position += 1;
                        Value::Array(Vec::new())
                    } else {
                        frames.push(Frame::Array(Vec::new()));
                        continue;
                    }
                }
                b'{' => {
                    self.position += 1;
                    self.whitespace();
                    if self.byte() == b'}' {
                        self.position += 1;
                        Value::Object(Vec::new())
                    } else {
                        frames.push(Frame::Object {
                            members: Vec::new(),
                            key: self.key()?,
                        });
                        continue;
                    }
                }
                b'"' => Value::String(self.string()?),
                b'n' => {
                    self.position += 4;
                    Value::Null
                }
                b't' => {
                    self.position += 4;
                    Value::Bool(true)
                }
                b'f' => {
                    self.position += 5;
                    Value::Bool(false)
                }
                b'-' | b'0'..=b'9' => {
                    let start = self.position;
                    while self.position < self.input.len()
                        && matches!(self.byte(), b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                    {
                        self.position += 1;
                    }
                    Value::Number(Number {
                        lexeme: self.input[start..self.position].to_owned(),
                    })
                }
                _ => unreachable!("RawValue already validated the JSON value"),
            };

            loop {
                self.whitespace();
                match frames.last_mut() {
                    None => return Ok(value),
                    Some(Frame::Array(items)) => {
                        items.push(value);
                        if self.byte() == b',' {
                            self.position += 1;
                            break;
                        }
                        debug_assert_eq!(self.byte(), b']');
                        self.position += 1;
                        let Some(Frame::Array(items)) = frames.pop() else {
                            unreachable!("the current frame is an array");
                        };
                        value = Value::Array(items);
                    }
                    Some(Frame::Object { members, key }) => {
                        members.push((std::mem::take(key), value));
                        if self.byte() == b',' {
                            self.position += 1;
                            *key = self.key()?;
                            break;
                        }
                        debug_assert_eq!(self.byte(), b'}');
                        self.position += 1;
                        let Some(Frame::Object { members, .. }) = frames.pop() else {
                            unreachable!("the current frame is an object");
                        };
                        value = Value::Object(members);
                    }
                }
            }
        }
    }

    fn byte(&self) -> u8 {
        self.input.as_bytes()[self.position]
    }

    fn whitespace(&mut self) {
        while self.position < self.input.len()
            && matches!(self.byte(), b' ' | b'\n' | b'\r' | b'\t')
        {
            self.position += 1;
        }
    }

    fn key(&mut self) -> Result<String, serde_json::Error> {
        self.whitespace();
        let key = self.string()?;
        self.whitespace();
        debug_assert_eq!(self.byte(), b':');
        self.position += 1;
        Ok(key)
    }

    fn string(&mut self) -> Result<String, serde_json::Error> {
        let start = self.position;
        debug_assert_eq!(self.byte(), b'"');
        self.position += 1;
        loop {
            match self.byte() {
                b'\\' => self.position += 2,
                b'"' => {
                    self.position += 1;
                    return serde_json::from_str(&self.input[start..self.position])
                        .map_err(|error| relocate_string_error(error, self.input, start));
                }
                _ => self.position += 1,
            }
        }
    }
}

fn relocate_string_error(error: serde_json::Error, input: &str, start: usize) -> serde_json::Error {
    let prefix = &input[..start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + error.line();
    let column = if error.line() == 1 {
        prefix.rsplit('\n').next().unwrap_or("").len() + error.column()
    } else {
        error.column()
    };
    let message = error.to_string();
    let reason = message
        .rsplit_once(" at line ")
        .map_or(message.as_str(), |(reason, _)| reason);
    // serde_json preserves a location passed through Error::custom in this
    // standard suffix form instead of replacing it with the outer cursor's end.
    serde::de::Error::custom(format!("{reason} at line {line} column {column}"))
}

#[cfg(test)]
mod tests {
    use super::Value;
    use std::cmp::Ordering::Greater;

    #[test]
    fn preserves_duplicate_members_in_source_order() {
        let value: Value =
            serde_json::from_str(r#"{"x":1,"x":2,"nested":{"x":3,"\u0078":4}}"#).unwrap();
        let Value::Object(members) = value else {
            panic!("expected an object");
        };
        assert_eq!(members.len(), 3);
        assert_eq!(
            members
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            ["x", "x", "nested"]
        );
        let Value::Number(first) = &members[0].1 else {
            panic!("expected the first duplicate's number");
        };
        assert_eq!(first.as_str(), "1");
        let Value::Number(second) = &members[1].1 else {
            panic!("expected the second duplicate's number");
        };
        assert_eq!(second.as_str(), "2");
        let Value::Object(nested) = &members[2].1 else {
            panic!("expected a nested object");
        };
        assert_eq!(nested.len(), 2);
        assert_eq!(nested[0].0, "x");
        assert_eq!(nested[1].0, "x");
    }

    #[test]
    fn preserves_every_numeric_lexeme() {
        let lexemes = [
            "0",
            "-0",
            "1.0",
            "1e1000",
            "1E+1000",
            "1e-000123",
            "18446744073709551616",
            "1000.0000000000000001",
            "1e9999999999999999999999999999999999999999999999999999999999",
            "-1E-9999999999999999999999999999999999999999999999999999999999",
        ];
        let document = format!("[{}]", lexemes.join(","));
        let value: Value = serde_json::from_str(&document).unwrap();
        let Value::Array(numbers) = value else {
            panic!("expected an array");
        };
        for (number, expected) in numbers.iter().zip(lexemes) {
            let Value::Number(number) = number else {
                panic!("expected a number, not an arbitrary-precision sentinel map");
            };
            assert_eq!(number.as_str(), expected);
        }
        assert_eq!(numbers.len(), lexemes.len());
        let Value::Number(number) = &numbers[3] else {
            panic!("expected a number");
        };
        assert_eq!(number.decimal().cmp_integer(1000), Greater);
    }

    #[test]
    fn preserves_private_sentinel_shaped_objects() {
        for key in [
            "$serde_json::private::Number",
            "$serde_json::private::RawValue",
        ] {
            let document = format!(r#"{{"{key}":"1"}}"#);
            let value: Value = serde_json::from_str(&document).unwrap();
            assert_eq!(
                value,
                Value::Object(vec![(key.to_owned(), Value::String("1".to_owned()))])
            );
        }
    }

    #[test]
    fn decodes_unicode_and_escaped_strings_without_losing_structure() {
        let value: Value = serde_json::from_str(
            r#" { "é🚲": [null, true, false, "é🚲\"\\\n\uD83D\uDE80", {}, []] } "#,
        )
        .unwrap();
        assert_eq!(
            value,
            Value::Object(vec![(
                "é🚲".to_owned(),
                Value::Array(vec![
                    Value::Null,
                    Value::Bool(true),
                    Value::Bool(false),
                    Value::String("é🚲\"\\\n🚀".to_owned()),
                    Value::Object(Vec::new()),
                    Value::Array(Vec::new()),
                ]),
            )])
        );
    }

    #[test]
    fn rejects_malformed_and_trailing_json() {
        for document in [
            "",
            "{",
            "[1,]",
            "{\"x\":}",
            "1e",
            "01",
            "{} {}",
            "\"\\uD800\"",
        ] {
            let error = serde_json::from_str::<Value>(document).unwrap_err();
            assert!(error.line() > 0, "{document:?}: {error}");
            assert!(
                error.to_string().contains("column"),
                "{document:?}: {error}"
            );
        }
    }
}
