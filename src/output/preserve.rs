use serde_json::{Map, Value};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

use crate::error::QfError;

/// Apply the structural diff between `original_val` and `updated_val` onto `original_toml`
/// using `toml_edit::DocumentMut`, preserving comments, blank lines, key ordering, and
/// formatting on untouched keys.
pub fn apply_toml_diff(
    original_toml: &str,
    original_val: &Value,
    updated_val: &Value,
) -> Result<String, QfError> {
    let orig_map = original_val.as_object().ok_or_else(|| {
        QfError::Serialize("TOML diff requires original value to be an object".to_string())
    })?;
    let updated_map = updated_val
        .as_object()
        .ok_or_else(|| QfError::Serialize("TOML root must be a table/object".to_string()))?;

    let mut doc: DocumentMut = original_toml
        .parse()
        .map_err(|e: toml_edit::TomlError| QfError::Parse(e.to_string()))?;

    diff_and_apply_table(doc.as_table_mut(), orig_map, updated_map)?;
    Ok(doc.to_string())
}

fn diff_and_apply_table(
    table: &mut Table,
    orig: &Map<String, Value>,
    updated: &Map<String, Value>,
) -> Result<(), QfError> {
    // Remove keys that were deleted or set to null (TOML has no null).
    for key in orig.keys() {
        if !matches!(updated.get(key), Some(v) if !v.is_null()) {
            table.remove(key);
        }
    }

    // Apply additions and updates.
    for (key, new_val) in updated {
        if new_val.is_null() {
            continue;
        }
        match orig.get(key) {
            Some(old_val) if old_val == new_val => {
                // Untouched: preserve existing formatting and comments.
            }
            Some(old_val) => {
                apply_updated_table_entry(table, key, old_val, new_val)?;
            }
            None => {
                table.insert(key, json_to_toml_item(new_val)?);
            }
        }
    }

    Ok(())
}

fn apply_updated_table_entry(
    table: &mut Table,
    key: &str,
    old_val: &Value,
    new_val: &Value,
) -> Result<(), QfError> {
    match (old_val, new_val, table.get_mut(key)) {
        (Value::Object(old_map), Value::Object(new_map), Some(Item::Table(sub_table))) => {
            diff_and_apply_table(sub_table, old_map, new_map)
        }
        (
            Value::Object(old_map),
            Value::Object(new_map),
            Some(Item::Value(TomlValue::InlineTable(inline))),
        ) => diff_and_apply_inline_table(inline, old_map, new_map),
        (Value::Array(old_arr), Value::Array(new_arr), Some(Item::ArrayOfTables(aot)))
            if new_arr.iter().all(Value::is_object) =>
        {
            diff_and_apply_array_of_tables(aot, old_arr, new_arr)
        }
        (_, _, Some(Item::Value(existing_val))) if !new_val.is_object() => {
            let decor = existing_val.decor().clone();
            let mut replacement = json_to_toml_value(new_val)?;
            *replacement.decor_mut() = decor;
            *existing_val = replacement;
            Ok(())
        }
        (_, _, Some(existing_item)) => {
            *existing_item = json_to_toml_item(new_val)?;
            Ok(())
        }
        (_, _, None) => {
            table.insert(key, json_to_toml_item(new_val)?);
            Ok(())
        }
    }
}

fn diff_and_apply_inline_table(
    inline: &mut InlineTable,
    orig: &Map<String, Value>,
    updated: &Map<String, Value>,
) -> Result<(), QfError> {
    for key in orig.keys() {
        if !matches!(updated.get(key), Some(v) if !v.is_null()) {
            inline.remove(key);
        }
    }

    for (key, new_val) in updated {
        if new_val.is_null() {
            continue;
        }
        match orig.get(key) {
            Some(old_val) if old_val == new_val => {}
            Some(Value::Object(old_map)) if new_val.is_object() => {
                let new_map = new_val.as_object().expect("checked is_object");
                if let Some(TomlValue::InlineTable(sub_inline)) = inline.get_mut(key) {
                    diff_and_apply_inline_table(sub_inline, old_map, new_map)?;
                } else {
                    inline.insert(key, json_to_toml_value(new_val)?);
                }
            }
            Some(_) => {
                if let Some(existing_val) = inline.get_mut(key) {
                    let decor = existing_val.decor().clone();
                    let mut replacement = json_to_toml_value(new_val)?;
                    *replacement.decor_mut() = decor;
                    *existing_val = replacement;
                } else {
                    inline.insert(key, json_to_toml_value(new_val)?);
                }
            }
            None => {
                inline.insert(key, json_to_toml_value(new_val)?);
            }
        }
    }

    Ok(())
}

fn diff_and_apply_array_of_tables(
    aot: &mut ArrayOfTables,
    old_arr: &[Value],
    new_arr: &[Value],
) -> Result<(), QfError> {
    let common = old_arr.len().min(new_arr.len()).min(aot.len());
    for i in 0..common {
        if old_arr[i] != new_arr[i] {
            if let (Some(old_map), Some(new_map), Some(tbl)) = (
                old_arr[i].as_object(),
                new_arr[i].as_object(),
                aot.get_mut(i),
            ) {
                diff_and_apply_table(tbl, old_map, new_map)?;
            }
        }
    }

    while aot.len() > new_arr.len() {
        aot.remove(aot.len() - 1);
    }

    for item in new_arr.iter().skip(common) {
        let map = item.as_object().ok_or_else(|| {
            QfError::Serialize("array of tables elements must be objects".to_string())
        })?;
        aot.push(json_map_to_table(map)?);
    }

    Ok(())
}

fn json_to_toml_item(value: &Value) -> Result<Item, QfError> {
    match value {
        Value::Object(map) => Ok(Item::Table(json_map_to_table(map)?)),
        _ => Ok(Item::Value(json_to_toml_value(value)?)),
    }
}

fn json_map_to_table(map: &Map<String, Value>) -> Result<Table, QfError> {
    let mut table = Table::new();
    for (k, v) in map {
        if !v.is_null() {
            table.insert(k, json_to_toml_item(v)?);
        }
    }
    Ok(table)
}

fn json_to_toml_value(value: &Value) -> Result<TomlValue, QfError> {
    match value {
        Value::Null => Err(QfError::Serialize(
            "TOML does not support null values".to_string(),
        )),
        Value::Bool(b) => Ok(TomlValue::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(TomlValue::from(i))
            } else if let Some(f) = n.as_f64() {
                Ok(TomlValue::from(f))
            } else {
                Err(QfError::Serialize(format!("unsupported number: {n}")))
            }
        }
        Value::String(s) => {
            // Preserve RFC3339 datetimes (date + time) as native TOML datetimes on round-trip.
            if let Ok(dt) = s.parse::<toml_edit::Datetime>() {
                if dt.date.is_some() && dt.time.is_some() {
                    return Ok(TomlValue::from(dt));
                }
            }
            Ok(TomlValue::from(s.as_str()))
        }
        Value::Array(arr) => {
            let mut toml_arr = Array::new();
            for item in arr {
                toml_arr.push(json_to_toml_value(item)?);
            }
            Ok(TomlValue::Array(toml_arr))
        }
        Value::Object(map) => {
            let mut inline = InlineTable::new();
            for (k, v) in map {
                if !v.is_null() {
                    inline.insert(k, json_to_toml_value(v)?);
                }
            }
            Ok(TomlValue::InlineTable(inline))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use crate::query;

    #[test]
    fn preserves_comments_blank_lines_and_formatting_on_update() {
        let original_toml = "\
# Project configuration
[package]
name = \"qf\" # tool name
version = \"0.1.0\"

# Custom formatting on untouched key
authors = [ 'Alice', 'Bob' ]

[dependencies]
serde = \"1.0\"
";
        let orig_val = parser::toml::parse(original_toml).unwrap();
        let updated_val = query::query(&orig_val, r#".package.version = "0.2.0""#)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let result = apply_toml_diff(original_toml, &orig_val, &updated_val).unwrap();
        let expected = "\
# Project configuration
[package]
name = \"qf\" # tool name
version = \"0.2.0\"

# Custom formatting on untouched key
authors = [ 'Alice', 'Bob' ]

[dependencies]
serde = \"1.0\"
";
        assert_eq!(result, expected);
    }

    #[test]
    fn preserves_comments_on_add_and_delete() {
        let original_toml = "\
# Top-level comment
title = \"demo\"

[server]
# Keep this comment
host = \"127.0.0.1\"
port = 8080
legacy = true
";
        let orig_val = parser::toml::parse(original_toml).unwrap();
        let updated_val = query::query(
            &orig_val,
            r#"del(.server.legacy) | (.server.tls = true) | (.server.port = 9090)"#,
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

        let result = apply_toml_diff(original_toml, &orig_val, &updated_val).unwrap();
        assert!(result.contains("# Top-level comment"));
        assert!(result.contains("# Keep this comment"));
        assert!(result.contains("host = \"127.0.0.1\""));
        assert!(result.contains("port = 9090"));
        assert!(result.contains("tls = true"));
        assert!(!result.contains("legacy"));
    }

    #[test]
    fn preserves_inline_table_and_array_of_tables() {
        let original_toml = "\
# Point config
point = { x = 1, y = 2 }

# Products list
[[products]]
name = \"Hammer\" # heavy
sku = 738594937
";
        let orig_val = parser::toml::parse(original_toml).unwrap();
        let updated_val = query::query(&orig_val, r#"(.point.y = 20) | (.products[0].sku = 999)"#)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let result = apply_toml_diff(original_toml, &orig_val, &updated_val).unwrap();
        assert!(result.contains("# Point config"));
        assert!(result.contains("point = { x = 1, y = 20 }"));
        assert!(result.contains("# Products list"));
        assert!(result.contains("name = \"Hammer\" # heavy"));
        assert!(result.contains("sku = 999"));
    }
}
