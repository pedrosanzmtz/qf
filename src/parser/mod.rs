pub mod json;
pub mod yaml;

use serde_json::Value;

use crate::error::QfError;
use crate::format::Format;

/// Parse input text into a serde_json::Value based on format.
pub fn parse(input: &str, format: Format) -> Result<Value, QfError> {
    match format {
        Format::Yaml => yaml::parse(input),
        Format::Json => json::parse(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_yaml() {
        let val = parse("key: value", Format::Yaml).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn dispatch_json() {
        let val = parse(r#"{"key": "value"}"#, Format::Json).unwrap();
        assert_eq!(val["key"], "value");
    }
}
