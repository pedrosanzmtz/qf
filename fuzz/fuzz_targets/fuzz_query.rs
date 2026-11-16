#![no_main]

use libfuzzer_sys::fuzz_target;
use qf::query::query;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    if let Ok(s) = std::str::from_utf8(data) {
        if let Some((q, json_str)) = s.split_once('\0') {
            if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                let _ = query(&val, q);
            }
        } else {
            let fixture = serde_json::json!({
                "a": [1, 2, 3],
                "b": {"c": "hello", "d": true, "e": null}
            });
            let _ = query(&fixture, s);
        }
    }
});
