use serde_json::Value;

use crate::error::QfError;

/// Options controlling delimited (CSV/TSV) parsing behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvOptions {
    pub delimiter: u8,
    pub has_header: bool,
    pub flexible: bool,
    pub auto_types: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
            flexible: false,
            auto_types: false,
        }
    }
}

pub fn parse(input: &str) -> Result<Value, QfError> {
    parse_delimited(input, b',')
}

pub(crate) fn parse_delimited(input: &str, delimiter: u8) -> Result<Value, QfError> {
    let options = CsvOptions {
        delimiter,
        ..CsvOptions::default()
    };
    parse_delimited_with_options(input, options)
}

/// Parse delimited text (CSV/TSV) using the provided [`CsvOptions`].
pub fn parse_delimited_with_options(input: &str, options: CsvOptions) -> Result<Value, QfError> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(options.delimiter)
        .has_headers(options.has_header)
        .flexible(options.flexible)
        .from_reader(input.as_bytes());

    let mut rows = Vec::new();
    if options.has_header {
        let headers: Vec<String> = rdr
            .headers()
            .map_err(|e| QfError::Parse(e.to_string()))?
            .iter()
            .map(|h| h.to_string())
            .collect();

        for result in rdr.records() {
            let record = result.map_err(|e| QfError::Parse(e.to_string()))?;
            rows.push(record_to_object(&headers, &record, options.auto_types));
        }
    } else {
        for result in rdr.records() {
            let record = result.map_err(|e| QfError::Parse(e.to_string()))?;
            rows.push(record_to_array(&record, options.auto_types));
        }
    }

    Ok(Value::Array(rows))
}

pub(crate) fn parse_csv_field(field: &str, auto_types: bool) -> Value {
    if !auto_types {
        return Value::String(field.to_string());
    }
    if field.is_empty() {
        return Value::Null;
    }
    if field == "true" {
        return Value::Bool(true);
    }
    if field == "false" {
        return Value::Bool(false);
    }
    if let Ok(i) = field.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = field.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(field.to_string())
}

pub(crate) fn record_to_object(
    headers: &[String],
    record: &csv::StringRecord,
    auto_types: bool,
) -> Value {
    let mut obj = serde_json::Map::new();
    for (i, h) in headers.iter().enumerate() {
        let raw = record.get(i).unwrap_or("");
        obj.insert(h.clone(), parse_csv_field(raw, auto_types));
    }
    for (i, raw) in record.iter().enumerate().skip(headers.len()) {
        obj.insert(
            format!("column_{}", i + 1),
            parse_csv_field(raw, auto_types),
        );
    }
    Value::Object(obj)
}

pub(crate) fn record_to_array(record: &csv::StringRecord, auto_types: bool) -> Value {
    Value::Array(
        record
            .iter()
            .map(|f| parse_csv_field(f, auto_types))
            .collect(),
    )
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

    #[test]
    fn flexible_ragged_rows() {
        let input = "a,b,c\n1,2\n4,5,6,7\n";
        let opts = CsvOptions {
            flexible: true,
            ..CsvOptions::default()
        };
        let val = parse_delimited_with_options(input, opts).unwrap();
        let arr = val.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["a"], "1");
        assert_eq!(arr[0]["b"], "2");
        assert_eq!(arr[0]["c"], "");
        assert_eq!(arr[1]["c"], "6");
        assert_eq!(arr[1]["column_4"], "7");
    }

    #[test]
    fn auto_types_inference() {
        let input = "name,age,score,active,note\nAlice,30,9.5,true,\nBob,-7,NaN,false,ok\n";
        let opts = CsvOptions {
            auto_types: true,
            ..CsvOptions::default()
        };
        let val = parse_delimited_with_options(input, opts).unwrap();
        assert_eq!(val[0]["name"], "Alice");
        assert_eq!(val[0]["age"], 30);
        assert_eq!(val[0]["score"], 9.5);
        assert_eq!(val[0]["active"], true);
        assert_eq!(val[0]["note"], Value::Null);
        assert_eq!(val[1]["age"], -7);
        assert_eq!(val[1]["score"], "NaN");
        assert_eq!(val[1]["active"], false);
        assert_eq!(val[1]["note"], "ok");
    }

    #[test]
    fn headerless_with_auto_types() {
        let input = "Alice,30,true\nBob,25,false\n";
        let opts = CsvOptions {
            has_header: false,
            auto_types: true,
            ..CsvOptions::default()
        };
        let val = parse_delimited_with_options(input, opts).unwrap();
        assert_eq!(val[0][0], "Alice");
        assert_eq!(val[0][1], 30);
        assert_eq!(val[0][2], true);
        assert_eq!(val[1][1], 25);
    }
}
