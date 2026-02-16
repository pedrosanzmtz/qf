use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::Value;

use crate::error::QfError;
use crate::query;

/// Stream XML elements, applying the query to each top-level child element.
pub fn stream_xml<F>(
    input: &str,
    query_str: &str,
    on_result: &mut F,
) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    let mut reader = Reader::from_str(input);
    let mut depth: usize = 0;
    let mut current_element = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                if depth == 2 {
                    // Start collecting a top-level child element
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let mut element_xml = String::new();
                    element_xml.push('<');
                    element_xml.push_str(&tag);
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref());
                        let val = String::from_utf8_lossy(&attr.value);
                        element_xml.push(' ');
                        element_xml.push_str(&key);
                        element_xml.push_str("=\"");
                        element_xml.push_str(&val);
                        element_xml.push('"');
                    }
                    element_xml.push('>');
                    current_element = element_xml;
                } else if depth > 2 {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    current_element.push('<');
                    current_element.push_str(&tag);
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref());
                        let val = String::from_utf8_lossy(&attr.value);
                        current_element.push(' ');
                        current_element.push_str(&key);
                        current_element.push_str("=\"");
                        current_element.push_str(&val);
                        current_element.push('"');
                    }
                    current_element.push('>');
                }
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if depth == 2 {
                    current_element.push_str("</");
                    current_element.push_str(&tag);
                    current_element.push('>');

                    // Parse the collected element and apply query
                    let value: Value = quick_xml::de::from_str(&current_element)
                        .map_err(|e| QfError::Parse(e.to_string()))?;
                    let results = query::query(&value, query_str)?;
                    for result in results {
                        on_result(result)?;
                    }
                    current_element.clear();
                } else if depth > 2 {
                    current_element.push_str("</");
                    current_element.push_str(&tag);
                    current_element.push('>');
                }
                depth -= 1;
            }
            Ok(Event::Empty(ref e)) => {
                if depth >= 1 {
                    let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if depth == 1 {
                        // Self-closing top-level child
                        let mut element_xml = format!("<{}", tag);
                        for attr in e.attributes().flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref());
                            let val = String::from_utf8_lossy(&attr.value);
                            element_xml.push(' ');
                            element_xml.push_str(&key);
                            element_xml.push_str("=\"");
                            element_xml.push_str(&val);
                            element_xml.push('"');
                        }
                        element_xml.push_str("/>");

                        let value: Value = quick_xml::de::from_str(&element_xml)
                            .map_err(|e| QfError::Parse(e.to_string()))?;
                        let results = query::query(&value, query_str)?;
                        for result in results {
                            on_result(result)?;
                        }
                    } else {
                        current_element.push('<');
                        current_element.push_str(&tag);
                        for attr in e.attributes().flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref());
                            let val = String::from_utf8_lossy(&attr.value);
                            current_element.push(' ');
                            current_element.push_str(&key);
                            current_element.push_str("=\"");
                            current_element.push_str(&val);
                            current_element.push('"');
                        }
                        current_element.push_str("/>");
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if depth >= 2 {
                    let text = e.unescape().map_err(|e| QfError::Parse(e.to_string()))?;
                    // Escape for XML
                    current_element.push_str(&text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(QfError::Parse(e.to_string())),
        }
        buf.clear();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_xml_elements() {
        let input = "<root><item><name>a</name></item><item><name>b</name></item></root>";
        let mut results = Vec::new();
        stream_xml(input, ".", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results.len(), 2);
    }
}
