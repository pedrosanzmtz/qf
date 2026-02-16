use serde_json::Value;

// ANSI color codes
const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;34m";
const GREEN: &str = "\x1b[0;32m";
const CYAN: &str = "\x1b[0;36m";
const YELLOW: &str = "\x1b[0;33m";
const RED: &str = "\x1b[0;31m";
const BOLD_WHITE: &str = "\x1b[1;37m";

/// Colorize a JSON value into a pretty-printed string with ANSI color codes.
pub fn colorize_json(value: &Value) -> String {
    let mut buf = String::new();
    write_value(value, &mut buf, 0);
    buf
}

fn write_value(value: &Value, buf: &mut String, indent: usize) {
    match value {
        Value::Null => {
            buf.push_str(RED);
            buf.push_str("null");
            buf.push_str(RESET);
        }
        Value::Bool(b) => {
            buf.push_str(YELLOW);
            buf.push_str(if *b { "true" } else { "false" });
            buf.push_str(RESET);
        }
        Value::Number(n) => {
            buf.push_str(CYAN);
            buf.push_str(&n.to_string());
            buf.push_str(RESET);
        }
        Value::String(s) => {
            buf.push_str(GREEN);
            buf.push('"');
            buf.push_str(&escape_json_string(s));
            buf.push('"');
            buf.push_str(RESET);
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                buf.push_str(BOLD_WHITE);
                buf.push_str("[]");
                buf.push_str(RESET);
                return;
            }
            buf.push_str(BOLD_WHITE);
            buf.push('[');
            buf.push_str(RESET);
            buf.push('\n');
            for (i, item) in arr.iter().enumerate() {
                write_indent(buf, indent + 1);
                write_value(item, buf, indent + 1);
                if i < arr.len() - 1 {
                    buf.push(',');
                }
                buf.push('\n');
            }
            write_indent(buf, indent);
            buf.push_str(BOLD_WHITE);
            buf.push(']');
            buf.push_str(RESET);
        }
        Value::Object(map) => {
            if map.is_empty() {
                buf.push_str(BOLD_WHITE);
                buf.push_str("{}");
                buf.push_str(RESET);
                return;
            }
            buf.push_str(BOLD_WHITE);
            buf.push('{');
            buf.push_str(RESET);
            buf.push('\n');
            let len = map.len();
            for (i, (key, val)) in map.iter().enumerate() {
                write_indent(buf, indent + 1);
                buf.push_str(BOLD_BLUE);
                buf.push('"');
                buf.push_str(&escape_json_string(key));
                buf.push('"');
                buf.push_str(RESET);
                buf.push_str(": ");
                write_value(val, buf, indent + 1);
                if i < len - 1 {
                    buf.push(',');
                }
                buf.push('\n');
            }
            write_indent(buf, indent);
            buf.push_str(BOLD_WHITE);
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
    let mut buf = String::with_capacity(yaml.len() * 2);
    for line in yaml.lines() {
        colorize_yaml_line(line, &mut buf);
        buf.push('\n');
    }
    buf
}

fn colorize_yaml_line(line: &str, buf: &mut String) {
    let trimmed = line.trim_start();

    // Comment lines
    if trimmed.starts_with('#') {
        buf.push_str(RED);
        buf.push_str(line);
        buf.push_str(RESET);
        return;
    }

    // Document separator
    if trimmed == "---" || trimmed == "..." {
        buf.push_str(BOLD_WHITE);
        buf.push_str(line);
        buf.push_str(RESET);
        return;
    }

    // List item prefix
    if trimmed.starts_with("- ") {
        let indent = &line[..line.len() - trimmed.len()];
        buf.push_str(indent);
        buf.push_str(BOLD_WHITE);
        buf.push_str("- ");
        buf.push_str(RESET);
        let rest = &trimmed[2..];
        colorize_yaml_value_or_key(rest, buf);
        return;
    }

    // Key: value
    if let Some(colon_pos) = find_yaml_colon(trimmed) {
        let indent = &line[..line.len() - trimmed.len()];
        let key = &trimmed[..colon_pos];
        let after_colon = &trimmed[colon_pos + 1..];
        buf.push_str(indent);
        buf.push_str(BOLD_BLUE);
        buf.push_str(key);
        buf.push_str(RESET);
        buf.push(':');
        if !after_colon.is_empty() {
            buf.push(' ');
            colorize_yaml_scalar(after_colon.trim_start(), buf);
        }
        return;
    }

    // Plain scalar or list continuation
    colorize_yaml_scalar(trimmed, buf);
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

fn colorize_yaml_value_or_key(s: &str, buf: &mut String) {
    if let Some(colon_pos) = find_yaml_colon(s) {
        let key = &s[..colon_pos];
        let after_colon = &s[colon_pos + 1..];
        buf.push_str(BOLD_BLUE);
        buf.push_str(key);
        buf.push_str(RESET);
        buf.push(':');
        if !after_colon.is_empty() {
            buf.push(' ');
            colorize_yaml_scalar(after_colon.trim_start(), buf);
        }
    } else {
        colorize_yaml_scalar(s, buf);
    }
}

fn colorize_yaml_scalar(s: &str, buf: &mut String) {
    match s {
        "null" | "~" => {
            buf.push_str(RED);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        "true" | "false" => {
            buf.push_str(YELLOW);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        _ if s.starts_with('\'') || s.starts_with('"') => {
            buf.push_str(GREEN);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        _ if looks_numeric(s) => {
            buf.push_str(CYAN);
            buf.push_str(s);
            buf.push_str(RESET);
        }
        _ => {
            buf.push_str(GREEN);
            buf.push_str(s);
            buf.push_str(RESET);
        }
    }
}

fn looks_numeric(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = if s.starts_with('-') || s.starts_with('+') {
        &s[1..]
    } else {
        s
    };
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
}
