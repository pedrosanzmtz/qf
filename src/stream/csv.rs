use serde_json::Value;

use crate::error::QfError;
use crate::query;

/// Stream CSV/TSV rows, applying the query to each row (as a JSON object with header keys).
pub fn stream_csv<F>(
    input: &str,
    query_str: &str,
    delimiter: u8,
    on_result: &mut F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(input.as_bytes());

    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| QfError::Parse(e.to_string()))?
        .iter()
        .map(|h| h.to_string())
        .collect();

    for result in rdr.records() {
        let record = result.map_err(|e| QfError::Parse(e.to_string()))?;
        let obj: serde_json::Map<String, Value> = headers
            .iter()
            .zip(record.iter())
            .map(|(h, v)| (h.clone(), Value::String(v.to_string())))
            .collect();
        let value = Value::Object(obj);
        let results = query::query(&value, query_str)?;
        for r in results {
            on_result(r)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_csv_rows() {
        let input = "name,age\nAlice,30\nBob,25\n";
        let mut results = Vec::new();
        stream_csv(input, ".name", b',', &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!("Alice"), json!("Bob")]);
    }

    #[test]
    fn stream_tsv_rows() {
        let input = "name\tage\nAlice\t30\nBob\t25\n";
        let mut results = Vec::new();
        stream_csv(input, ".age", b'\t', &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!("30"), json!("25")]);
    }

    #[test]
    fn stream_csv_identity() {
        let input = "x,y\n1,2\n3,4\n";
        let mut results = Vec::new();
        stream_csv(input, ".", b',', &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["x"], "1");
        assert_eq!(results[1]["y"], "4");
    }
}
