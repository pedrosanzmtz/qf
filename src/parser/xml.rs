use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use serde_json::map::Entry;
use serde_json::{Map, Value};

use crate::error::QfError;

/// Accumulates attributes, child elements, and text content for a single XML element.
pub(crate) struct ElementBuilder {
    name: String,
    attrs: Map<String, Value>,
    children: Map<String, Value>,
    text: String,
}

impl ElementBuilder {
    pub(crate) fn from_start(e: &BytesStart<'_>) -> Result<Self, QfError> {
        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
        let mut attrs = Map::new();
        for attr_res in e.attributes() {
            let attr = attr_res.map_err(|err| QfError::Parse(err.to_string()))?;
            let key = format!("@{}", String::from_utf8_lossy(attr.key.as_ref()));
            let val = attr
                .unescape_value()
                .map_err(|err| QfError::Parse(err.to_string()))?
                .into_owned();
            attrs.insert(key, Value::String(val));
        }
        Ok(Self {
            name,
            attrs,
            children: Map::new(),
            text: String::new(),
        })
    }

    pub(crate) fn push_text(&mut self, text: &str) {
        self.text.push_str(text);
    }

    pub(crate) fn add_child(&mut self, child_name: String, child_value: Value) {
        match self.children.entry(child_name) {
            Entry::Vacant(e) => {
                e.insert(child_value);
            }
            Entry::Occupied(mut e) => {
                let existing = e.get_mut();
                if let Value::Array(arr) = existing {
                    arr.push(child_value);
                } else {
                    let prev = existing.take();
                    *existing = Value::Array(vec![prev, child_value]);
                }
            }
        }
    }

    pub(crate) fn finish(self) -> (String, Value) {
        let val = if self.attrs.is_empty() && self.children.is_empty() {
            if self.text.is_empty() {
                Value::Null
            } else {
                Value::String(self.text)
            }
        } else {
            let mut map = self.attrs;
            for (k, v) in self.children {
                map.insert(k, v);
            }
            if !self.text.is_empty() {
                map.insert("$text".to_string(), Value::String(self.text));
            }
            Value::Object(map)
        };
        (self.name, val)
    }
}

/// Parse an XML document into a `serde_json::Value`, preserving the root element name,
/// collecting repeated sibling elements into arrays, storing attributes as `@attr`,
/// and using plain strings for text-only elements (or `$text` when combined with attributes/children).
pub fn parse(input: &str) -> Result<Value, QfError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<ElementBuilder> = Vec::new();
    let mut root: Option<(String, Value)> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if stack.is_empty() && root.is_some() {
                    return Err(QfError::Parse("multiple root elements in XML".to_string()));
                }
                stack.push(ElementBuilder::from_start(e)?);
            }
            Ok(Event::Empty(ref e)) => {
                let builder = ElementBuilder::from_start(e)?;
                let (name, val) = builder.finish();
                if let Some(parent) = stack.last_mut() {
                    parent.add_child(name, val);
                } else if root.is_some() {
                    return Err(QfError::Parse("multiple root elements in XML".to_string()));
                } else {
                    root = Some((name, val));
                }
            }
            Ok(Event::End(ref e)) => {
                let end_name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let builder = stack.pop().ok_or_else(|| {
                    QfError::Parse(format!("unexpected closing tag </{end_name}>"))
                })?;
                if builder.name != end_name {
                    return Err(QfError::Parse(format!(
                        "mismatched closing tag: expected </{}>, found </{end_name}>",
                        builder.name
                    )));
                }
                let (name, val) = builder.finish();
                if let Some(parent) = stack.last_mut() {
                    parent.add_child(name, val);
                } else {
                    root = Some((name, val));
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
            unclosed.name
        )));
    }

    let (root_name, root_val) =
        root.ok_or_else(|| QfError::Parse("empty XML document".to_string()))?;
    let mut map = Map::new();
    map.insert(root_name, root_val);
    Ok(Value::Object(map))
}

/// Serialize a `serde_json::Value` into a pretty-printed XML string.
pub fn to_xml(value: &Value) -> Result<String, QfError> {
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);

    match value {
        Value::Object(map) if map.len() == 1 => {
            let (key, val) = map.iter().next().expect("checked len == 1");
            if !key.starts_with('@') && key != "$text" && !val.is_array() {
                write_element(&mut writer, key, val)?;
            } else {
                write_element(&mut writer, "root", value)?;
            }
        }
        Value::Array(arr) => {
            writer
                .write_event(Event::Start(BytesStart::new("root")))
                .map_err(|e| QfError::Parse(e.to_string()))?;
            for item in arr {
                write_element(&mut writer, "item", item)?;
            }
            writer
                .write_event(Event::End(BytesEnd::new("root")))
                .map_err(|e| QfError::Parse(e.to_string()))?;
        }
        _ => {
            write_element(&mut writer, "root", value)?;
        }
    }

    let bytes = writer.into_inner();
    String::from_utf8(bytes).map_err(|e| QfError::Parse(e.to_string()))
}

fn scalar_to_string(value: &Value) -> Result<String, QfError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::String(s) => Ok(s.clone()),
        other => serde_json::to_string(other).map_err(|e| QfError::Parse(e.to_string())),
    }
}

fn write_element(writer: &mut Writer<Vec<u8>>, name: &str, value: &Value) -> Result<(), QfError> {
    match value {
        Value::Null => {
            writer
                .write_event(Event::Empty(BytesStart::new(name)))
                .map_err(|e| QfError::Parse(e.to_string()))?;
        }
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            let text = scalar_to_string(value)?;
            writer
                .write_event(Event::Start(BytesStart::new(name)))
                .map_err(|e| QfError::Parse(e.to_string()))?;
            if !text.is_empty() {
                writer
                    .write_event(Event::Text(BytesText::new(&text)))
                    .map_err(|e| QfError::Parse(e.to_string()))?;
            }
            writer
                .write_event(Event::End(BytesEnd::new(name)))
                .map_err(|e| QfError::Parse(e.to_string()))?;
        }
        Value::Array(arr) => {
            for item in arr {
                if item.is_array() {
                    writer
                        .write_event(Event::Start(BytesStart::new(name)))
                        .map_err(|e| QfError::Parse(e.to_string()))?;
                    write_element(writer, "item", item)?;
                    writer
                        .write_event(Event::End(BytesEnd::new(name)))
                        .map_err(|e| QfError::Parse(e.to_string()))?;
                } else {
                    write_element(writer, name, item)?;
                }
            }
        }
        Value::Object(map) => {
            let mut elem = BytesStart::new(name);
            let mut text_val: Option<&Value> = None;
            let mut children: Vec<(&String, &Value)> = Vec::new();

            for (k, v) in map {
                if let Some(attr_name) = k.strip_prefix('@') {
                    let attr_str = scalar_to_string(v)?;
                    elem.push_attribute((attr_name, attr_str.as_str()));
                } else if k == "$text" {
                    text_val = Some(v);
                } else {
                    children.push((k, v));
                }
            }

            if children.is_empty() && text_val.is_none() {
                writer
                    .write_event(Event::Empty(elem))
                    .map_err(|e| QfError::Parse(e.to_string()))?;
            } else {
                writer
                    .write_event(Event::Start(elem))
                    .map_err(|e| QfError::Parse(e.to_string()))?;
                if let Some(tv) = text_val {
                    let text = scalar_to_string(tv)?;
                    if !text.is_empty() {
                        writer
                            .write_event(Event::Text(BytesText::new(&text)))
                            .map_err(|e| QfError::Parse(e.to_string()))?;
                    }
                }
                for (child_name, child_val) in children {
                    write_element(writer, child_name, child_val)?;
                }
                writer
                    .write_event(Event::End(BytesEnd::new(name)))
                    .map_err(|e| QfError::Parse(e.to_string()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn simple_elements() {
        let input = "<root><name>test</name><count>42</count></root>";
        let val = parse(input).unwrap();
        assert_eq!(val["root"]["name"], "test");
        assert_eq!(val["root"]["count"], "42");
    }

    #[test]
    fn attributes() {
        let input = r#"<root><item id="1" active="true">hello</item></root>"#;
        let val = parse(input).unwrap();
        assert_eq!(val["root"]["item"]["@id"], "1");
        assert_eq!(val["root"]["item"]["@active"], "true");
        assert_eq!(val["root"]["item"]["$text"], "hello");
    }

    #[test]
    fn nested_structure() {
        let input = "<root><parent><child>value</child></parent></root>";
        let val = parse(input).unwrap();
        assert_eq!(val["root"]["parent"]["child"], "value");
    }

    #[test]
    fn repeated_sibling_elements() {
        let input = r#"
            <root>
                <item id="1">first</item>
                <item id="2">second</item>
                <item id="3">third</item>
            </root>
        "#;
        let val = parse(input).unwrap();
        let items = val["root"]["item"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["@id"], "1");
        assert_eq!(items[0]["$text"], "first");
        assert_eq!(items[1]["@id"], "2");
        assert_eq!(items[1]["$text"], "second");
        assert_eq!(items[2]["@id"], "3");
        assert_eq!(items[2]["$text"], "third");
    }

    #[test]
    fn text_only_vs_mixed_content() {
        let input = r#"
            <root>
                <host>localhost</host>
                <port>8080</port>
                <endpoint tls="true">https://localhost</endpoint>
            </root>
        "#;
        let val = parse(input).unwrap();
        assert_eq!(val["root"]["host"], json!("localhost"));
        assert_eq!(val["root"]["port"], json!("8080"));
        assert_eq!(
            val["root"]["endpoint"],
            json!({"@tls": "true", "$text": "https://localhost"})
        );
    }

    #[test]
    fn entities_and_cdata() {
        let input = r#"<root><escaped attr="a &amp; b">1 &lt; 2 &amp; 3 &gt; 0</escaped><raw><![CDATA[<tag> & "raw"]]></raw></root>"#;
        let val = parse(input).unwrap();
        assert_eq!(val["root"]["escaped"]["@attr"], "a & b");
        assert_eq!(val["root"]["escaped"]["$text"], "1 < 2 & 3 > 0");
        assert_eq!(val["root"]["raw"], r#"<tag> & "raw""#);
    }

    #[test]
    fn empty_elements() {
        let input = r#"<root><empty/><flagged enabled="false"/></root>"#;
        let val = parse(input).unwrap();
        assert_eq!(val["root"]["empty"], Value::Null);
        assert_eq!(val["root"]["flagged"], json!({"@enabled": "false"}));
    }

    #[test]
    fn malformed_xml() {
        assert!(parse("<root><unclosed>").is_err());
        assert!(parse("<root></wrong></root>").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn xml_round_trip() {
        let input = r#"<config><host>localhost</host><server id="1">primary</server><server id="2">backup</server><empty/></config>"#;
        let parsed = parse(input).unwrap();
        let serialized = to_xml(&parsed).unwrap();
        let reparsed = parse(&serialized).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn json_to_xml() {
        let json_obj = json!({
            "name": "Alice",
            "age": 30,
            "roles": ["admin", "dev"]
        });
        let xml = to_xml(&json_obj).unwrap();
        let reparsed = parse(&xml).unwrap();
        assert_eq!(reparsed["root"]["name"], "Alice");
        assert_eq!(reparsed["root"]["age"], "30");
        assert_eq!(reparsed["root"]["roles"], json!(["admin", "dev"]));

        let json_arr = json!([1, 2, 3]);
        let arr_xml = to_xml(&json_arr).unwrap();
        let arr_reparsed = parse(&arr_xml).unwrap();
        assert_eq!(arr_reparsed["root"]["item"], json!(["1", "2", "3"]));
    }
}
