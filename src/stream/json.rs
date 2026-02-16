use serde_json::Value;

use crate::error::QfError;
use crate::query;

/// Stream a JSON array, applying the query to each element.
pub fn stream_json<F>(
    input: &str,
    query_str: &str,
    on_result: &mut F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    // Use serde_json::StreamDeserializer for lazy parsing
    let stream = serde_json::Deserializer::from_str(input).into_iter::<Value>();

    for item in stream {
        let value = item.map_err(|e| QfError::Parse(e.to_string()))?;
        let results = query::query(&value, query_str)?;
        for result in results {
            on_result(result)?;
        }
    }

    Ok(())
}

/// Stream newline-delimited JSON (NDJSON/JSON Lines).
pub fn stream_ndjson<F>(
    input: &str,
    query_str: &str,
    on_result: &mut F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).map_err(|e| QfError::Parse(e.to_string()))?;
        let results = query::query(&value, query_str)?;
        for result in results {
            on_result(result)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_json_array() {
        let input = r#"[1,2,3]"#;
        let mut results = Vec::new();
        stream_json(input, ".", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        // StreamDeserializer parses the whole array as one value
        assert_eq!(results, vec![json!([1, 2, 3])]);
    }

    #[test]
    fn stream_multiple_json_values() {
        let input = r#"{"a":1}{"a":2}{"a":3}"#;
        let mut results = Vec::new();
        stream_json(input, ".a", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn stream_ndjson_lines() {
        let input = "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n";
        let mut results = Vec::new();
        stream_ndjson(input, ".a", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!(1), json!(2), json!(3)]);
    }
}
