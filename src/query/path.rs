use serde_json::Value;

use crate::error::QfError;

/// A single segment of a query path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Object key lookup: `.foo`
    Key(String),
    /// Array index: `[0]`, `[42]`
    Index(usize),
    /// Array iterator: `[]` — maps remaining path over all elements
    Iterator,
}

/// A parsed query path consisting of segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPath {
    pub segments: Vec<Segment>,
}

impl QueryPath {
    /// Parse a query string like `.foo.bar[0].baz` or `.items[].name`.
    pub fn parse(input: &str) -> Result<Self, QfError> {
        let input = input.trim();

        // Identity query
        if input == "." || input.is_empty() {
            return Ok(QueryPath { segments: vec![] });
        }

        let mut segments = Vec::new();
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        // Must start with '.'
        if chars.first() != Some(&'.') {
            return Err(QfError::InvalidQuery(format!(
                "path must start with '.', got: {input}"
            )));
        }
        i += 1; // skip leading dot

        while i < chars.len() {
            if chars[i] == '[' {
                i += 1; // skip '['

                if i < chars.len() && chars[i] == ']' {
                    // Iterator: []
                    segments.push(Segment::Iterator);
                    i += 1; // skip ']'
                } else {
                    // Index: [N]
                    let start = i;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i >= chars.len() || chars[i] != ']' {
                        return Err(QfError::InvalidQuery(format!(
                            "expected ']' in index at position {i}"
                        )));
                    }
                    let num_str: String = chars[start..i].iter().collect();
                    let index: usize = num_str.parse().map_err(|_| {
                        QfError::InvalidQuery(format!("invalid array index: {num_str}"))
                    })?;
                    segments.push(Segment::Index(index));
                    i += 1; // skip ']'
                }
            } else if chars[i] == '.' {
                i += 1; // skip '.'
                // Read key
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                    i += 1;
                }
                if i == start {
                    return Err(QfError::InvalidQuery(format!(
                        "empty key at position {start}"
                    )));
                }
                let key: String = chars[start..i].iter().collect();
                segments.push(Segment::Key(key));
            } else {
                // First key after leading dot (no second dot needed)
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                    i += 1;
                }
                if i == start {
                    return Err(QfError::InvalidQuery(format!(
                        "empty key at position {start}"
                    )));
                }
                let key: String = chars[start..i].iter().collect();
                segments.push(Segment::Key(key));
            }
        }

        Ok(QueryPath { segments })
    }

    /// Evaluate this path against a JSON Value, returning the matched value(s).
    pub fn evaluate<'a>(&self, value: &'a Value) -> Result<Value, QfError> {
        evaluate_segments(&self.segments, value)
    }
}

fn evaluate_segments(segments: &[Segment], value: &Value) -> Result<Value, QfError> {
    if segments.is_empty() {
        return Ok(value.clone());
    }

    let segment = &segments[0];
    let rest = &segments[1..];

    match segment {
        Segment::Key(key) => match value {
            Value::Object(map) => match map.get(key) {
                Some(v) => evaluate_segments(rest, v),
                None => Err(QfError::PathNotFound(format!(".{key}"))),
            },
            other => Err(QfError::ExpectedObject(value_type_name(other).into())),
        },
        Segment::Index(idx) => match value {
            Value::Array(arr) => match arr.get(*idx) {
                Some(v) => evaluate_segments(rest, v),
                None => Err(QfError::IndexOutOfBounds {
                    index: *idx,
                    length: arr.len(),
                }),
            },
            other => Err(QfError::ExpectedArray(value_type_name(other).into())),
        },
        Segment::Iterator => match value {
            Value::Array(arr) => {
                let results: Result<Vec<Value>, _> = arr
                    .iter()
                    .map(|item| evaluate_segments(rest, item))
                    .collect();
                Ok(Value::Array(results?))
            }
            other => Err(QfError::ExpectedArray(value_type_name(other).into())),
        },
    }
}

fn value_type_name(v: &Value) -> &str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Parse tests ---

    #[test]
    fn parse_identity() {
        let p = QueryPath::parse(".").unwrap();
        assert!(p.segments.is_empty());
    }

    #[test]
    fn parse_simple_key() {
        let p = QueryPath::parse(".name").unwrap();
        assert_eq!(p.segments, vec![Segment::Key("name".into())]);
    }

    #[test]
    fn parse_nested_keys() {
        let p = QueryPath::parse(".a.b.c").unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::Key("a".into()),
                Segment::Key("b".into()),
                Segment::Key("c".into()),
            ]
        );
    }

    #[test]
    fn parse_index() {
        let p = QueryPath::parse(".items[0]").unwrap();
        assert_eq!(
            p.segments,
            vec![Segment::Key("items".into()), Segment::Index(0)]
        );
    }

    #[test]
    fn parse_iterator() {
        let p = QueryPath::parse(".items[]").unwrap();
        assert_eq!(
            p.segments,
            vec![Segment::Key("items".into()), Segment::Iterator]
        );
    }

    #[test]
    fn parse_complex_path() {
        let p = QueryPath::parse(".spec.containers[0].image").unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::Key("spec".into()),
                Segment::Key("containers".into()),
                Segment::Index(0),
                Segment::Key("image".into()),
            ]
        );
    }

    #[test]
    fn parse_iterator_with_key() {
        let p = QueryPath::parse(".items[].name").unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::Key("items".into()),
                Segment::Iterator,
                Segment::Key("name".into()),
            ]
        );
    }

    #[test]
    fn parse_error_no_dot() {
        assert!(QueryPath::parse("name").is_err());
    }

    // --- Evaluate tests ---

    #[test]
    fn eval_identity() {
        let val = json!({"a": 1});
        let path = QueryPath::parse(".").unwrap();
        assert_eq!(path.evaluate(&val).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn eval_simple_key() {
        let val = json!({"name": "hello"});
        let path = QueryPath::parse(".name").unwrap();
        assert_eq!(path.evaluate(&val).unwrap(), json!("hello"));
    }

    #[test]
    fn eval_nested() {
        let val = json!({"a": {"b": {"c": 42}}});
        let path = QueryPath::parse(".a.b.c").unwrap();
        assert_eq!(path.evaluate(&val).unwrap(), json!(42));
    }

    #[test]
    fn eval_array_index() {
        let val = json!({"items": [10, 20, 30]});
        let path = QueryPath::parse(".items[1]").unwrap();
        assert_eq!(path.evaluate(&val).unwrap(), json!(20));
    }

    #[test]
    fn eval_iterator() {
        let val = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let path = QueryPath::parse(".items[].name").unwrap();
        assert_eq!(path.evaluate(&val).unwrap(), json!(["a", "b"]));
    }

    #[test]
    fn eval_complex_path() {
        let val = json!({
            "spec": {
                "containers": [
                    {"image": "nginx:latest"},
                    {"image": "redis:7"}
                ]
            }
        });
        let path = QueryPath::parse(".spec.containers[0].image").unwrap();
        assert_eq!(path.evaluate(&val).unwrap(), json!("nginx:latest"));
    }

    #[test]
    fn eval_missing_key_errors() {
        let val = json!({"a": 1});
        let path = QueryPath::parse(".missing").unwrap();
        assert!(path.evaluate(&val).is_err());
    }

    #[test]
    fn eval_index_out_of_bounds_errors() {
        let val = json!({"items": [1, 2]});
        let path = QueryPath::parse(".items[5]").unwrap();
        assert!(path.evaluate(&val).is_err());
    }

    #[test]
    fn eval_key_on_non_object_errors() {
        let val = json!("just a string");
        let path = QueryPath::parse(".foo").unwrap();
        assert!(path.evaluate(&val).is_err());
    }

    #[test]
    fn parse_key_with_hyphens_and_underscores() {
        let p = QueryPath::parse(".my-key.other_key").unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::Key("my-key".into()),
                Segment::Key("other_key".into()),
            ]
        );
    }
}
