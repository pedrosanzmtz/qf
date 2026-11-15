use std::path::Path;

use crate::error::QfError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Json,
    Xml,
    Toml,
    Csv,
    Tsv,
}

impl Format {
    /// Detect format from a file extension.
    pub fn from_extension(path: &Path) -> Result<Self, QfError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or(QfError::NoExtension)?;

        match ext.to_ascii_lowercase().as_str() {
            "yaml" | "yml" => Ok(Format::Yaml),
            "json" | "jsonl" | "ndjson" => Ok(Format::Json),
            "xml" => Ok(Format::Xml),
            "toml" => Ok(Format::Toml),
            "csv" => Ok(Format::Csv),
            "tsv" => Ok(Format::Tsv),
            other => Err(QfError::UnknownExtension(other.to_string())),
        }
    }

    /// Returns true if the path has a `.jsonl` or `.ndjson` extension.
    pub fn is_ndjson_extension(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "jsonl" | "ndjson"))
            .unwrap_or(false)
    }

    /// Detect format from content when file extension is missing or unknown.
    pub fn detect_from_content(input: &str) -> Self {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Format::Yaml;
        }

        // Valid JSON (single value, including scalars like "hi", 42, true, null,
        // or a stream of multiple JSON values).
        if is_valid_json_or_stream(trimmed) {
            return Format::Json;
        }

        if trimmed.starts_with('<') {
            return Format::Xml;
        }

        if looks_like_toml(trimmed) && toml::from_str::<toml::Value>(trimmed).is_ok() {
            return Format::Toml;
        }

        if trimmed.starts_with('{') {
            return Format::Json;
        }

        Format::Yaml
    }

    /// Parse a format string from CLI flags.
    pub fn from_str_name(s: &str) -> Result<Self, QfError> {
        match s.to_ascii_lowercase().as_str() {
            "yaml" | "yml" => Ok(Format::Yaml),
            "json" => Ok(Format::Json),
            "xml" => Ok(Format::Xml),
            "toml" => Ok(Format::Toml),
            "csv" => Ok(Format::Csv),
            "tsv" => Ok(Format::Tsv),
            other => Err(QfError::UnsupportedFormat(other.to_string())),
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::Yaml => write!(f, "yaml"),
            Format::Json => write!(f, "json"),
            Format::Xml => write!(f, "xml"),
            Format::Toml => write!(f, "toml"),
            Format::Csv => write!(f, "csv"),
            Format::Tsv => write!(f, "tsv"),
        }
    }
}

fn is_valid_json_or_stream(trimmed: &str) -> bool {
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return true;
    }
    let mut count = 0usize;
    for item in serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>() {
        match item {
            Ok(_) => count += 1,
            Err(_) => return false,
        }
    }
    count > 0
}

fn looks_like_toml(trimmed: &str) -> bool {
    trimmed.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        if line.starts_with('[') && line.ends_with(']') {
            return true;
        }
        if let Some((lhs, _)) = line.split_once('=') {
            let key = lhs.trim();
            return !key.is_empty() && !key.starts_with('-');
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_yaml() {
        assert_eq!(
            Format::from_extension(Path::new("foo.yaml")).unwrap(),
            Format::Yaml
        );
        assert_eq!(
            Format::from_extension(Path::new("foo.yml")).unwrap(),
            Format::Yaml
        );
        assert_eq!(
            Format::from_extension(Path::new("foo.YML")).unwrap(),
            Format::Yaml
        );
    }

    #[test]
    fn detect_json() {
        assert_eq!(
            Format::from_extension(Path::new("foo.json")).unwrap(),
            Format::Json
        );
        assert_eq!(
            Format::from_extension(Path::new("foo.JSON")).unwrap(),
            Format::Json
        );
    }

    #[test]
    fn no_extension_errors() {
        assert!(Format::from_extension(Path::new("foo")).is_err());
    }

    #[test]
    fn detect_xml() {
        assert_eq!(
            Format::from_extension(Path::new("foo.xml")).unwrap(),
            Format::Xml
        );
    }

    #[test]
    fn detect_toml() {
        assert_eq!(
            Format::from_extension(Path::new("foo.toml")).unwrap(),
            Format::Toml
        );
    }

    #[test]
    fn detect_csv() {
        assert_eq!(
            Format::from_extension(Path::new("foo.csv")).unwrap(),
            Format::Csv
        );
    }

    #[test]
    fn detect_tsv() {
        assert_eq!(
            Format::from_extension(Path::new("foo.tsv")).unwrap(),
            Format::Tsv
        );
    }

    #[test]
    fn unknown_extension_errors() {
        assert!(Format::from_extension(Path::new("foo.xyz")).is_err());
    }

    #[test]
    fn from_str_name() {
        assert_eq!(Format::from_str_name("yaml").unwrap(), Format::Yaml);
        assert_eq!(Format::from_str_name("json").unwrap(), Format::Json);
        assert_eq!(Format::from_str_name("xml").unwrap(), Format::Xml);
        assert_eq!(Format::from_str_name("toml").unwrap(), Format::Toml);
        assert_eq!(Format::from_str_name("csv").unwrap(), Format::Csv);
        assert_eq!(Format::from_str_name("tsv").unwrap(), Format::Tsv);
        assert!(Format::from_str_name("xyz").is_err());
    }

    #[test]
    fn detect_ndjson_extensions() {
        assert_eq!(
            Format::from_extension(Path::new("data.jsonl")).unwrap(),
            Format::Json
        );
        assert_eq!(
            Format::from_extension(Path::new("data.ndjson")).unwrap(),
            Format::Json
        );
        assert!(Format::is_ndjson_extension(Path::new("data.jsonl")));
        assert!(Format::is_ndjson_extension(Path::new("data.NDJSON")));
        assert!(!Format::is_ndjson_extension(Path::new("data.json")));
    }

    #[test]
    fn detect_from_content_json_scalars_and_containers() {
        assert_eq!(Format::detect_from_content(r#"{"a": 1}"#), Format::Json);
        assert_eq!(Format::detect_from_content("[1, 2, 3]"), Format::Json);
        assert_eq!(Format::detect_from_content(r#""hi""#), Format::Json);
        assert_eq!(Format::detect_from_content("42"), Format::Json);
        assert_eq!(Format::detect_from_content("true"), Format::Json);
        assert_eq!(Format::detect_from_content("null"), Format::Json);
        assert_eq!(
            Format::detect_from_content("{\"a\":1}\n{\"b\":2}"),
            Format::Json
        );
    }

    #[test]
    fn detect_from_content_xml_toml_yaml() {
        assert_eq!(
            Format::detect_from_content("<root><a/></root>"),
            Format::Xml
        );
        assert_eq!(
            Format::detect_from_content("[package]\nname = \"qf\""),
            Format::Toml
        );
        assert_eq!(
            Format::detect_from_content("key = \"value\"\nnum = 1"),
            Format::Toml
        );
        assert_eq!(Format::detect_from_content("a: 1\nb: hello"), Format::Yaml);
        assert_eq!(Format::detect_from_content(""), Format::Yaml);
    }
}
