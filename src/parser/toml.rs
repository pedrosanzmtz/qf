use serde_json::Value;

use crate::error::QfError;

pub fn parse(input: &str) -> Result<Value, QfError> {
    let toml_val: toml::Value =
        toml::from_str(input).map_err(|e| QfError::Parse(e.to_string()))?;
    Ok(toml_to_json(toml_val))
}

fn toml_to_json(val: toml::Value) -> Value {
    match val {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(i.into()),
        toml::Value::Float(f) => {
            serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(arr) => Value::Array(arr.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let map = table
                .into_iter()
                .map(|(k, v)| (k, toml_to_json(v)))
                .collect();
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_values() {
        let input = r#"
name = "test"
version = 42
enabled = true
"#;
        let val = parse(input).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["version"], 42);
        assert_eq!(val["enabled"], true);
    }

    #[test]
    fn nested_tables() {
        let input = r#"
[package]
name = "qf"
version = "0.1.0"

[package.metadata]
category = "tools"
"#;
        let val = parse(input).unwrap();
        assert_eq!(val["package"]["name"], "qf");
        assert_eq!(val["package"]["metadata"]["category"], "tools");
    }

    #[test]
    fn arrays() {
        let input = r#"
tags = ["cli", "rust", "query"]
"#;
        let val = parse(input).unwrap();
        assert_eq!(val["tags"][0], "cli");
        assert_eq!(val["tags"][2], "query");
    }

    #[test]
    fn datetimes() {
        let input = r#"
created = 2024-01-15T10:30:00Z
"#;
        let val = parse(input).unwrap();
        assert!(val["created"].as_str().unwrap().contains("2024-01-15"));
    }

    #[test]
    fn invalid_toml() {
        assert!(parse("= invalid").is_err());
    }
}
