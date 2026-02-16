use std::collections::HashMap;

use serde_json::Value;

use super::ast::Expr;

#[derive(Debug, Clone)]
pub struct FuncDef {
    pub params: Vec<String>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub struct Env {
    variables: HashMap<String, Value>,
    functions: HashMap<(String, usize), FuncDef>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            variables: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    pub fn get_var(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }

    pub fn set_var(&mut self, name: String, value: Value) {
        self.variables.insert(name, value);
    }

    pub fn get_func(&self, name: &str, arity: usize) -> Option<&FuncDef> {
        self.functions.get(&(name.to_string(), arity))
    }

    pub fn set_func(&mut self, name: String, arity: usize, def: FuncDef) {
        self.functions.insert((name, arity), def);
    }

    pub fn child(&self) -> Self {
        self.clone()
    }
}
