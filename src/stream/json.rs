use std::io::{BufRead, Cursor};

use serde::de::{SeqAccess, Visitor};
use serde::Deserializer;
use serde_json::Value;

use crate::error::QfError;
use crate::query::CompiledQuery;

fn eval_and_emit<F>(
    compiled: &CompiledQuery,
    value: &Value,
    on_result: &mut F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    for result in compiled.eval(value)? {
        on_result(result)?;
    }
    Ok(())
}

fn peek_first_non_whitespace<R: BufRead>(reader: &mut R) -> Result<Option<u8>, QfError> {
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(None);
        }
        if let Some(pos) = buf
            .iter()
            .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            let byte = buf[pos];
            reader.consume(pos);
            return Ok(Some(byte));
        }
        let len = buf.len();
        reader.consume(len);
    }
}

struct ArrayStreamVisitor<'a, F> {
    compiled: &'a CompiledQuery,
    on_result: &'a mut F,
    error: &'a mut Option<QfError>,
}

impl<'de, 'a, F> Visitor<'de> for ArrayStreamVisitor<'a, F>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON array")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(elem) = seq.next_element::<Value>()? {
            if let Err(err) = eval_and_emit(self.compiled, &elem, self.on_result) {
                *self.error = Some(err);
                return Err(serde::de::Error::custom("stream aborted"));
            }
        }
        Ok(())
    }
}

fn stream_json_array_reader<R, F>(
    reader: R,
    compiled: &CompiledQuery,
    on_result: &mut F,
) -> Result<(), QfError>
where
    R: BufRead,
    F: FnMut(Value) -> Result<(), QfError>,
{
    let mut de = serde_json::Deserializer::from_reader(reader);
    let mut error: Option<QfError> = None;
    let visitor = ArrayStreamVisitor {
        compiled,
        on_result,
        error: &mut error,
    };
    let res = de.deserialize_seq(visitor);
    if let Some(err) = error {
        return Err(err);
    }
    res.map_err(|e| QfError::Parse(e.to_string()))?;
    de.end().map_err(|e| QfError::Parse(e.to_string()))?;
    Ok(())
}

/// Stream JSON from a `BufRead` source, applying the query to each top-level value
/// or each element of a top-level JSON array.
pub fn stream_json_reader<R, F>(
    mut reader: R,
    query_str: &str,
    on_result: &mut F,
) -> Result<(), QfError>
where
    R: BufRead,
    F: FnMut(Value) -> Result<(), QfError>,
{
    let compiled = CompiledQuery::compile(query_str)?;

    match peek_first_non_whitespace(&mut reader)? {
        None => Ok(()),
        Some(b'[') => stream_json_array_reader(reader, &compiled, on_result),
        Some(_) => {
            let stream = serde_json::Deserializer::from_reader(reader).into_iter::<Value>();
            for item in stream {
                let value = item.map_err(|e| QfError::Parse(e.to_string()))?;
                eval_and_emit(&compiled, &value, on_result)?;
            }
            Ok(())
        }
    }
}

/// Stream JSON from a string slice, applying the query to each top-level value
/// or each element of a top-level JSON array.
pub fn stream_json<F>(input: &str, query_str: &str, on_result: &mut F) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    stream_json_reader(Cursor::new(input.as_bytes()), query_str, on_result)
}

/// Stream newline-delimited JSON (NDJSON/JSON Lines) from a `BufRead` source.
pub fn stream_ndjson_reader<R, F>(
    mut reader: R,
    query_str: &str,
    on_result: &mut F,
) -> Result<(), QfError>
where
    R: BufRead,
    F: FnMut(Value) -> Result<(), QfError>,
{
    let compiled = CompiledQuery::compile(query_str)?;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(trimmed).map_err(|e| QfError::Parse(e.to_string()))?;
        eval_and_emit(&compiled, &value, on_result)?;
    }

    Ok(())
}

/// Stream newline-delimited JSON (NDJSON/JSON Lines) from a string slice.
pub fn stream_ndjson<F>(input: &str, query_str: &str, on_result: &mut F) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    stream_ndjson_reader(Cursor::new(input.as_bytes()), query_str, on_result)
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
        assert_eq!(results, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn stream_json_array_objects_query() {
        let input = r#"  [ {"a": 1}, {"a": 2} ]  "#;
        let mut results = Vec::new();
        stream_json_reader(Cursor::new(input.as_bytes()), ".a", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!(1), json!(2)]);
    }

    #[test]
    fn stream_json_empty_array_and_empty_input() {
        let mut results = Vec::new();
        stream_json_reader(Cursor::new(b"[]"), ".a", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert!(results.is_empty());

        stream_json_reader(Cursor::new(b"   \n\t "), ".", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert!(results.is_empty());
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

    #[test]
    fn stream_ndjson_reader_skips_blank_lines_and_handles_no_trailing_newline() {
        let input = "\n  \n{\"a\":10}\r\n\n{\"a\":20}";
        let mut results = Vec::new();
        stream_ndjson_reader(std::io::BufReader::new(input.as_bytes()), ".a", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!(10), json!(20)]);
    }

    #[test]
    fn stream_json_invalid_query_errors_even_on_empty_input() {
        let err = stream_json_reader(Cursor::new(b""), ".[[", &mut |_| Ok(())).unwrap_err();
        assert!(matches!(err, QfError::SyntaxError { .. }));
    }
}
