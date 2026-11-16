use serde_json::Value;

use crate::error::QfError;
use crate::format::Format;

/// Format a Value as a string in the given format.
pub fn format_value(
    value: &Value,
    format: Format,
    compact: bool,
    raw: bool,
) -> Result<String, QfError> {
    format_value_colored(value, format, compact, raw, false)
}

/// Format a Value as a string in the given format, with optional colorization.
pub fn format_value_colored(
    value: &Value,
    format: Format,
    compact: bool,
    raw: bool,
    colorize: bool,
) -> Result<String, QfError> {
    // Raw mode: if the value is a string, output it without quotes
    if raw {
        if let Value::String(s) = value {
            return Ok(s.clone());
        }
    }

    if colorize && !compact {
        match format {
            Format::Json => return Ok(super::color::colorize_json(value)),
            Format::Yaml => {
                let yaml = format_yaml(value)?;
                return Ok(super::color::colorize_yaml(&yaml));
            }
            _ => {} // fall through to non-colorized for other formats
        }
    }

    match format {
        Format::Json => format_json(value, compact),
        Format::Yaml => format_yaml(value),
        Format::Xml => format_xml(value),
        Format::Toml => format_toml(value),
        Format::Csv => format_delimited(value, b','),
        Format::Tsv => format_delimited(value, b'\t'),
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

fn format_xml(value: &Value) -> Result<String, QfError> {
    crate::parser::xml::to_xml(value)
}

fn format_toml(value: &Value) -> Result<String, QfError> {
    match value {
        Value::Object(_) => {
            let toml_val = json_to_toml(value)?;
            toml::to_string_pretty(&toml_val).map_err(|e| QfError::Serialize(e.to_string()))
        }
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Null => Ok("null".to_string()),
        Value::Array(_) => {
            serde_json::to_string_pretty(value).map_err(|e| QfError::Serialize(e.to_string()))
        }
    }
}

fn json_to_toml(value: &Value) -> Result<toml::Value, QfError> {
    match value {
        Value::Null => Err(QfError::Serialize(
            "TOML does not support null values".to_string(),
        )),
        Value::Bool(b) => Ok(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(toml::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(toml::Value::Float(f))
            } else {
                Err(QfError::Serialize(format!("unsupported number: {n}")))
            }
        }
        Value::String(s) => {
            // Preserve RFC3339 datetimes (date + time) as native TOML datetimes on round-trip.
            if let Ok(dt) = s.parse::<toml::value::Datetime>() {
                if dt.date.is_some() && dt.time.is_some() {
                    return Ok(toml::Value::Datetime(dt));
                }
            }
            Ok(toml::Value::String(s.clone()))
        }
        Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr.iter().map(json_to_toml).collect();
            Ok(toml::Value::Array(items?))
        }
        Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                if !v.is_null() {
                    table.insert(k.clone(), json_to_toml(v)?);
                }
            }
            Ok(toml::Value::Table(table))
        }
    }
}

fn format_csv_cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn format_delimited(value: &Value, delimiter: u8) -> Result<String, QfError> {
    let rows = match value {
        Value::Array(arr) => arr,
        _ => {
            return Err(QfError::Serialize(
                "CSV/TSV output requires an array of objects or arrays".to_string(),
            ))
        }
    };

    if rows.is_empty() {
        return Ok(String::new());
    }

    let mut wtr = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_writer(vec![]);

    match &rows[0] {
        Value::Object(_) => {
            let mut headers: Vec<String> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for row in rows {
                let obj = row.as_object().ok_or_else(|| {
                    QfError::Serialize("CSV/TSV output requires all rows to be objects".to_string())
                })?;
                for k in obj.keys() {
                    if seen.insert(k.clone()) {
                        headers.push(k.clone());
                    }
                }
            }

            wtr.write_record(&headers)
                .map_err(|e| QfError::Serialize(e.to_string()))?;

            for row in rows {
                let obj = row.as_object().ok_or_else(|| {
                    QfError::Serialize("CSV/TSV output requires all rows to be objects".to_string())
                })?;
                let fields: Vec<String> = headers
                    .iter()
                    .map(|h| obj.get(h).map_or_else(String::new, format_csv_cell))
                    .collect();
                wtr.write_record(&fields)
                    .map_err(|e| QfError::Serialize(e.to_string()))?;
            }
        }
        Value::Array(_) => {
            for row in rows {
                let arr = row.as_array().ok_or_else(|| {
                    QfError::Serialize("CSV/TSV output requires all rows to be arrays".to_string())
                })?;
                let fields: Vec<String> = arr.iter().map(format_csv_cell).collect();
                wtr.write_record(&fields)
                    .map_err(|e| QfError::Serialize(e.to_string()))?;
            }
        }
        _ => {
            return Err(QfError::Serialize(
                "CSV/TSV output requires an array of objects or arrays".to_string(),
            ))
        }
    }

    let bytes = wtr
        .into_inner()
        .map_err(|e| QfError::Serialize(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| QfError::Serialize(e.to_string()))
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

    #[test]
    fn toml_scalar_and_array_output() {
        assert_eq!(
            format_value(&json!("hello"), Format::Toml, false, false).unwrap(),
            "hello"
        );
        assert_eq!(
            format_value(&json!(42), Format::Toml, false, false).unwrap(),
            "42"
        );
        assert_eq!(
            format_value(&json!(true), Format::Toml, false, false).unwrap(),
            "true"
        );
        assert_eq!(
            format_value(&json!(null), Format::Toml, false, false).unwrap(),
            "null"
        );
        let arr_out = format_value(&json!([1, 2, 3]), Format::Toml, false, false).unwrap();
        let parsed: Value = serde_json::from_str(&arr_out).unwrap();
        assert_eq!(parsed, json!([1, 2, 3]));
    }

    #[test]
    fn toml_omits_null_keys() {
        let val = json!({
            "name": "qf",
            "empty": null,
            "nested": {
                "keep": 1,
                "drop": null
            }
        });
        let out = format_value(&val, Format::Toml, false, false).unwrap();
        assert!(out.contains("name = \"qf\""));
        assert!(!out.contains("empty"));
        assert!(out.contains("keep = 1"));
        assert!(!out.contains("drop"));
    }

    #[test]
    fn toml_null_in_array_returns_serialize_error() {
        let val = json!({"items": [1, null, 2]});
        let err = format_value(&val, Format::Toml, false, false).unwrap_err();
        assert!(matches!(err, QfError::Serialize(_)));
    }

    #[test]
    fn toml_datetime_roundtrip() {
        let input = "created = 2024-01-15T10:30:00Z\n";
        let parsed = crate::parser::toml::parse(input).unwrap();
        let out = format_value(&parsed, Format::Toml, false, false).unwrap();
        assert_eq!(out, "created = 2024-01-15T10:30:00Z\n");
    }

    #[test]
    fn csv_union_headers_first_seen_order() {
        let val = json!([
            {"a": 1, "b": 2},
            {"b": 3, "c": 4},
            {"d": 5, "a": 6}
        ]);
        let out = format_value(&val, Format::Csv, false, false).unwrap();
        assert_eq!(out, "a,b,c,d\n1,2,,\n,3,4,\n6,,,5\n");
    }

    #[test]
    fn csv_and_tsv_array_of_arrays_headerless() {
        let val = json!([["Alice", 30, true], ["Bob", 25, null]]);
        let csv_out = format_value(&val, Format::Csv, false, false).unwrap();
        assert_eq!(csv_out, "Alice,30,true\nBob,25,\n");

        let tsv_out = format_value(&val, Format::Tsv, false, false).unwrap();
        assert_eq!(tsv_out, "Alice\t30\ttrue\nBob\t25\t\n");
    }
}
