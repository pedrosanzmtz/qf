pub mod ast;
pub mod builtins;
pub mod env;
pub mod eval;
pub mod jq_parser;
pub mod lexer;
pub mod path;

use serde_json::Value;

use crate::error::QfError;

/// Execute a query string against a JSON Value, returning multiple results.
///
/// Uses the JQ-compatible engine for complex queries, falls back to
/// the simple path engine for basic dot-notation paths.
pub fn query(input: &Value, query_str: &str) -> Result<Vec<Value>, QfError> {
    // Use the JQ engine for all queries
    let mut lex = lexer::Lexer::new(query_str);
    lex.tokenize()?;
    let mut parser = jq_parser::Parser::new(lex.tokens);
    let expr = parser.parse()?;
    let env = env::Env::new();
    eval::eval(&expr, input, &env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_shorthand() {
        let val = json!({"a": {"b": 1}});
        let results = query(&val, ".a.b").unwrap();
        assert_eq!(results, vec![json!(1)]);
    }

    #[test]
    fn query_identity() {
        let val = json!({"x": 1});
        let results = query(&val, ".").unwrap();
        assert_eq!(results, vec![val]);
    }

    #[test]
    fn query_pipe() {
        let val = json!({"a": {"b": 2}});
        let results = query(&val, ".a | .b").unwrap();
        assert_eq!(results, vec![json!(2)]);
    }

    #[test]
    fn query_iterate() {
        let val = json!({"items": [1, 2, 3]});
        let results = query(&val, ".items[]").unwrap();
        assert_eq!(results, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn query_iterate_with_field() {
        let val = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let results = query(&val, "[.items[] | .name]").unwrap();
        assert_eq!(results, vec![json!(["a", "b"])]);
    }

    #[test]
    fn query_select() {
        let val = json!([1, 2, 3, 4, 5]);
        let results = query(&val, "[.[] | select(. > 3)]").unwrap();
        assert_eq!(results, vec![json!([4, 5])]);
    }

    #[test]
    fn query_addition() {
        let val = json!({"a": 1, "b": 2});
        let results = query(&val, ".a + .b").unwrap();
        assert_eq!(results, vec![json!(3)]);
    }

    #[test]
    fn query_object_construct() {
        let val = json!({"x": 1, "y": 2});
        let results = query(&val, "{a: .x, b: .y}").unwrap();
        assert_eq!(results, vec![json!({"a": 1, "b": 2})]);
    }

    #[test]
    fn query_map() {
        let val = json!([1, 2, 3]);
        let results = query(&val, "map(. * 2)").unwrap();
        assert_eq!(results, vec![json!([2, 4, 6])]);
    }

    #[test]
    fn query_reduce() {
        let val = json!([1, 2, 3, 4, 5]);
        let results = query(&val, "reduce .[] as $x (0; . + $x)").unwrap();
        assert_eq!(results, vec![json!(15)]);
    }

    #[test]
    fn query_sort() {
        let val = json!([3, 1, 2]);
        let results = query(&val, "sort").unwrap();
        assert_eq!(results, vec![json!([1, 2, 3])]);
    }

    #[test]
    fn query_keys() {
        let val = json!({"b": 1, "a": 2});
        let results = query(&val, "keys").unwrap();
        assert_eq!(results, vec![json!(["a", "b"])]);
    }

    #[test]
    fn query_length() {
        let val = json!([1, 2, 3]);
        let results = query(&val, "length").unwrap();
        assert_eq!(results, vec![json!(3)]);
    }

    #[test]
    fn query_alternative() {
        let val = json!({"a": null});
        let results = query(&val, ".a // 42").unwrap();
        assert_eq!(results, vec![json!(42)]);
    }

    #[test]
    fn query_missing_key_returns_null() {
        let val = json!({"a": 1});
        let results = query(&val, ".missing").unwrap();
        assert_eq!(results, vec![json!(null)]);
    }

    #[test]
    fn query_update_assign() {
        let val = json!({"a": 1});
        let results = query(&val, ".a |= . + 10").unwrap();
        assert_eq!(results, vec![json!({"a": 11})]);
    }

    #[test]
    fn query_if_then_else() {
        let val = json!(5);
        let results = query(&val, "if . > 3 then \"big\" else \"small\" end").unwrap();
        assert_eq!(results, vec![json!("big")]);
    }

    #[test]
    fn query_def() {
        let results = query(&json!(null), "def double: . * 2; 5 | double").unwrap();
        assert_eq!(results, vec![json!(10)]);
    }

    #[test]
    fn query_comma() {
        let val = json!({"a": 1, "b": 2});
        let results = query(&val, ".a, .b").unwrap();
        assert_eq!(results, vec![json!(1), json!(2)]);
    }

    #[test]
    fn query_negative_index() {
        let val = json!([1, 2, 3]);
        let results = query(&val, ".[-1]").unwrap();
        assert_eq!(results, vec![json!(3)]);
    }

    #[test]
    fn query_object_merge() {
        let results = query(&json!(null), r#"{"a":1} * {"b":2}"#).unwrap();
        assert_eq!(results, vec![json!({"a": 1, "b": 2})]);
    }

    #[test]
    fn query_string_interpolation() {
        let val = json!({"name": "world"});
        let results = query(&val, r#""Hello \(.name)!""#).unwrap();
        assert_eq!(results, vec![json!("Hello world!")]);
    }

    #[test]
    fn query_try_catch() {
        let results = query(&json!("hello"), r#"try .foo catch "err""#).unwrap();
        assert_eq!(results, vec![json!("err")]);
    }

    #[test]
    fn query_group_by() {
        let val = json!([{"a":1},{"a":2},{"a":1}]);
        let results = query(&val, "group_by(.a)").unwrap();
        assert_eq!(
            results,
            vec![json!([[{"a":1},{"a":1}],[{"a":2}]])]
        );
    }

    #[test]
    fn query_unique() {
        let val = json!([1, 2, 1, 3, 2]);
        let results = query(&val, "unique").unwrap();
        assert_eq!(results, vec![json!([1, 2, 3])]);
    }

    #[test]
    fn query_flatten() {
        let val = json!([[1, 2], [3, [4, 5]]]);
        let results = query(&val, "flatten").unwrap();
        assert_eq!(results, vec![json!([1, 2, 3, 4, 5])]);
    }

    #[test]
    fn query_to_entries() {
        let val = json!({"a": 1, "b": 2});
        let results = query(&val, "to_entries").unwrap();
        let arr = results[0].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn query_from_entries() {
        let val = json!([{"key":"a","value":1},{"key":"b","value":2}]);
        let results = query(&val, "from_entries").unwrap();
        assert_eq!(results, vec![json!({"a": 1, "b": 2})]);
    }

    #[test]
    fn query_format_base64() {
        let results = query(&json!("hello"), "@base64").unwrap();
        assert_eq!(results, vec![json!("aGVsbG8=")]);
    }

    #[test]
    fn query_contains() {
        let results = query(&json!("foobar"), r#"contains("foo")"#).unwrap();
        assert_eq!(results, vec![json!(true)]);
    }

    #[test]
    fn query_split_join() {
        let results = query(&json!("a,b,c"), r#"split(",") | join("-")"#).unwrap();
        assert_eq!(results, vec![json!("a-b-c")]);
    }

    #[test]
    fn query_regex_test() {
        let results = query(&json!("hello123"), r#"test("\\d+")"#).unwrap();
        assert_eq!(results, vec![json!(true)]);
    }

    #[test]
    fn query_floor_ceil() {
        let results = query(&json!(3.7), "floor").unwrap();
        assert_eq!(results, vec![json!(3)]);
        let results = query(&json!(3.2), "ceil").unwrap();
        assert_eq!(results, vec![json!(4)]);
    }

    #[test]
    fn query_range() {
        let results = query(&json!(null), "range(5)").unwrap();
        assert_eq!(
            results,
            vec![json!(0), json!(1), json!(2), json!(3), json!(4)]
        );
    }

    #[test]
    fn query_tojson_fromjson() {
        let results = query(&json!({"a": 1}), "tojson").unwrap();
        assert_eq!(results, vec![json!(r#"{"a":1}"#)]);
    }

    #[test]
    fn query_ascii_case() {
        assert_eq!(
            query(&json!("Hello"), "ascii_downcase").unwrap(),
            vec![json!("hello")]
        );
        assert_eq!(
            query(&json!("Hello"), "ascii_upcase").unwrap(),
            vec![json!("HELLO")]
        );
    }

    #[test]
    fn query_logical_ops() {
        assert_eq!(
            query(&json!(null), "true and false").unwrap(),
            vec![json!(false)]
        );
        assert_eq!(
            query(&json!(null), "true or false").unwrap(),
            vec![json!(true)]
        );
        assert_eq!(
            query(&json!(null), "true | not").unwrap(),
            vec![json!(false)]
        );
    }
}
