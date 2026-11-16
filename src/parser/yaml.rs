use serde::Deserialize;
use serde_json::Value;

use crate::error::QfError;

/// Parse a YAML string into a serde_json::Value.
///
/// Single-document inputs return the document value directly; empty inputs
/// return `Value::Null`; multi-document inputs return a JSON array of
/// documents. YAML merge keys (`<<`) are resolved prior to conversion.
pub fn parse(input: &str) -> Result<Value, QfError> {
    let mut docs = parse_all(input)?;
    match docs.len() {
        0 => Ok(Value::Null),
        1 => Ok(docs.remove(0)),
        _ => Ok(Value::Array(docs)),
    }
}

/// Parse a YAML string containing one or more documents into a `Vec<Value>`.
///
/// Iterates over all YAML documents using `serde_yaml_ng::Deserializer` and
/// resolves YAML merge keys (`<<`) in each document.
pub fn parse_all(input: &str) -> Result<Vec<Value>, QfError> {
    let mut docs = Vec::new();
    for doc in serde_yaml_ng::Deserializer::from_str(input) {
        let yaml_value =
            serde_yaml_ng::Value::deserialize(doc).map_err(|e| QfError::Parse(e.to_string()))?;
        let resolved = resolve_merges(yaml_value)?;
        docs.push(yaml_to_json(resolved)?);
    }
    Ok(docs)
}

/// Recursively resolve YAML 1.1 merge keys (`<<`) in `serde_yaml_ng::Value`.
///
/// Explicit keys in a mapping always take precedence over merged keys.
/// When `<<` maps to a sequence of mappings, earlier mappings in the sequence
/// take precedence over later ones.
fn resolve_merges(yaml: serde_yaml_ng::Value) -> Result<serde_yaml_ng::Value, QfError> {
    match yaml {
        serde_yaml_ng::Value::Sequence(seq) => {
            let resolved: Result<Vec<_>, _> = seq.into_iter().map(resolve_merges).collect();
            Ok(serde_yaml_ng::Value::Sequence(resolved?))
        }
        serde_yaml_ng::Value::Tagged(mut tagged) => {
            tagged.value = resolve_merges(tagged.value)?;
            Ok(serde_yaml_ng::Value::Tagged(tagged))
        }
        serde_yaml_ng::Value::Mapping(mut map) => {
            let merge_key = serde_yaml_ng::Value::String("<<".to_string());
            let merge_val = map.remove(&merge_key);

            let mut resolved = serde_yaml_ng::Mapping::new();
            for (k, v) in map {
                resolved.insert(k, resolve_merges(v)?);
            }

            if let Some(mv) = merge_val {
                match resolve_merges(mv)? {
                    serde_yaml_ng::Value::Mapping(m) => {
                        merge_into(&mut resolved, m);
                    }
                    serde_yaml_ng::Value::Sequence(seq) => {
                        for item in seq {
                            match item {
                                serde_yaml_ng::Value::Mapping(m) => merge_into(&mut resolved, m),
                                _ => {
                                    return Err(QfError::Parse(
                                        "YAML merge key '<<' sequence elements must be mappings"
                                            .into(),
                                    ))
                                }
                            }
                        }
                    }
                    _ => {
                        return Err(QfError::Parse(
                            "YAML merge key '<<' value must be a mapping or sequence of mappings"
                                .into(),
                        ))
                    }
                }
            }

            Ok(serde_yaml_ng::Value::Mapping(resolved))
        }
        other => Ok(other),
    }
}

fn merge_into(target: &mut serde_yaml_ng::Mapping, source: serde_yaml_ng::Mapping) {
    for (k, v) in source {
        if !target.contains_key(&k) {
            target.insert(k, v);
        }
    }
}

fn yaml_to_json(yaml: serde_yaml_ng::Value) -> Result<Value, QfError> {
    match yaml {
        serde_yaml_ng::Value::Null => Ok(Value::Null),
        serde_yaml_ng::Value::Bool(b) => Ok(Value::Bool(b)),
        serde_yaml_ng::Value::Number(n) => {
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
        serde_yaml_ng::Value::String(s) => Ok(Value::String(s)),
        serde_yaml_ng::Value::Sequence(seq) => {
            let items: Result<Vec<Value>, _> = seq.into_iter().map(yaml_to_json).collect();
            Ok(Value::Array(items?))
        }
        serde_yaml_ng::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml_ng::Value::String(s) => s,
                    serde_yaml_ng::Value::Number(n) => n.to_string(),
                    serde_yaml_ng::Value::Bool(b) => b.to_string(),
                    serde_yaml_ng::Value::Null => "null".to_string(),
                    _ => return Err(QfError::Parse("unsupported YAML map key type".into())),
                };
                obj.insert(key, yaml_to_json(v)?);
            }
            Ok(Value::Object(obj))
        }
        serde_yaml_ng::Value::Tagged(tagged) => yaml_to_json(tagged.value),
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

    #[test]
    fn parse_empty_yaml() {
        let val = parse("").unwrap();
        assert!(val.is_null());
        let docs = parse_all("").unwrap();
        assert_eq!(docs, vec![Value::Null]);
    }

    #[test]
    fn parse_multidoc_yaml() {
        let input = "---\nname: first\ncount: 1\n---\nname: second\ncount: 2\n";
        let docs = parse_all(input).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0]["name"], "first");
        assert_eq!(docs[0]["count"], 1);
        assert_eq!(docs[1]["name"], "second");
        assert_eq!(docs[1]["count"], 2);

        let arr = parse(input).unwrap();
        assert!(arr.is_array());
        assert_eq!(arr.as_array().unwrap().len(), 2);
        assert_eq!(arr[0]["name"], "first");
        assert_eq!(arr[1]["name"], "second");
    }

    #[test]
    fn merge_key_single_alias() {
        let input = "\
defaults: &base
  host: localhost
  port: 8080
  tls: false
service:
  <<: *base
  port: 9090
  name: api
";
        let val = parse(input).unwrap();
        assert_eq!(val["service"]["host"], "localhost");
        assert_eq!(val["service"]["port"], 9090);
        assert_eq!(val["service"]["tls"], false);
        assert_eq!(val["service"]["name"], "api");
        assert!(val["service"].get("<<").is_none());
    }

    #[test]
    fn merge_key_multi_alias_precedence() {
        let input = "\
a: &a
  x: from_a
  y: from_a
b: &b
  y: from_b
  z: from_b
merged:
  <<: [*a, *b]
  x: explicit
";
        let val = parse(input).unwrap();
        assert_eq!(val["merged"]["x"], "explicit");
        assert_eq!(val["merged"]["y"], "from_a");
        assert_eq!(val["merged"]["z"], "from_b");
        assert!(val["merged"].get("<<").is_none());
    }

    #[test]
    fn merge_key_recursive() {
        let input = "\
grandparent: &gp
  a: 1
  b: 2
parent: &p
  <<: *gp
  b: 20
  c: 30
child:
  <<: *p
  c: 300
  d: 400
";
        let val = parse(input).unwrap();
        assert_eq!(val["child"]["a"], 1);
        assert_eq!(val["child"]["b"], 20);
        assert_eq!(val["child"]["c"], 300);
        assert_eq!(val["child"]["d"], 400);
    }

    #[test]
    fn merge_key_invalid_target_errors() {
        assert!(parse("a:\n  <<: not_a_mapping").is_err());
        assert!(parse("a:\n  <<: [123]").is_err());
    }
}
