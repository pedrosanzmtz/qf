use serde_json::Value;

use crate::error::QfError;

/// Parse a YAML string into a serde_json::Value.
///
/// We parse via serde_yaml then convert to serde_json::Value so the rest
/// of the pipeline works with a single value type.
pub fn parse(input: &str) -> Result<Value, QfError> {
    let yaml_value: serde_yaml::Value =
        serde_yaml::from_str(input).map_err(|e| QfError::Parse(e.to_string()))?;
    yaml_to_json(yaml_value)
}

fn yaml_to_json(yaml: serde_yaml::Value) -> Result<Value, QfError> {
    match yaml {
        serde_yaml::Value::Null => Ok(Value::Null),
        serde_yaml::Value::Bool(b) => Ok(Value::Bool(b)),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Number(i.into()))
            } else if let Some(u) = n.as_u64() {
                Ok(Value::Number(u.into()))
            } else if let Some(f) = n.as_f64() {
                Ok(serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or(Value::Null))
            } else {
                Ok(Value::Null)
            }
        }
        serde_yaml::Value::String(s) => Ok(Value::String(s)),
        serde_yaml::Value::Sequence(seq) => {
            let items: Result<Vec<Value>, _> = seq.into_iter().map(yaml_to_json).collect();
            Ok(Value::Array(items?))
        }
        serde_yaml::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    serde_yaml::Value::Null => "null".to_string(),
                    _ => return Err(QfError::Parse("unsupported YAML map key type".into())),
                };
                obj.insert(key, yaml_to_json(v)?);
            }
            Ok(Value::Object(obj))
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let val = parse("name: hello\ncount: 42").unwrap();
        assert_eq!(val["name"], "hello");
        assert_eq!(val["count"], 42);
    }

    #[test]
    fn parse_nested() {
        let input = "parent:\n  child:\n    value: deep";
        let val = parse(input).unwrap();
        assert_eq!(val["parent"]["child"]["value"], "deep");
    }

    #[test]
    fn parse_array() {
        let input = "items:\n  - one\n  - two\n  - three";
        let val = parse(input).unwrap();
        assert_eq!(val["items"][0], "one");
        assert_eq!(val["items"][2], "three");
    }

    #[test]
    fn parse_boolean_and_null() {
        let input = "flag: true\nempty: null";
        let val = parse(input).unwrap();
        assert_eq!(val["flag"], true);
        assert!(val["empty"].is_null());
    }

    #[test]
    fn invalid_yaml_errors() {
        assert!(parse("key: [unterminated").is_err());
    }

    #[test]
    fn roundtrip_yaml_json() {
        let input = "a: 1\nb:\n  - x\n  - y";
        let val = parse(input).unwrap();
        let json_str = serde_json::to_string(&val).unwrap();
        let back: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(val, back);
    }
}
