use crate::error::QfError;

use super::ast::*;
use super::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    pub fn parse(&mut self) -> Result<Expr, QfError> {
        let expr = self.parse_pipe()?;
        if !self.at_eof() {
            return Err(self.error(format!(
                "unexpected token: {:?}",
                self.current()
            )));
        }
        Ok(expr)
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos + 1).unwrap_or(&Token::Eof)
    }

    fn at_eof(&self) -> bool {
        matches!(self.current(), Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), QfError> {
        if self.current() == expected {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!(
                "expected {:?}, got {:?}",
                expected,
                self.current()
            )))
        }
    }

    fn error(&self, message: String) -> QfError {
        QfError::SyntaxError {
            position: self.pos,
            message,
        }
    }

    // ── Precedence levels (lowest to highest) ──────────────────────

    /// Top-level expression: comma-separated pipes
    /// pipe_or_comma: pipe (',' pipe)*
    fn parse_pipe(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_pipe_no_comma()?;
        while matches!(self.current(), Token::Comma) {
            self.advance();
            let right = self.parse_pipe_no_comma()?;
            expr = Expr::Comma(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    /// Pipe without comma: assign ('|' assign)*
    /// Used for object values and other contexts where comma is a delimiter
    fn parse_pipe_no_comma(&mut self) -> Result<Expr, QfError> {
        // Handle `def` at pipe level
        if matches!(self.current(), Token::Def) {
            return self.parse_funcdef();
        }

        // Handle `label $name |` at pipe level
        if matches!(self.current(), Token::Label) {
            return self.parse_label();
        }

        let mut expr = self.parse_assign()?;

        // Handle `as $var |`
        if matches!(self.current(), Token::As) {
            self.advance(); // skip 'as'
            let pattern = self.parse_pattern()?;
            self.expect(&Token::Pipe)?;
            let body = self.parse_pipe()?;
            return Ok(Expr::As {
                expr: Box::new(expr),
                pattern,
                body: Box::new(body),
            });
        }

        while matches!(self.current(), Token::Pipe) {
            self.advance();
            let right = self.parse_assign()?;
            expr = Expr::Pipe(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    /// assign: or ('=' pipe | '|=' pipe | '+=' pipe | ...)?
    fn parse_assign(&mut self) -> Result<Expr, QfError> {
        let expr = self.parse_or()?;
        match self.current() {
            Token::Assign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::Assign(Box::new(expr), Box::new(val)))
            }
            Token::UpdateAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::UpdateAssign(Box::new(expr), Box::new(val)))
            }
            Token::PlusAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::ArithAssign(BinOp::Add, Box::new(expr), Box::new(val)))
            }
            Token::MinusAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::ArithAssign(BinOp::Sub, Box::new(expr), Box::new(val)))
            }
            Token::StarAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::ArithAssign(BinOp::Mul, Box::new(expr), Box::new(val)))
            }
            Token::SlashAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::ArithAssign(BinOp::Div, Box::new(expr), Box::new(val)))
            }
            Token::PercentAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::ArithAssign(BinOp::Mod, Box::new(expr), Box::new(val)))
            }
            Token::AltAssign => {
                self.advance();
                let val = self.parse_pipe()?;
                Ok(Expr::AltAssign(Box::new(expr), Box::new(val)))
            }
            _ => Ok(expr),
        }
    }

    /// or: and ('or' and)*
    fn parse_or(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_and()?;
        while matches!(self.current(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            expr = Expr::BinOp(BinOp::Or, Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    /// and: not_expr ('and' not_expr)*
    fn parse_and(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_not()?;
        while matches!(self.current(), Token::And) {
            self.advance();
            let right = self.parse_not()?;
            expr = Expr::BinOp(BinOp::And, Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    /// not: 'not' comparison | comparison
    fn parse_not(&mut self) -> Result<Expr, QfError> {
        // In jq, `not` is a filter, not a prefix operator.
        // It appears after pipe: `.foo | not`
        // But we handle it at comparison level for simplicity.
        let expr = self.parse_comparison()?;
        Ok(expr)
    }

    /// comparison: alternative (('==' | '!=' | '<' | '<=' | '>' | '>=') alternative)?
    fn parse_comparison(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_alternative()?;
        loop {
            let op = match self.current() {
                Token::Eq => BinOp::Eq,
                Token::Ne => BinOp::Ne,
                Token::Lt => BinOp::Lt,
                Token::Le => BinOp::Le,
                Token::Gt => BinOp::Gt,
                Token::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.parse_alternative()?;
            expr = Expr::BinOp(op, Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    /// alternative: addition ('//' addition)*
    fn parse_alternative(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_addition()?;
        while matches!(self.current(), Token::Alternative) {
            self.advance();
            let right = self.parse_addition()?;
            expr = Expr::Alternative(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    /// addition: multiplication (('+' | '-') multiplication)*
    fn parse_addition(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_multiplication()?;
        loop {
            match self.current() {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_multiplication()?;
                    expr = Expr::BinOp(BinOp::Add, Box::new(expr), Box::new(right));
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_multiplication()?;
                    expr = Expr::BinOp(BinOp::Sub, Box::new(expr), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// multiplication: unary (('*' | '/' | '%') unary)*
    fn parse_multiplication(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_unary()?;
        loop {
            match self.current() {
                Token::Star => {
                    self.advance();
                    let right = self.parse_unary()?;
                    expr = Expr::BinOp(BinOp::Mul, Box::new(expr), Box::new(right));
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_unary()?;
                    expr = Expr::BinOp(BinOp::Div, Box::new(expr), Box::new(right));
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_unary()?;
                    expr = Expr::BinOp(BinOp::Mod, Box::new(expr), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// unary: '-' unary | postfix
    fn parse_unary(&mut self) -> Result<Expr, QfError> {
        if matches!(self.current(), Token::Minus) {
            // Need to distinguish unary minus from binary minus
            // Unary minus if previous token is operator or start of expr
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::Neg(Box::new(expr)));
        }
        self.parse_postfix()
    }

    /// postfix: primary ('.' ident | '[' expr ']' | '[]' | '?')*
    fn parse_postfix(&mut self) -> Result<Expr, QfError> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.current() {
                Token::Dot => {
                    // Could be .field or just . (identity at start)
                    if matches!(self.peek(), Token::Ident(_)) {
                        self.advance(); // skip dot
                        if let Token::Ident(name) = self.advance() {
                            // Check for optional
                            if matches!(self.current(), Token::Question) {
                                self.advance();
                                expr = Expr::Pipe(
                                    Box::new(expr),
                                    Box::new(Expr::OptionalField(name)),
                                );
                            } else {
                                expr = Expr::Pipe(
                                    Box::new(expr),
                                    Box::new(Expr::Field(name)),
                                );
                            }
                        }
                    } else {
                        break;
                    }
                }
                Token::LBracket => {
                    self.advance(); // skip [
                    if matches!(self.current(), Token::RBracket) {
                        self.advance();
                        // Check for optional
                        if matches!(self.current(), Token::Question) {
                            self.advance();
                            expr = Expr::OptionalIterate(Box::new(expr));
                        } else {
                            expr = Expr::Iterate(Box::new(expr));
                        }
                    } else if matches!(self.current(), Token::Colon) {
                        // Slice: [: N]
                        self.advance(); // skip :
                        let end = self.parse_pipe()?;
                        self.expect(&Token::RBracket)?;
                        expr = Expr::Slice(Box::new(expr), None, Some(Box::new(end)));
                    } else {
                        let idx = self.parse_pipe()?;
                        if matches!(self.current(), Token::Colon) {
                            // Slice: [M:N] or [M:]
                            self.advance(); // skip :
                            let end = if matches!(self.current(), Token::RBracket) {
                                None
                            } else {
                                Some(Box::new(self.parse_pipe()?))
                            };
                            self.expect(&Token::RBracket)?;
                            expr = Expr::Slice(Box::new(expr), Some(Box::new(idx)), end);
                        } else {
                            self.expect(&Token::RBracket)?;
                            // Check for optional
                            if matches!(self.current(), Token::Question) {
                                self.advance();
                                expr = Expr::OptionalIndex(Box::new(expr), Box::new(idx));
                            } else {
                                expr = Expr::Index(Box::new(expr), Box::new(idx));
                            }
                        }
                    }
                }
                Token::Question => {
                    self.advance();
                    expr = Expr::Optional(Box::new(expr));
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// primary: atoms and prefix constructs
    fn parse_primary(&mut self) -> Result<Expr, QfError> {
        match self.current().clone() {
            Token::Dot => {
                self.advance();
                // Check if followed by identifier (field access)
                match self.current() {
                    Token::Ident(name) => {
                        let name = name.clone();
                        self.advance();
                        if matches!(self.current(), Token::Question) {
                            self.advance();
                            Ok(Expr::OptionalField(name))
                        } else {
                            Ok(Expr::Field(name))
                        }
                    }
                    Token::LBracket => {
                        // .[expr] or .[] — handled as postfix on identity
                        // Return identity, let postfix handle brackets
                        Ok(Expr::Identity)
                    }
                    Token::String(s) => {
                        // ."field-with-special-chars"
                        let s = s.clone();
                        self.advance();
                        Ok(Expr::Field(s))
                    }
                    _ => Ok(Expr::Identity),
                }
            }
            Token::DotDot => {
                self.advance();
                Ok(Expr::RecurseAll)
            }
            Token::Number(n) => {
                self.advance();
                if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                    Ok(Expr::Literal(serde_json::Value::Number(
                        serde_json::Number::from(n as i64),
                    )))
                } else {
                    Ok(Expr::Literal(serde_json::Value::Number(
                        serde_json::Number::from_f64(n)
                            .unwrap_or_else(|| serde_json::Number::from(0)),
                    )))
                }
            }
            Token::String(s) => {
                self.advance();
                // Check if this is part of string interpolation (string + (...) + string)
                // The lexer already handles interpolation by emitting String + Plus + (expr | tostring) + Plus
                Ok(Expr::Literal(serde_json::Value::String(s)))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Literal(serde_json::Value::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Literal(serde_json::Value::Bool(false)))
            }
            Token::Null => {
                self.advance();
                Ok(Expr::Literal(serde_json::Value::Null))
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_pipe()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBracket => {
                self.advance();
                if matches!(self.current(), Token::RBracket) {
                    self.advance();
                    Ok(Expr::ArrayConstruct(Box::new(Expr::Literal(
                        serde_json::Value::Array(vec![]),
                    ))))
                } else {
                    let inner = self.parse_pipe()?;
                    self.expect(&Token::RBracket)?;
                    Ok(Expr::ArrayConstruct(Box::new(inner)))
                }
            }
            Token::LBrace => {
                self.parse_object_construct()
            }
            Token::If => {
                self.parse_if()
            }
            Token::Try => {
                self.advance();
                let expr = self.parse_postfix()?;
                let catch = if matches!(self.current(), Token::Catch) {
                    self.advance();
                    Some(Box::new(self.parse_postfix()?))
                } else {
                    None
                };
                Ok(Expr::Try(Box::new(expr), catch))
            }
            Token::Reduce => {
                self.parse_reduce()
            }
            Token::Foreach => {
                self.parse_foreach()
            }
            Token::Not => {
                self.advance();
                // `not` in jq is a postfix/function, but can appear as prefix in some contexts
                // We treat it as a function call
                Ok(Expr::FuncCall("not".into(), vec![]))
            }
            Token::Variable(name) => {
                self.advance();
                Ok(Expr::VarRef(name))
            }
            Token::Format(name) => {
                self.advance();
                Ok(Expr::Format(name))
            }
            Token::Ident(name) => {
                self.advance();
                // Check if it's a function call with args
                if matches!(self.current(), Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.current(), Token::RParen) {
                        args.push(self.parse_pipe()?);
                        while matches!(self.current(), Token::Semicolon) {
                            self.advance();
                            args.push(self.parse_pipe()?);
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::FuncCall(name, args))
                } else {
                    Ok(Expr::FuncCall(name, vec![]))
                }
            }
            Token::Break => {
                self.advance();
                if let Token::Variable(name) = self.current().clone() {
                    self.advance();
                    Ok(Expr::Break(name))
                } else {
                    Err(self.error("expected $variable after 'break'".into()))
                }
            }
            Token::Minus => {
                // Unary minus handled by parse_unary
                self.advance();
                let expr = self.parse_postfix()?;
                Ok(Expr::Neg(Box::new(expr)))
            }
            _ => Err(self.error(format!("unexpected token: {:?}", self.current()))),
        }
    }

    fn parse_object_construct(&mut self) -> Result<Expr, QfError> {
        self.advance(); // skip {
        let mut entries = Vec::new();

        if !matches!(self.current(), Token::RBrace) {
            entries.push(self.parse_object_entry()?);
            while matches!(self.current(), Token::Comma) {
                self.advance();
                if matches!(self.current(), Token::RBrace) {
                    break; // trailing comma
                }
                entries.push(self.parse_object_entry()?);
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(Expr::ObjectConstruct(entries))
    }

    fn parse_object_entry(&mut self) -> Result<ObjectEntry, QfError> {
        match self.current().clone() {
            Token::Ident(name) => {
                self.advance();
                if matches!(self.current(), Token::Colon) {
                    self.advance();
                    let value = self.parse_pipe_no_comma()?;
                    Ok(ObjectEntry::KeyValue(ObjectKey::Ident(name), value))
                } else {
                    // Shorthand: {name} means {name: .name}
                    Ok(ObjectEntry::Shorthand(name))
                }
            }
            Token::String(s) => {
                self.advance();
                if matches!(self.current(), Token::Colon) {
                    self.advance();
                    let value = self.parse_pipe_no_comma()?;
                    Ok(ObjectEntry::KeyValue(ObjectKey::String(s), value))
                } else {
                    Ok(ObjectEntry::KeyValue(
                        ObjectKey::String(s.clone()),
                        Expr::Field(s),
                    ))
                }
            }
            Token::Variable(name) => {
                self.advance();
                if matches!(self.current(), Token::Colon) {
                    self.advance();
                    let value = self.parse_pipe_no_comma()?;
                    Ok(ObjectEntry::ComputedKeyValue(
                        Expr::VarRef(name),
                        value,
                    ))
                } else {
                    Ok(ObjectEntry::ShorthandVar(name))
                }
            }
            Token::LParen => {
                self.advance();
                let key_expr = self.parse_pipe()?;
                self.expect(&Token::RParen)?;
                self.expect(&Token::Colon)?;
                let value = self.parse_pipe_no_comma()?;
                Ok(ObjectEntry::ComputedKeyValue(key_expr, value))
            }
            Token::Format(name) => {
                self.advance();
                Ok(ObjectEntry::ShorthandFormat(name))
            }
            Token::Dot => {
                // .field: value shorthand
                let key_expr = self.parse_primary()?;
                if matches!(self.current(), Token::Colon) {
                    self.advance();
                    let value = self.parse_pipe_no_comma()?;
                    Ok(ObjectEntry::ComputedKeyValue(key_expr, value))
                } else {
                    Ok(ObjectEntry::ComputedKeyValue(
                        key_expr.clone(),
                        key_expr,
                    ))
                }
            }
            _ => Err(self.error(format!(
                "expected object key, got {:?}",
                self.current()
            ))),
        }
    }

    fn parse_if(&mut self) -> Result<Expr, QfError> {
        self.advance(); // skip 'if'
        let cond = self.parse_pipe()?;
        self.expect(&Token::Then)?;
        let then_branch = self.parse_pipe()?;

        let mut elif_branches = Vec::new();
        while matches!(self.current(), Token::Elif) {
            self.advance();
            let elif_cond = self.parse_pipe()?;
            self.expect(&Token::Then)?;
            let elif_body = self.parse_pipe()?;
            elif_branches.push((elif_cond, elif_body));
        }

        let else_branch = if matches!(self.current(), Token::Else) {
            self.advance();
            Some(Box::new(self.parse_pipe()?))
        } else {
            None
        };

        self.expect(&Token::End)?;
        Ok(Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            elif_branches,
            else_branch,
        })
    }

    fn parse_reduce(&mut self) -> Result<Expr, QfError> {
        self.advance(); // skip 'reduce'
        let expr = self.parse_postfix()?;
        self.expect(&Token::As)?;
        let pattern = self.parse_pattern()?;
        self.expect(&Token::LParen)?;
        let init = self.parse_pipe()?;
        self.expect(&Token::Semicolon)?;
        let update = self.parse_pipe()?;
        self.expect(&Token::RParen)?;
        Ok(Expr::Reduce {
            expr: Box::new(expr),
            pattern,
            init: Box::new(init),
            update: Box::new(update),
        })
    }

    fn parse_foreach(&mut self) -> Result<Expr, QfError> {
        self.advance(); // skip 'foreach'
        let expr = self.parse_postfix()?;
        self.expect(&Token::As)?;
        let pattern = self.parse_pattern()?;
        self.expect(&Token::LParen)?;
        let init = self.parse_pipe()?;
        self.expect(&Token::Semicolon)?;
        let update = self.parse_pipe()?;
        let extract = if matches!(self.current(), Token::Semicolon) {
            self.advance();
            Some(Box::new(self.parse_pipe()?))
        } else {
            None
        };
        self.expect(&Token::RParen)?;
        Ok(Expr::Foreach {
            expr: Box::new(expr),
            pattern,
            init: Box::new(init),
            update: Box::new(update),
            extract,
        })
    }

    fn parse_funcdef(&mut self) -> Result<Expr, QfError> {
        self.advance(); // skip 'def'
        let name = match self.advance() {
            Token::Ident(name) => name,
            other => return Err(self.error(format!("expected function name, got {:?}", other))),
        };

        let mut params = Vec::new();
        if matches!(self.current(), Token::LParen) {
            self.advance();
            if !matches!(self.current(), Token::RParen) {
                match self.advance() {
                    Token::Ident(p) => params.push(p),
                    other => {
                        return Err(
                            self.error(format!("expected parameter name, got {:?}", other))
                        )
                    }
                }
                while matches!(self.current(), Token::Semicolon) {
                    self.advance();
                    match self.advance() {
                        Token::Ident(p) => params.push(p),
                        other => {
                            return Err(
                                self.error(format!("expected parameter name, got {:?}", other))
                            )
                        }
                    }
                }
            }
            self.expect(&Token::RParen)?;
        }

        self.expect(&Token::Colon)?;
        let body = self.parse_pipe()?;
        self.expect(&Token::Semicolon)?;
        let rest = self.parse_pipe()?;

        Ok(Expr::FuncDef {
            name,
            params,
            body: Box::new(body),
            rest: Box::new(rest),
        })
    }

    fn parse_label(&mut self) -> Result<Expr, QfError> {
        self.advance(); // skip 'label'
        let name = match self.advance() {
            Token::Variable(name) => name,
            other => return Err(self.error(format!("expected $variable after 'label', got {:?}", other))),
        };
        self.expect(&Token::Pipe)?;
        let body = self.parse_pipe()?;
        Ok(Expr::Label(name, Box::new(body)))
    }

    fn parse_pattern(&mut self) -> Result<Pattern, QfError> {
        match self.current().clone() {
            Token::Variable(name) => {
                self.advance();
                Ok(Pattern::Variable(name))
            }
            Token::LBracket => {
                self.advance();
                let mut patterns = Vec::new();
                if !matches!(self.current(), Token::RBracket) {
                    patterns.push(self.parse_pattern()?);
                    while matches!(self.current(), Token::Comma) {
                        self.advance();
                        patterns.push(self.parse_pattern()?);
                    }
                }
                self.expect(&Token::RBracket)?;
                Ok(Pattern::Array(patterns))
            }
            Token::LBrace => {
                self.advance();
                let mut fields = Vec::new();
                if !matches!(self.current(), Token::RBrace) {
                    let key = match self.advance() {
                        Token::Ident(k) => k,
                        other => {
                            return Err(self.error(format!(
                                "expected field name in pattern, got {:?}",
                                other
                            )))
                        }
                    };
                    self.expect(&Token::Colon)?;
                    let pat = self.parse_pattern()?;
                    fields.push((key, pat));
                    while matches!(self.current(), Token::Comma) {
                        self.advance();
                        let key = match self.advance() {
                            Token::Ident(k) => k,
                            other => {
                                return Err(self.error(format!(
                                    "expected field name in pattern, got {:?}",
                                    other
                                )))
                            }
                        };
                        self.expect(&Token::Colon)?;
                        let pat = self.parse_pattern()?;
                        fields.push((key, pat));
                    }
                }
                self.expect(&Token::RBrace)?;
                Ok(Pattern::Object(fields))
            }
            _ => Err(self.error(format!(
                "expected pattern ($var, [...], or {{...}}), got {:?}",
                self.current()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::lexer::Lexer;

    fn parse_expr(input: &str) -> Expr {
        let mut lexer = Lexer::new(input);
        lexer.tokenize().unwrap();
        let mut parser = Parser::new(lexer.tokens);
        parser.parse().unwrap()
    }

    #[test]
    fn parse_identity() {
        assert_eq!(parse_expr("."), Expr::Identity);
    }

    #[test]
    fn parse_field() {
        assert_eq!(parse_expr(".foo"), Expr::Field("foo".into()));
    }

    #[test]
    fn parse_nested_field() {
        let expr = parse_expr(".foo.bar");
        assert_eq!(
            expr,
            Expr::Pipe(
                Box::new(Expr::Field("foo".into())),
                Box::new(Expr::Field("bar".into())),
            )
        );
    }

    #[test]
    fn parse_index() {
        let expr = parse_expr(".[0]");
        assert_eq!(
            expr,
            Expr::Index(
                Box::new(Expr::Identity),
                Box::new(Expr::Literal(serde_json::Value::Number(0.into()))),
            )
        );
    }

    #[test]
    fn parse_iterate() {
        let expr = parse_expr(".[]");
        assert_eq!(expr, Expr::Iterate(Box::new(Expr::Identity)));
    }

    #[test]
    fn parse_pipe_expr() {
        let expr = parse_expr(".foo | .bar");
        assert_eq!(
            expr,
            Expr::Pipe(
                Box::new(Expr::Field("foo".into())),
                Box::new(Expr::Field("bar".into())),
            )
        );
    }

    #[test]
    fn parse_addition() {
        let expr = parse_expr(".a + .b");
        assert_eq!(
            expr,
            Expr::BinOp(
                BinOp::Add,
                Box::new(Expr::Field("a".into())),
                Box::new(Expr::Field("b".into())),
            )
        );
    }

    #[test]
    fn parse_comparison() {
        let expr = parse_expr(".a == .b");
        assert_eq!(
            expr,
            Expr::BinOp(
                BinOp::Eq,
                Box::new(Expr::Field("a".into())),
                Box::new(Expr::Field("b".into())),
            )
        );
    }

    #[test]
    fn parse_array_construct() {
        let expr = parse_expr("[.a, .b]");
        assert_eq!(
            expr,
            Expr::ArrayConstruct(Box::new(Expr::Comma(
                Box::new(Expr::Field("a".into())),
                Box::new(Expr::Field("b".into())),
            )))
        );
    }

    #[test]
    fn parse_if_then_else() {
        let expr = parse_expr("if .a then .b else .c end");
        assert_eq!(
            expr,
            Expr::If {
                cond: Box::new(Expr::Field("a".into())),
                then_branch: Box::new(Expr::Field("b".into())),
                elif_branches: vec![],
                else_branch: Some(Box::new(Expr::Field("c".into()))),
            }
        );
    }

    #[test]
    fn parse_select() {
        let expr = parse_expr("select(.a > 1)");
        assert_eq!(
            expr,
            Expr::FuncCall(
                "select".into(),
                vec![Expr::BinOp(
                    BinOp::Gt,
                    Box::new(Expr::Field("a".into())),
                    Box::new(Expr::Literal(serde_json::Value::Number(1.into()))),
                )],
            )
        );
    }

    #[test]
    fn parse_object_construct() {
        let expr = parse_expr("{name: .foo, value: .bar}");
        assert_eq!(
            expr,
            Expr::ObjectConstruct(vec![
                ObjectEntry::KeyValue(
                    ObjectKey::Ident("name".into()),
                    Expr::Field("foo".into()),
                ),
                ObjectEntry::KeyValue(
                    ObjectKey::Ident("value".into()),
                    Expr::Field("bar".into()),
                ),
            ])
        );
    }

    #[test]
    fn parse_reduce() {
        let expr = parse_expr("reduce .[] as $x (0; . + $x)");
        assert_eq!(
            expr,
            Expr::Reduce {
                expr: Box::new(Expr::Iterate(Box::new(Expr::Identity))),
                pattern: Pattern::Variable("x".into()),
                init: Box::new(Expr::Literal(serde_json::Value::Number(0.into()))),
                update: Box::new(Expr::BinOp(
                    BinOp::Add,
                    Box::new(Expr::Identity),
                    Box::new(Expr::VarRef("x".into())),
                )),
            }
        );
    }
}
