use serde_json::Value;

use crate::error::QfError;

pub fn parse(input: &str) -> Result<Value, QfError> {
    parse_delimited(input, b',')
}

pub(crate) fn parse_delimited(input: &str, delimiter: u8) -> Result<Value, QfError> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(input.as_bytes());

    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| QfError::Parse(e.to_string()))?
        .iter()
        .map(|h| h.to_string())
        .collect();

    let mut rows = Vec::new();
    for result in rdr.records() {
        let record = result.map_err(|e| QfError::Parse(e.to_string()))?;
        let obj: serde_json::Map<String, Value> = headers
            .iter()
            .zip(record.iter())
            .map(|(h, v)| (h.clone(), Value::String(v.to_string())))
            .collect();
        rows.push(Value::Object(obj));
    }

    Ok(Value::Array(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_csv() {
        let input = "name,age,city\nAlice,30,NYC\nBob,25,LA\n";
        let val = parse(input).unwrap();
        let arr = val.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "Alice");
        assert_eq!(arr[0]["age"], "30");
        assert_eq!(arr[1]["city"], "LA");
    }

    #[test]
    fn quoted_fields() {
        let input = "name,desc\nAlice,\"has, commas\"\n";
        let val = parse(input).unwrap();
        assert_eq!(val[0]["desc"], "has, commas");
    }

    #[test]
    fn empty_fields() {
        let input = "a,b,c\n1,,3\n";
        let val = parse(input).unwrap();
        assert_eq!(val[0]["a"], "1");
        assert_eq!(val[0]["b"], "");
        assert_eq!(val[0]["c"], "3");
    }

    #[test]
    fn single_row() {
        let input = "x,y\n10,20\n";
        let val = parse(input).unwrap();
        assert_eq!(val.as_array().unwrap().len(), 1);
    }
}
