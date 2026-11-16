pub mod csv;
pub mod json;
pub mod xml;

use std::io::{BufRead, Cursor};

use serde_json::Value;

use crate::error::QfError;
use crate::format::Format;

/// Process input from a `BufRead` source in streaming mode, applying a query to each record.
/// Returns results one at a time via a callback.
pub fn stream_process_reader<R, F>(
    reader: R,
    format: Format,
    query_str: &str,
    mut on_result: F,
) -> Result<(), QfError>
where
    R: BufRead,
    F: FnMut(Value) -> Result<(), QfError>,
{
    match format {
        Format::Json => json::stream_json_reader(reader, query_str, &mut on_result),
        Format::Xml => xml::stream_xml_reader(reader, query_str, &mut on_result),
        Format::Csv => csv::stream_csv_reader(reader, query_str, b',', &mut on_result),
        Format::Tsv => csv::stream_csv_reader(reader, query_str, b'\t', &mut on_result),
        _ => Err(QfError::Runtime(format!(
            "streaming not supported for {}",
            format
        ))),
    }
}

/// Process input in streaming mode, applying a query to each record.
/// Returns results one at a time via a callback.
pub fn stream_process<F>(
    input: &str,
    format: Format,
    query_str: &str,
    on_result: F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    stream_process_reader(Cursor::new(input.as_bytes()), format, query_str, on_result)
}

/// Process NDJSON (newline-delimited JSON) input from a `BufRead` source.
pub fn stream_ndjson_reader<R, F>(
    reader: R,
    query_str: &str,
    mut on_result: F,
) -> Result<(), QfError>
where
    R: BufRead,
    F: FnMut(Value) -> Result<(), QfError>,
{
    json::stream_ndjson_reader(reader, query_str, &mut on_result)
}

/// Process NDJSON (newline-delimited JSON) input.
pub fn stream_ndjson<F>(input: &str, query_str: &str, on_result: F) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    stream_ndjson_reader(Cursor::new(input.as_bytes()), query_str, on_result)
}
