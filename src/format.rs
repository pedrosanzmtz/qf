use std::path::Path;

use crate::error::QfError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Json,
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
            other => Err(QfError::UnknownExtension(other.to_string())),
        }
    }

    /// Parse a format string from CLI flags.
    pub fn from_str_name(s: &str) -> Result<Self, QfError> {
        match s.to_ascii_lowercase().as_str() {
            "yaml" | "yml" => Ok(Format::Yaml),
            "json" => Ok(Format::Json),
            other => Err(QfError::UnsupportedFormat(other.to_string())),
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::Yaml => write!(f, "yaml"),
            Format::Json => write!(f, "json"),
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
    fn unknown_extension_errors() {
        assert!(Format::from_extension(Path::new("foo.xml")).is_err());
    }

    #[test]
    fn from_str_name() {
        assert_eq!(Format::from_str_name("yaml").unwrap(), Format::Yaml);
        assert_eq!(Format::from_str_name("json").unwrap(), Format::Json);
        assert!(Format::from_str_name("xml").is_err());
    }
}
