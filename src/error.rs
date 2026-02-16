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

    #[error("syntax error at position {position}: {message}")]
    SyntaxError { position: usize, message: String },

    #[error("type error: {0}")]
    TypeError(String),

    #[error("undefined variable: ${0}")]
    UndefinedVariable(String),

    #[error("undefined function: {0}/{1}")]
    UndefinedFunction(String, usize),

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("{0}")]
    UserError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
