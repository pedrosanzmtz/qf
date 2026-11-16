#![no_main]

use libfuzzer_sys::fuzz_target;
use qf::query::{jq_parser::Parser, lexer::Lexer, CompiledQuery};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let mut lex = Lexer::new(s);
        if lex.tokenize().is_ok() {
            let mut parser = Parser::new(lex.tokens);
            let _ = parser.parse();
        }
        let _ = CompiledQuery::compile(s);
    }
});
