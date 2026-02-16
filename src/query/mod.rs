pub mod path;

use serde_json::Value;

use crate::error::QfError;
use path::QueryPath;

/// Execute a query string against a JSON Value.
pub fn query(input: &Value, query_str: &str) -> Result<Value, QfError> {
    let path = QueryPath::parse(query_str)?;
    path.evaluate(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_shorthand() {
        let val = json!({"a": {"b": 1}});
        assert_eq!(query(&val, ".a.b").unwrap(), json!(1));
    }

    #[test]
    fn query_identity() {
        let val = json!({"x": 1});
        assert_eq!(query(&val, ".").unwrap(), val);
    }
}
