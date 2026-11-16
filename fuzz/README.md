# Fuzzing `qf`

This directory contains `cargo-fuzz` (`libfuzzer-sys`) targets for `qf`.
It is configured as an isolated `[workspace]` so it does not interfere with the root `Cargo.toml` workspace.

## Targets

- `fuzz_lexer_parser`: Fuzzes `qf::query::lexer::Lexer`, `qf::query::jq_parser::Parser`, and `qf::query::CompiledQuery::compile` with arbitrary UTF-8 query strings.
- `fuzz_query`: Fuzzes `qf::query::query` with arbitrary query strings and optional NUL-delimited JSON payloads.
- `fuzz_formats`: Fuzzes `qf::parser::parse` across all 6 supported formats (`Json`, `Yaml`, `Xml`, `Toml`, `Csv`, `Tsv`) as well as `Format::detect_from_content`.

## Running

Requires `cargo-fuzz` (`cargo install cargo-fuzz`) and a nightly Rust toolchain:

```bash
cargo fuzz run fuzz_lexer_parser
cargo fuzz run fuzz_query
cargo fuzz run fuzz_formats
```

In offline environments where `libfuzzer-sys` is not cached, `tests/jq_conformance.rs` also includes a deterministic pseudo-random & malformed-input panic-freedom smoke test covering the lexer, parser, query evaluator, and all 6 format parsers.
