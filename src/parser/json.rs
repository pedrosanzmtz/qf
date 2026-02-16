use serde_json::Value;

use crate::error::QfError;

/// Parse a JSON string into a serde_json::Value.
pub fn parse(input: &str) -> Result<Value, QfError> {
    serde_json::from_str(input).map_err(|e| QfError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_object() {
        let val = parse(r#"{"name": "hello", "count": 42}"#).unwrap();
        assert_eq!(val["name"], "hello");
        assert_eq!(val["count"], 42);
    }

    #[test]
    fn parse_array() {
        let val = parse(r#"[1, 2, 3]"#).unwrap();
        assert_eq!(val[0], 1);
        assert_eq!(val[2], 3);
    }

    #[test]
    fn parse_nested() {
        let val = parse(r#"{"a": {"b": {"c": true}}}"#).unwrap();
        assert_eq!(val["a"]["b"]["c"], true);
    }

    #[test]
    fn invalid_json_errors() {
        assert!(parse("{not json}").is_err());
    }
}
