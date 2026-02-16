use thiserror::Error;

#[derive(Error, Debug)]
pub enum QfError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("cannot detect format: no file extension")]
    NoExtension,

    #[error("unknown file extension: .{0}")]
    UnknownExtension(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("invalid query path: {0}")]
    InvalidQuery(String),

    #[error("path not found: {0}")]
    PathNotFound(String),

    #[error("index out of bounds: {index} (length {length})")]
    IndexOutOfBounds { index: usize, length: usize },

    #[error("expected array but found {0}")]
    ExpectedArray(String),

    #[error("expected object but found {0}")]
    ExpectedObject(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
