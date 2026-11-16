use serde_json::Value;

// ANSI color codes
const RESET: &str = "\x1b[0m";
const DEFAULT_NULL: &str = "\x1b[0;31m";
const DEFAULT_BOOL: &str = "\x1b[0;33m";
const DEFAULT_NUMBER: &str = "\x1b[0;36m";
const DEFAULT_STRING: &str = "\x1b[0;32m";
const DEFAULT_CONTAINER: &str = "\x1b[1;37m";
const DEFAULT_KEY: &str = "\x1b[1;34m";

/// ANSI SGR color palette following jq's 8-slot `JQ_COLORS` format:
/// `null:false:true:numbers:strings:arrays:objects:keys`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ColorPalette {
    null: String,
    bool_false: String,
    bool_true: String,
    numbers: String,
    strings: String,
    arrays: String,
    objects: String,
    keys: String,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self {
            null: DEFAULT_NULL.to_string(),
            bool_false: DEFAULT_BOOL.to_string(),
            bool_true: DEFAULT_BOOL.to_string(),
            numbers: DEFAULT_NUMBER.to_string(),
            strings: DEFAULT_STRING.to_string(),
            arrays: DEFAULT_CONTAINER.to_string(),
            objects: DEFAULT_CONTAINER.to_string(),
            keys: DEFAULT_KEY.to_string(),
        }
    }
}

impl ColorPalette {
    fn from_env() -> Self {
        let spec = std::env::var("QF_COLORS")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("JQ_COLORS").ok().filter(|s| !s.is_empty()));
        spec.and_then(|s| Self::parse(&s)).unwrap_or_default()
    }

    fn parse(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split(':').collect();
        if parts.len() != 8 {
            return None;
        }
        if !parts.iter().all(|p| is_valid_sgr(p)) {
            return None;
        }
        Some(Self {
            null: format!("\x1b[{}m", parts[0]),
            bool_false: format!("\x1b[{}m", parts[1]),
            bool_true: format!("\x1b[{}m", parts[2]),
            numbers: format!("\x1b[{}m", parts[3]),
            strings: format!("\x1b[{}m", parts[4]),
            arrays: format!("\x1b[{}m", parts[5]),
            objects: format!("\x1b[{}m", parts[6]),
            keys: format!("\x1b[{}m", parts[7]),
        })
    }
}

fn is_valid_sgr(part: &str) -> bool {
    !part.is_empty()
        && part
            .split(';')
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Colorize a JSON value into a pretty-printed string with ANSI color codes.
pub fn colorize_json(value: &Value) -> String {
    colorize_json_with_options(value, false)
}

/// Colorize a JSON value into a pretty or compact string with ANSI color codes.
pub fn colorize_json_with_options(value: &Value, compact: bool) -> String {
    let palette = ColorPalette::from_env();
    colorize_json_with_palette(value, compact, &palette)
}

fn colorize_json_with_palette(value: &Value, compact: bool, palette: &ColorPalette) -> String {
    let mut buf = String::new();
    write_value(value, &mut buf, 0, compact, palette);
    buf
}

fn write_value(
    value: &Value,
    buf: &mut String,
    indent: usize,
    compact: bool,
    palette: &ColorPalette,
) {
    match value {
        Value::Null => {
            buf.push_str(&palette.null);
            buf.push_str("null");
            buf.push_str(RESET);
        }
        Value::Bool(b) => {
            let color = if *b {
                &palette.bool_true
            } else {
                &palette.bool_false
            };
            buf.push_str(color);
            buf.push_str(if *b { "true" } else { "false" });
            buf.push_str(RESET);
        }
        Value::Number(n) => {
            buf.push_str(&palette.numbers);
            buf.push_str(&n.to_string());
            buf.push_str(RESET);
        }
        Value::String(s) => {
            buf.push_str(&palette.strings);
            buf.push('"');
            buf.push_str(&escape_json_string(s));
            buf.push('"');
            buf.push_str(RESET);
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                buf.push_str(&palette.arrays);
                buf.push_str("[]");
                buf.push_str(RESET);
                return;
            }
            buf.push_str(&palette.arrays);
            buf.push('[');
            buf.push_str(RESET);
            if !compact {
                buf.push('\n');
            }
            for (i, item) in arr.iter().enumerate() {
                if !compact {
                    write_indent(buf, indent + 1);
                }
                write_value(item, buf, indent + 1, compact, palette);
                if i < arr.len() - 1 {
                    buf.push(',');
                }
                if !compact {
                    buf.push('\n');
                }
            }
            if !compact {
                write_indent(buf, indent);
            }
            buf.push_str(&palette.arrays);
            buf.push(']');
            buf.push_str(RESET);
        }
        Value::Object(map) => {
            if map.is_empty() {
                buf.push_str(&palette.objects);
                buf.push_str("{}");
                buf.push_str(RESET);
                return;
            }
            buf.push_str(&palette.objects);
            buf.push('{');
            buf.push_str(RESET);
            if !compact {
                buf.push('\n');
            }
            let len = map.len();
            for (i, (key, val)) in map.iter().enumerate() {
                if !compact {
                    write_indent(buf, indent + 1);
                }
                buf.push_str(&palette.keys);
                buf.push('"');
                buf.push_str(&escape_json_string(key));
                buf.push('"');
                buf.push_str(RESET);
                buf.push_str(if compact { ":" } else { ": " });
                write_value(val, buf, indent + 1, compact, palette);
                if i < len - 1 {
                    buf.push(',');
                }
                if !compact {
                    buf.push('\n');
                }
            }
            if !compact {
                write_indent(buf, indent);
            }
            buf.push_str(&palette.objects);
            buf.push('}');
            buf.push_str(RESET);
        }
    }
}

fn write_indent(buf: &mut String, level: usize) {
    for _ in 0..level {
        buf.push_str("  ");
    }
}

fn escape_json_string(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c < '\x20' => {
                escaped.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => escaped.push(c),
        }
    }
    escaped
}

/// Colorize YAML output by post-processing the serde_yaml string.
pub fn colorize_yaml(yaml: &str) -> String {
    let palette = ColorPalette::from_env();
    let mut buf = String::with_capacity(yaml.len() * 2);
    for line in yaml.lines() {
        colorize_yaml_line(line, &mut buf, &palette);
        buf.push('\n');
    }
    buf
}

fn colorize_yaml_line(line: &str, buf: &mut String, palette: &ColorPalette) {
    let trimmed = line.trim_start();

    // Comment lines
    if trimmed.starts_with('#') {
        buf.push_str(&palette.null);
        buf.push_str(line);
        buf.push_str(RESET);
        return;
    }

    // Document separator
    if trimmed == "---" || trimmed == "..." {
        buf.push_str(&palette.arrays);
        buf.push_str(line);
        buf.push_str(RESET);
        return;
    }

    // List item prefix
    if let Some(rest) = trimmed.strip_prefix("- ") {
        let indent = &line[..line.len() - trimmed.len()];
        buf.push_str(indent);
        buf.push_str(&palette.arrays);
        buf.push_str("- ");
        buf.push_str(RESET);
        colorize_yaml_value_or_key(rest, buf, palette);
        return;
    }

    // Key: value
    if let Some(colon_pos) = find_yaml_colon(trimmed) {
        let indent = &line[..line.len() - trimmed.len()];
        let key = &trimmed[..colon_pos];
        let after_colon = &trimmed[colon_pos + 1..];
        buf.push_str(indent);
        buf.push_str(&palette.keys);
        buf.push_str(key);
        buf.push_str(RESET);
        buf.push(':');
        if !after_colon.is_empty() {
            buf.push(' ');
            colorize_yaml_scalar(after_colon.trim_start(), buf, palette);
        }
        return;
    }

    // Plain scalar or list continuation
    colorize_yaml_scalar(trimmed, buf, palette);
}

fn find_yaml_colon(s: &str) -> Option<usize> {
    // Find ': ' or ':' at end — but not inside quotes
    let bytes = s.as_bytes();
    let mut in_quote = false;
    let mut quote_char = 0u8;
    for (i, &b) in bytes.iter().enumerate() {
        if in_quote {
            if b == quote_char {
                in_quote = false;
            }
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_quote = true;
            quote_char = b;
            continue;
        }
        if b == b':' && (i + 1 >= bytes.len() || bytes[i + 1] == b' ') {
            return Some(i);
        }
    }
    None
}

fn colorize_yaml_value_or_key(s: &str, buf: &mut String, palette: &ColorPalette) {
    if let Some(colon_pos) = find_yaml_colon(s) {
        let key = &s[..colon_pos];
        let after_colon = &s[colon_pos + 1..];
        buf.push_str(&palette.keys);
        buf.push_str(key);
        buf.push_str(RESET);
        buf.push(':');
        if !after_colon.is_empty() {
            buf.push(' ');
            colorize_yaml_scalar(after_colon.trim_start(), buf, palette);
        }
    } else {
        colorize_yaml_scalar(s, buf, palette);
    }
}

fn colorize_yaml_scalar(s: &str, buf: &mut String, palette: &ColorPalette) {
    match s {
        "null" | "~" => {
            buf.push_str(&palette.null);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        "true" => {
            buf.push_str(&palette.bool_true);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        "false" => {
            buf.push_str(&palette.bool_false);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        _ if s.starts_with('\'') || s.starts_with('"') => {
            buf.push_str(&palette.strings);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        _ if looks_numeric(s) => {
            buf.push_str(&palette.numbers);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        _ => {
            buf.push_str(&palette.strings);
            buf.push_str(s);
            buf.push_str(RESET);
        }
    }
}

fn looks_numeric(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    if s.is_empty() {
        return false;
    }
    let mut has_dot = false;
    for c in s.chars() {
        if c == '.' {
            if has_dot {
                return false;
            }
            has_dot = true;
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colorize_simple_object() {
        let val = json!({"name": "test", "count": 42});
        let out = colorize_json(&val);
        assert!(out.contains("\x1b[1;34m\"name\"\x1b[0m"));
        assert!(out.contains("\x1b[0;32m\"test\"\x1b[0m"));
        assert!(out.contains("\x1b[0;36m42\x1b[0m"));
    }

    #[test]
    fn colorize_null_and_bool() {
        let val = json!({"flag": true, "empty": null});
        let out = colorize_json(&val);
        assert!(out.contains("\x1b[0;33mtrue\x1b[0m"));
        assert!(out.contains("\x1b[0;31mnull\x1b[0m"));
    }

    #[test]
    fn colorize_empty_containers() {
        let val = json!({"arr": [], "obj": {}});
        let out = colorize_json(&val);
        assert!(out.contains("[]"));
        assert!(out.contains("{}"));
    }

    #[test]
    fn colorize_yaml_basic() {
        let yaml = "name: test\ncount: 42\nflag: true\nempty: null\n";
        let out = colorize_yaml(yaml);
        assert!(out.contains("\x1b[1;34mname\x1b[0m:"));
        assert!(out.contains("\x1b[0;36m42\x1b[0m"));
        assert!(out.contains("\x1b[0;33mtrue\x1b[0m"));
        assert!(out.contains("\x1b[0;31mnull\x1b[0m"));
    }

    #[test]
    fn escape_special_chars() {
        let s = "hello \"world\"\nnewline";
        let escaped = escape_json_string(s);
        assert_eq!(escaped, "hello \\\"world\\\"\\nnewline");
    }

    #[test]
    fn colorize_compact_object_and_array() {
        let val = json!({"a": 1, "b": [true, "hi"]});
        let out = colorize_json_with_options(&val, true);
        assert!(!out.contains('\n'));
        assert!(!out.contains(": "));
        assert!(out.contains("\x1b[1;34m\"a\"\x1b[0m:\x1b[0;36m1\x1b[0m"));
        assert!(out.contains("\x1b[0;33mtrue\x1b[0m,\x1b[0;32m\"hi\"\x1b[0m"));
    }

    #[test]
    fn colorize_compact_empty_containers() {
        let val = json!({"arr": [], "obj": {}});
        let out = colorize_json_with_options(&val, true);
        assert!(!out.contains('\n'));
        assert!(out.contains("\x1b[1;37m[]\x1b[0m"));
        assert!(out.contains("\x1b[1;37m{}\x1b[0m"));
    }

    #[test]
    fn palette_parse_valid_jq_colors() {
        let palette = ColorPalette::parse("90:31:32:33:34:35:36:37").unwrap();
        let val = json!({"k": [null, false, true, 7, "s"]});
        let out = colorize_json_with_palette(&val, true, &palette);
        assert!(out.contains("\x1b[37m\"k\"\x1b[0m:"));
        assert!(out.contains("\x1b[36m{\x1b[0m"));
        assert!(out.contains("\x1b[35m[\x1b[0m"));
        assert!(out.contains("\x1b[90mnull\x1b[0m"));
        assert!(out.contains("\x1b[31mfalse\x1b[0m"));
        assert!(out.contains("\x1b[32mtrue\x1b[0m"));
        assert!(out.contains("\x1b[33m7\x1b[0m"));
        assert!(out.contains("\x1b[34m\"s\"\x1b[0m"));
    }

    #[test]
    fn palette_parse_invalid_falls_back() {
        assert!(ColorPalette::parse("1;34:0;32").is_none());
        assert!(ColorPalette::parse("1:2:3:4:5:6:7:8:9").is_none());
        assert!(ColorPalette::parse("1:2:3::5:6:7:8").is_none());
        assert!(ColorPalette::parse("1:2:3:bad:5:6:7:8").is_none());
        assert!(ColorPalette::parse("1:2:3:1;;2:5:6:7:8").is_none());
    }

    #[test]
    fn colorize_yaml_list_items() {
        let yaml = "- name: alice\n- 42\n- -10.5\n";
        let out = colorize_yaml(yaml);
        assert!(out.contains("\x1b[1;37m- \x1b[0m\x1b[1;34mname\x1b[0m: \x1b[0;32malice\x1b[0m"));
        assert!(out.contains("\x1b[1;37m- \x1b[0m\x1b[0;36m42\x1b[0m"));
        assert!(out.contains("\x1b[1;37m- \x1b[0m\x1b[0;36m-10.5\x1b[0m"));
    }
}
