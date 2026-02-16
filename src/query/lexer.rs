use crate::error::QfError;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    String(String),
    True,
    False,
    Null,

    // Identifiers and variables
    Ident(String),
    Variable(String), // $name
    Format(String),   // @base64, @csv, etc.

    // Operators
    Dot,        // .
    Pipe,       // |
    Comma,      // ,
    Colon,      // :
    Semicolon,  // ;
    Question,   // ?
    DotDot,     // .. (recursive descent)

    // Brackets
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    LBrace,     // {
    RBrace,     // }

    // Arithmetic
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %

    // Comparison
    Eq,         // ==
    Ne,         // !=
    Lt,         // <
    Le,         // <=
    Gt,         // >
    Ge,         // >=

    // Assignment
    Assign,     // =
    UpdateAssign, // |=
    PlusAssign, // +=
    MinusAssign, // -=
    StarAssign, // *=
    SlashAssign, // /=
    PercentAssign, // %=
    AltAssign,  // //=

    // Logic / keywords
    And,
    Or,
    Not,
    If,
    Then,
    Elif,
    Else,
    End,
    As,
    Def,
    Reduce,
    Foreach,
    Try,
    Catch,
    Import,
    Include,
    Label,
    Break,

    // Alternative
    Alternative, // //

    Eof,
}

impl Token {
    pub fn is_keyword(s: &str) -> Option<Token> {
        match s {
            "and" => Some(Token::And),
            "or" => Some(Token::Or),
            "not" => Some(Token::Not),
            "if" => Some(Token::If),
            "then" => Some(Token::Then),
            "elif" => Some(Token::Elif),
            "else" => Some(Token::Else),
            "end" => Some(Token::End),
            "as" => Some(Token::As),
            "def" => Some(Token::Def),
            "reduce" => Some(Token::Reduce),
            "foreach" => Some(Token::Foreach),
            "try" => Some(Token::Try),
            "catch" => Some(Token::Catch),
            "import" => Some(Token::Import),
            "include" => Some(Token::Include),
            "label" => Some(Token::Label),
            "true" => Some(Token::True),
            "false" => Some(Token::False),
            "null" => Some(Token::Null),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    pub tokens: Vec<Token>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Lexer {
            input: input.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
        }
    }

    pub fn tokenize(&mut self) -> Result<&[Token], QfError> {
        while self.pos < self.input.len() {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                break;
            }

            let ch = self.input[self.pos];
            match ch {
                '#' => {
                    // Comment — skip to end of line
                    while self.pos < self.input.len() && self.input[self.pos] != '\n' {
                        self.pos += 1;
                    }
                }
                '.' => {
                    if self.peek_next() == Some('.') {
                        self.pos += 2;
                        self.tokens.push(Token::DotDot);
                    } else if self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
                        // .123 — number starting with dot
                        self.read_number()?;
                    } else {
                        self.pos += 1;
                        self.tokens.push(Token::Dot);
                    }
                }
                '|' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::UpdateAssign);
                    } else {
                        self.tokens.push(Token::Pipe);
                    }
                }
                ',' => { self.pos += 1; self.tokens.push(Token::Comma); }
                ':' => { self.pos += 1; self.tokens.push(Token::Colon); }
                ';' => { self.pos += 1; self.tokens.push(Token::Semicolon); }
                '?' => {
                    self.pos += 1;
                    // ?// is the "try alternative" operator; we just emit ? and let parser handle
                    self.tokens.push(Token::Question);
                }
                '(' => { self.pos += 1; self.tokens.push(Token::LParen); }
                ')' => { self.pos += 1; self.tokens.push(Token::RParen); }
                '[' => { self.pos += 1; self.tokens.push(Token::LBracket); }
                ']' => { self.pos += 1; self.tokens.push(Token::RBracket); }
                '{' => { self.pos += 1; self.tokens.push(Token::LBrace); }
                '}' => { self.pos += 1; self.tokens.push(Token::RBrace); }
                '+' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::PlusAssign);
                    } else {
                        self.tokens.push(Token::Plus);
                    }
                }
                '-' => {
                    // Could be unary minus or subtraction
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::MinusAssign);
                    } else {
                        self.tokens.push(Token::Minus);
                    }
                }
                '*' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::StarAssign);
                    } else {
                        self.tokens.push(Token::Star);
                    }
                }
                '/' => {
                    self.pos += 1;
                    if self.peek_current() == Some('/') {
                        self.pos += 1;
                        if self.peek_current() == Some('=') {
                            self.pos += 1;
                            self.tokens.push(Token::AltAssign);
                        } else {
                            self.tokens.push(Token::Alternative);
                        }
                    } else if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::SlashAssign);
                    } else {
                        self.tokens.push(Token::Slash);
                    }
                }
                '%' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::PercentAssign);
                    } else {
                        self.tokens.push(Token::Percent);
                    }
                }
                '=' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::Eq);
                    } else {
                        self.tokens.push(Token::Assign);
                    }
                }
                '!' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::Ne);
                    } else {
                        return Err(QfError::SyntaxError {
                            position: self.pos - 1,
                            message: "unexpected '!', did you mean 'not'?".into(),
                        });
                    }
                }
                '<' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::Le);
                    } else {
                        self.tokens.push(Token::Lt);
                    }
                }
                '>' => {
                    self.pos += 1;
                    if self.peek_current() == Some('=') {
                        self.pos += 1;
                        self.tokens.push(Token::Ge);
                    } else {
                        self.tokens.push(Token::Gt);
                    }
                }
                '"' => {
                    self.read_string()?;
                }
                '@' => {
                    self.pos += 1;
                    let start = self.pos;
                    while self.pos < self.input.len()
                        && (self.input[self.pos].is_ascii_alphanumeric()
                            || self.input[self.pos] == '_')
                    {
                        self.pos += 1;
                    }
                    let name: String = self.input[start..self.pos].iter().collect();
                    if name.is_empty() {
                        return Err(QfError::SyntaxError {
                            position: self.pos,
                            message: "expected format name after '@'".into(),
                        });
                    }
                    self.tokens.push(Token::Format(name));
                }
                '$' => {
                    self.pos += 1;
                    let start = self.pos;
                    while self.pos < self.input.len()
                        && (self.input[self.pos].is_ascii_alphanumeric()
                            || self.input[self.pos] == '_')
                    {
                        self.pos += 1;
                    }
                    let name: String = self.input[start..self.pos].iter().collect();
                    if name.is_empty() {
                        return Err(QfError::SyntaxError {
                            position: self.pos,
                            message: "expected variable name after '$'".into(),
                        });
                    }
                    self.tokens.push(Token::Variable(name));
                }
                c if c.is_ascii_digit() => {
                    self.read_number()?;
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    self.read_ident();
                }
                _ => {
                    return Err(QfError::SyntaxError {
                        position: self.pos,
                        message: format!("unexpected character: '{ch}'"),
                    });
                }
            }
        }

        self.tokens.push(Token::Eof);
        Ok(&self.tokens)
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek_current(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.input.get(self.pos + 1).copied()
    }

    fn read_number(&mut self) -> Result<(), QfError> {
        let start = self.pos;
        // Handle leading dot for decimals like .5
        if self.pos < self.input.len() && self.input[self.pos] == '.' {
            self.pos += 1;
        }
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos < self.input.len() && self.input[self.pos] == '.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        // Scientific notation
        if self.pos < self.input.len()
            && (self.input[self.pos] == 'e' || self.input[self.pos] == 'E')
        {
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == '+' || self.input[self.pos] == '-')
            {
                self.pos += 1;
            }
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let num_str: String = self.input[start..self.pos].iter().collect();
        let n: f64 = num_str.parse().map_err(|_| QfError::SyntaxError {
            position: start,
            message: format!("invalid number: {num_str}"),
        })?;
        self.tokens.push(Token::Number(n));
        Ok(())
    }

    fn read_string(&mut self) -> Result<(), QfError> {
        self.pos += 1; // skip opening quote
        let mut s = String::new();
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                '"' => {
                    self.pos += 1;
                    self.tokens.push(Token::String(s));
                    return Ok(());
                }
                '\\' => {
                    self.pos += 1;
                    if self.pos >= self.input.len() {
                        return Err(QfError::SyntaxError {
                            position: self.pos,
                            message: "unterminated string escape".into(),
                        });
                    }
                    match self.input[self.pos] {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '/' => s.push('/'),
                        '(' => {
                            // String interpolation: \(expr)
                            // We emit the accumulated string, then a special sequence
                            // We handle this by emitting tokens inline
                            self.tokens.push(Token::String(std::mem::take(&mut s)));
                            self.tokens.push(Token::Plus);
                            self.pos += 1; // skip '('

                            // We need to lex until matching ')' and emit those tokens
                            // wrapped in tostring
                            self.tokens.push(Token::LParen);
                            let mut depth = 1;
                            let interp_start = self.pos;
                            while self.pos < self.input.len() && depth > 0 {
                                match self.input[self.pos] {
                                    '(' => depth += 1,
                                    ')' => depth -= 1,
                                    _ => {}
                                }
                                if depth > 0 {
                                    self.pos += 1;
                                }
                            }
                            // Re-lex the interpolated expression
                            let interp_str: String =
                                self.input[interp_start..self.pos].iter().collect();
                            let mut sub_lexer = Lexer::new(&interp_str);
                            sub_lexer.tokenize()?;
                            // Remove the Eof from sub-tokens
                            if let Some(Token::Eof) = sub_lexer.tokens.last() {
                                sub_lexer.tokens.pop();
                            }
                            // Add pipe to tostring before closing
                            self.tokens.extend(sub_lexer.tokens);
                            self.tokens.push(Token::Pipe);
                            self.tokens.push(Token::Ident("tostring".into()));
                            self.tokens.push(Token::RParen);
                            self.tokens.push(Token::Plus);

                            self.pos += 1; // skip ')'
                            continue;
                        }
                        'u' => {
                            self.pos += 1;
                            let hex_start = self.pos;
                            for _ in 0..4 {
                                if self.pos >= self.input.len() {
                                    return Err(QfError::SyntaxError {
                                        position: self.pos,
                                        message: "incomplete unicode escape".into(),
                                    });
                                }
                                self.pos += 1;
                            }
                            let hex: String = self.input[hex_start..self.pos].iter().collect();
                            let code = u32::from_str_radix(&hex, 16).map_err(|_| {
                                QfError::SyntaxError {
                                    position: hex_start,
                                    message: format!("invalid unicode escape: \\u{hex}"),
                                }
                            })?;
                            if let Some(c) = char::from_u32(code) {
                                s.push(c);
                            }
                            continue;
                        }
                        c => {
                            return Err(QfError::SyntaxError {
                                position: self.pos,
                                message: format!("invalid escape character: '\\{c}'"),
                            });
                        }
                    }
                    self.pos += 1;
                }
                c => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
        Err(QfError::SyntaxError {
            position: self.pos,
            message: "unterminated string literal".into(),
        })
    }

    fn read_ident(&mut self) {
        let start = self.pos;
        while self.pos < self.input.len()
            && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == '_')
        {
            self.pos += 1;
        }
        let word: String = self.input[start..self.pos].iter().collect();
        if let Some(kw) = Token::is_keyword(&word) {
            self.tokens.push(kw);
        } else {
            self.tokens.push(Token::Ident(word));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(input: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(input);
        lexer.tokenize().unwrap();
        lexer.tokens
    }

    #[test]
    fn lex_dot() {
        let tokens = lex(".");
        assert_eq!(tokens, vec![Token::Dot, Token::Eof]);
    }

    #[test]
    fn lex_path() {
        let tokens = lex(".foo.bar");
        assert_eq!(
            tokens,
            vec![
                Token::Dot,
                Token::Ident("foo".into()),
                Token::Dot,
                Token::Ident("bar".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lex_index() {
        let tokens = lex(".[0]");
        assert_eq!(
            tokens,
            vec![
                Token::Dot,
                Token::LBracket,
                Token::Number(0.0),
                Token::RBracket,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lex_pipe() {
        let tokens = lex(".foo | .bar");
        assert_eq!(
            tokens,
            vec![
                Token::Dot,
                Token::Ident("foo".into()),
                Token::Pipe,
                Token::Dot,
                Token::Ident("bar".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lex_operators() {
        let tokens = lex("== != < <= > >= + - * / %");
        assert_eq!(
            tokens,
            vec![
                Token::Eq, Token::Ne, Token::Lt, Token::Le, Token::Gt, Token::Ge,
                Token::Plus, Token::Minus, Token::Star, Token::Slash, Token::Percent,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lex_keywords() {
        let tokens = lex("if then else end and or not");
        assert_eq!(
            tokens,
            vec![
                Token::If, Token::Then, Token::Else, Token::End,
                Token::And, Token::Or, Token::Not, Token::Eof,
            ]
        );
    }

    #[test]
    fn lex_string() {
        let tokens = lex(r#""hello world""#);
        assert_eq!(tokens, vec![Token::String("hello world".into()), Token::Eof]);
    }

    #[test]
    fn lex_variable() {
        let tokens = lex("$x");
        assert_eq!(tokens, vec![Token::Variable("x".into()), Token::Eof]);
    }

    #[test]
    fn lex_alternative() {
        let tokens = lex(".a // .b");
        assert_eq!(
            tokens,
            vec![
                Token::Dot,
                Token::Ident("a".into()),
                Token::Alternative,
                Token::Dot,
                Token::Ident("b".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lex_format() {
        let tokens = lex("@base64");
        assert_eq!(tokens, vec![Token::Format("base64".into()), Token::Eof]);
    }

    #[test]
    fn lex_assignment() {
        let tokens = lex("|= += -= *= /= %= //=");
        assert_eq!(
            tokens,
            vec![
                Token::UpdateAssign, Token::PlusAssign, Token::MinusAssign,
                Token::StarAssign, Token::SlashAssign, Token::PercentAssign,
                Token::AltAssign, Token::Eof,
            ]
        );
    }
}
