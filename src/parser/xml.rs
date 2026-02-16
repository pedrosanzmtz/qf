use serde_json::Value;

use crate::error::QfError;

pub fn parse(input: &str) -> Result<Value, QfError> {
    quick_xml::de::from_str(input).map_err(|e| QfError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_elements() {
        let input = "<root><name>test</name><count>42</count></root>";
        let val = parse(input).unwrap();
        // quick-xml wraps text content as {"$text": "value"}
        assert_eq!(val["name"]["$text"], "test");
        assert_eq!(val["count"]["$text"], "42");
    }

    #[test]
    fn attributes() {
        let input = r#"<root><item id="1">hello</item></root>"#;
        let val = parse(input).unwrap();
        assert_eq!(val["item"]["@id"], "1");
        assert_eq!(val["item"]["$text"], "hello");
    }

    #[test]
    fn nested_structure() {
        let input = "<root><parent><child>value</child></parent></root>";
        let val = parse(input).unwrap();
        assert_eq!(val["parent"]["child"]["$text"], "value");
    }

    #[test]
    fn malformed_xml() {
        assert!(parse("<root><unclosed>").is_err());
    }
}
