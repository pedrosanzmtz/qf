/// AST node for jq expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Identity: `.`
    Identity,

    /// Recursive descent: `..`
    RecurseAll,

    /// Field access: `.foo`
    Field(String),

    /// Optional field access: `.foo?`
    OptionalField(String),

    /// Index: `.[N]`
    Index(Box<Expr>, Box<Expr>),

    /// Optional index: `.[N]?`
    OptionalIndex(Box<Expr>, Box<Expr>),

    /// Slice: `.[M:N]`
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),

    /// Iterator: `.[]`
    Iterate(Box<Expr>),

    /// Optional iterator: `.[]?`
    OptionalIterate(Box<Expr>),

    /// Pipe: `expr | expr`
    Pipe(Box<Expr>, Box<Expr>),

    /// Comma: `expr, expr` (multiple outputs)
    Comma(Box<Expr>, Box<Expr>),

    /// Literal number
    Literal(serde_json::Value),

    /// String literal (may be result of interpolation concatenation)
    StringLiteral(String),

    /// Unary negation: `-expr`
    Neg(Box<Expr>),

    /// Arithmetic / comparison / logic: `expr op expr`
    BinOp(BinOp, Box<Expr>, Box<Expr>),

    /// Logical not: `expr | not`
    Not(Box<Expr>),

    /// Alternative: `expr // expr`
    Alternative(Box<Expr>, Box<Expr>),

    /// Try: `expr?` or `try expr`
    Try(Box<Expr>, Option<Box<Expr>>),

    /// Array construction: `[expr]`
    ArrayConstruct(Box<Expr>),

    /// Object construction: `{key: value, ...}`
    ObjectConstruct(Vec<ObjectEntry>),

    /// If-then-elif-else-end
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        elif_branches: Vec<(Expr, Expr)>,
        else_branch: Option<Box<Expr>>,
    },

    /// Variable binding: `expr as $var | body`
    As {
        expr: Box<Expr>,
        pattern: Pattern,
        body: Box<Expr>,
    },

    /// Reduce: `reduce expr as $var (init; update)`
    Reduce {
        expr: Box<Expr>,
        pattern: Pattern,
        init: Box<Expr>,
        update: Box<Expr>,
    },

    /// Foreach: `foreach expr as $var (init; update; extract)`
    Foreach {
        expr: Box<Expr>,
        pattern: Pattern,
        init: Box<Expr>,
        update: Box<Expr>,
        extract: Option<Box<Expr>>,
    },

    /// Label-break: `label $name | expr`
    Label(String, Box<Expr>),

    /// Break: `break $name`
    Break(String),

    /// Function definition: `def name(params): body;`
    FuncDef {
        name: String,
        params: Vec<String>,
        body: Box<Expr>,
        rest: Box<Expr>,
    },

    /// Function call: `name`, `name(args)`
    FuncCall(String, Vec<Expr>),

    /// Variable reference: `$name`
    VarRef(String),

    /// Assignment: `path = value`
    Assign(Box<Expr>, Box<Expr>),

    /// Update assignment: `path |= value`
    UpdateAssign(Box<Expr>, Box<Expr>),

    /// Arithmetic assignment: `path += value`, etc.
    ArithAssign(BinOp, Box<Expr>, Box<Expr>),

    /// Alternative assignment: `path //= value`
    AltAssign(Box<Expr>, Box<Expr>),

    /// Format string: `@base64`, `@csv`, etc.
    Format(String),

    /// Optional operator applied to expression
    Optional(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectEntry {
    /// `key: value` where key is a fixed identifier
    KeyValue(ObjectKey, Expr),
    /// `(expr): value` where key is computed
    ComputedKeyValue(Expr, Expr),
    /// Just an identifier (shorthand for `key: .key`)
    Shorthand(String),
    /// `@base64` or similar (shorthand in object)
    ShorthandFormat(String),
    /// `$var` shorthand for `($var): $var`
    ShorthandVar(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectKey {
    Ident(String),
    String(String),
    Format(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Variable(String),
    Array(Vec<Pattern>),
    Object(Vec<(String, Pattern)>),
}
