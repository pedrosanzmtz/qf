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
    quick_xml::se::to_string(value).map_err(|e| QfError::Parse(e.to_string()))
}

fn format_toml(value: &Value) -> Result<String, QfError> {
    let toml_val = json_to_toml(value)?;
    toml::to_string_pretty(&toml_val).map_err(|e| QfError::Parse(e.to_string()))
}

fn json_to_toml(value: &Value) -> Result<toml::Value, QfError> {
    match value {
        Value::Null => Ok(toml::Value::String("null".to_string())),
        Value::Bool(b) => Ok(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(toml::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(toml::Value::Float(f))
            } else {
                Err(QfError::Parse(format!("unsupported number: {n}")))
            }
        }
        Value::String(s) => Ok(toml::Value::String(s.clone())),
        Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr.iter().map(json_to_toml).collect();
            Ok(toml::Value::Array(items?))
        }
        Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                table.insert(k.clone(), json_to_toml(v)?);
            }
            Ok(toml::Value::Table(table))
        }
    }
}

fn format_delimited(value: &Value, delimiter: u8) -> Result<String, QfError> {
    let rows = match value {
        Value::Array(arr) => arr,
        _ => return Err(QfError::Parse("CSV/TSV output requires an array of objects".to_string())),
    };

    if rows.is_empty() {
        return Ok(String::new());
    }

    let headers: Vec<String> = match &rows[0] {
        Value::Object(map) => map.keys().cloned().collect(),
        _ => return Err(QfError::Parse("CSV/TSV output requires an array of objects".to_string())),
    };

    let mut wtr = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(vec![]);

    wtr.write_record(&headers)
        .map_err(|e| QfError::Parse(e.to_string()))?;

    for row in rows {
        let obj = row.as_object().ok_or_else(|| {
            QfError::Parse("CSV/TSV output requires an array of objects".to_string())
        })?;
        let fields: Vec<String> = headers
            .iter()
            .map(|h| match obj.get(h) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(v) => v.to_string(),
            })
            .collect();
        wtr.write_record(&fields)
            .map_err(|e| QfError::Parse(e.to_string()))?;
    }

    let bytes = wtr
        .into_inner()
        .map_err(|e| QfError::Parse(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| QfError::Parse(e.to_string()))
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
