#![no_main]

use libfuzzer_sys::fuzz_target;
use qf::format::Format;
use qf::parser;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        for fmt in [
            Format::Json,
            Format::Yaml,
            Format::Xml,
            Format::Toml,
            Format::Csv,
            Format::Tsv,
        ] {
            let _ = parser::parse(s, fmt);
        }
        let _ = Format::detect_from_content(s);
    }
});
