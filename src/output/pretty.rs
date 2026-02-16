use serde_json::Value;

use crate::error::QfError;
use crate::format::Format;

/// Format a Value as a string in the given format.
pub fn format_value(value: &Value, format: Format, compact: bool, raw: bool) -> Result<String, QfError> {
    // Raw mode: if the value is a string, output it without quotes
    if raw {
        if let Value::String(s) = value {
            return Ok(s.clone());
        }
    }

    match format {
        Format::Json => format_json(value, compact),
        Format::Yaml => format_yaml(value),
    }
}

fn format_json(value: &Value, compact: bool) -> Result<String, QfError> {
    let result = if compact {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    };
    result.map_err(|e| QfError::Parse(e.to_string()))
}

fn format_yaml(value: &Value) -> Result<String, QfError> {
    serde_yaml::to_string(value).map_err(|e| QfError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_pretty() {
        let val = json!({"a": 1, "b": 2});
        let out = format_value(&val, Format::Json, false, false).unwrap();
        assert!(out.contains('\n'));
        assert!(out.contains("\"a\""));
    }

    #[test]
    fn json_compact() {
        let val = json!({"a": 1});
        let out = format_value(&val, Format::Json, true, false).unwrap();
        assert!(!out.contains('\n'));
    }

    #[test]
    fn yaml_output() {
        let val = json!({"name": "test", "count": 3});
        let out = format_value(&val, Format::Yaml, false, false).unwrap();
        assert!(out.contains("name:"));
        assert!(out.contains("count:"));
    }

    #[test]
    fn raw_string() {
        let val = json!("hello world");
        let out = format_value(&val, Format::Json, false, true).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn raw_non_string_ignored() {
        let val = json!(42);
        let out = format_value(&val, Format::Json, false, true).unwrap();
        assert_eq!(out, "42");
    }
}
