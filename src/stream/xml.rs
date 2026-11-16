use std::io::{BufRead, Cursor};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::Value;

use crate::error::QfError;
use crate::parser::xml::ElementBuilder;
use crate::query::CompiledQuery;

/// Stream XML elements from a `BufRead` source, applying the query to each top-level child element.
pub fn stream_xml_reader<R, F>(reader: R, query_str: &str, on_result: &mut F) -> Result<(), QfError>
where
    R: BufRead,
    F: FnMut(Value) -> Result<(), QfError>,
{
    let compiled = CompiledQuery::compile(query_str)?;
    let mut reader = Reader::from_reader(reader);
    reader.config_mut().trim_text(true);

    let mut depth: usize = 0;
    let mut stack: Vec<ElementBuilder> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                if depth >= 2 {
                    stack.push(ElementBuilder::from_start(e)?);
                }
            }
            Ok(Event::End(ref e)) => {
                if depth >= 2 {
                    let end_name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                    let builder = stack.pop().ok_or_else(|| {
                        QfError::Parse(format!("unexpected closing tag </{end_name}>"))
                    })?;
                    let (name, value) = builder.finish();
                    if name != end_name {
                        return Err(QfError::Parse(format!(
                            "mismatched closing tag: expected </{name}>, found </{end_name}>"
                        )));
                    }
                    if stack.is_empty() {
                        for result in compiled.eval(&value)? {
                            on_result(result)?;
                        }
                    } else if let Some(parent) = stack.last_mut() {
                        parent.add_child(name, value);
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Empty(ref e)) => {
                if depth >= 1 {
                    let builder = ElementBuilder::from_start(e)?;
                    let (name, value) = builder.finish();
                    if depth == 1 {
                        for result in compiled.eval(&value)? {
                            on_result(result)?;
                        }
                    } else if let Some(parent) = stack.last_mut() {
                        parent.add_child(name, value);
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Some(current) = stack.last_mut() {
                    let text = e
                        .unescape()
                        .map_err(|err| QfError::Parse(err.to_string()))?;
                    if !text.is_empty() {
                        current.push_text(&text);
                    }
                }
            }
            Ok(Event::CData(ref e)) => {
                if let Some(current) = stack.last_mut() {
                    let text = String::from_utf8_lossy(e.as_ref());
                    if !text.is_empty() {
                        current.push_text(&text);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(QfError::Parse(e.to_string())),
        }
        buf.clear();
    }

    if let Some(unclosed) = stack.pop() {
        return Err(QfError::Parse(format!(
            "unclosed XML element <{}>",
            unclosed.finish().0
        )));
    }

    Ok(())
}

/// Stream XML elements, applying the query to each top-level child element.
pub fn stream_xml<F>(input: &str, query_str: &str, on_result: &mut F) -> Result<(), QfError>
where
    F: FnMut(Value) -> Result<(), QfError>,
{
    stream_xml_reader(Cursor::new(input.as_bytes()), query_str, on_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert_eq!(results[0], json!({"name": "a"}));
        assert_eq!(results[1], json!({"name": "b"}));
    }

    #[test]
    fn stream_xml_with_repeated_nested_and_attrs() {
        let input = r#"<root><order id="1"><tag>fast</tag><tag>fragile</tag></order><order id="2"/></root>"#;
        let mut results = Vec::new();
        stream_xml(input, ".", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], json!({"@id": "1", "tag": ["fast", "fragile"]}));
        assert_eq!(results[1], json!({"@id": "2"}));
    }

    #[test]
    fn stream_xml_query_field() {
        let input = "<root><user><name>Alice</name></user><user><name>Bob</name></user></root>";
        let mut results = Vec::new();
        stream_xml(input, ".name", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!("Alice"), json!("Bob")]);
    }

    #[test]
    fn stream_xml_entities_and_cdata() {
        let input = r#"<root><msg note="a &amp; b"><![CDATA[hello <world>]]></msg></root>"#;
        let mut results = Vec::new();
        stream_xml(input, ".", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            results,
            vec![json!({"@note": "a & b", "$text": "hello <world>"})]
        );
    }

    #[test]
    fn stream_xml_reader_bufread() {
        let input = "<root><item><v>1</v></item><item><v>2</v></item></root>";
        let mut results = Vec::new();
        stream_xml_reader(std::io::BufReader::new(input.as_bytes()), ".v", &mut |v| {
            results.push(v);
            Ok(())
        })
        .unwrap();
        assert_eq!(results, vec![json!("1"), json!("2")]);
    }

    #[test]
    fn stream_xml_reader_invalid_query_errors_on_empty_input() {
        let err = stream_xml_reader(Cursor::new(b""), ".[[", &mut |_| Ok(())).unwrap_err();
        assert!(matches!(err, QfError::SyntaxError { .. }));
    }
}
