pub mod csv;
pub mod json;
pub mod xml;

use serde_json::Value;

use crate::error::QfError;
use crate::format::Format;

/// Process input in streaming mode, applying a query to each record.
/// Returns results one at a time via a callback.
pub fn stream_process<F>(
    input: &str,
    format: Format,
    query_str: &str,
    mut on_result: F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    match format {
        Format::Json => json::stream_json(input, query_str, &mut on_result),
        Format::Xml => xml::stream_xml(input, query_str, &mut on_result),
        Format::Csv => csv::stream_csv(input, query_str, b',', &mut on_result),
        Format::Tsv => csv::stream_csv(input, query_str, b'\t', &mut on_result),
        _ => Err(QfError::Runtime(format!(
            "streaming not supported for {}",
            format
        ))),
    }
}

/// Process NDJSON (newline-delimited JSON) input.
pub fn stream_ndjson<F>(
    input: &str,
    query_str: &str,
    mut on_result: F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    json::stream_ndjson(input, query_str, &mut on_result)
}
