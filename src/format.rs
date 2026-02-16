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
            "json" => Ok(Format::Json),
            "xml" => Ok(Format::Xml),
            "toml" => Ok(Format::Toml),
            "csv" => Ok(Format::Csv),
            "tsv" => Ok(Format::Tsv),
            other => Err(QfError::UnknownExtension(other.to_string())),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_yaml() {
        assert_eq!(Format::from_extension(Path::new("foo.yaml")).unwrap(), Format::Yaml);
        assert_eq!(Format::from_extension(Path::new("foo.yml")).unwrap(), Format::Yaml);
        assert_eq!(Format::from_extension(Path::new("foo.YML")).unwrap(), Format::Yaml);
    }

    #[test]
    fn detect_json() {
        assert_eq!(Format::from_extension(Path::new("foo.json")).unwrap(), Format::Json);
        assert_eq!(Format::from_extension(Path::new("foo.JSON")).unwrap(), Format::Json);
    }

    #[test]
    fn no_extension_errors() {
        assert!(Format::from_extension(Path::new("foo")).is_err());
    }

    #[test]
    fn detect_xml() {
        assert_eq!(Format::from_extension(Path::new("foo.xml")).unwrap(), Format::Xml);
    }

    #[test]
    fn detect_toml() {
        assert_eq!(Format::from_extension(Path::new("foo.toml")).unwrap(), Format::Toml);
    }

    #[test]
    fn detect_csv() {
        assert_eq!(Format::from_extension(Path::new("foo.csv")).unwrap(), Format::Csv);
    }

    #[test]
    fn detect_tsv() {
        assert_eq!(Format::from_extension(Path::new("foo.tsv")).unwrap(), Format::Tsv);
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
}
