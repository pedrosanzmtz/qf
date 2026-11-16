use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;

use super::ast::Expr;

#[derive(Debug, Clone)]
pub enum FuncDef {
    User { params: Vec<String>, body: Expr },
    Closure { expr: Expr, env: Box<Env> },
}

#[derive(Debug, Clone, Default)]
struct Scope {
    parent: Option<Rc<Scope>>,
    variables: HashMap<String, Value>,
    functions: HashMap<(String, usize), FuncDef>,
}

/// Persistent lexical environment backed by an `Rc`-linked scope chain.
///
/// Creating a child scope (`child()`) or cloning an environment is O(1), and
/// mutations use `Rc::make_mut` so only the current frame is copied if shared.
#[derive(Debug, Clone, Default)]
pub struct Env {
    scope: Rc<Scope>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_var(&self, name: &str) -> Option<&Value> {
        let mut cur = Some(&*self.scope);
        while let Some(scope) = cur {
            if let Some(val) = scope.variables.get(name) {
                return Some(val);
            }
            cur = scope.parent.as_deref();
        }
        None
    }

    pub fn set_var(&mut self, name: String, value: Value) {
        Rc::make_mut(&mut self.scope).variables.insert(name, value);
    }

    pub fn get_func(&self, name: &str, arity: usize) -> Option<&FuncDef> {
        let mut cur = Some(&*self.scope);
        while let Some(scope) = cur {
            if !scope.functions.is_empty() {
                if let Some(func) = scope.functions.get(&(name.to_string(), arity)) {
                    return Some(func);
                }
            }
            cur = scope.parent.as_deref();
        }
        None
    }

    pub fn set_func(&mut self, name: String, arity: usize, def: FuncDef) {
        Rc::make_mut(&mut self.scope)
            .functions
            .insert((name, arity), def);
    }

    pub fn child(&self) -> Self {
        Env {
            scope: Rc::new(Scope {
                parent: Some(Rc::clone(&self.scope)),
                variables: HashMap::new(),
                functions: HashMap::new(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_creates_empty_env() {
        let env = Env::default();
        assert_eq!(env.get_var("x"), None);
        assert!(env.get_func("f", 0).is_none());

        let env_new = Env::new();
        assert_eq!(env_new.get_var("x"), None);
        assert!(env_new.get_func("f", 1).is_none());
    }

    #[test]
    fn parent_lookup_for_vars_and_funcs() {
        let mut parent = Env::new();
        parent.set_var("x".to_string(), json!(42));
        parent.set_func(
            "inc".to_string(),
            1,
            FuncDef::User {
                params: vec!["n".to_string()],
                body: Expr::Identity,
            },
        );

        let child = parent.child();
        let grandchild = child.child();

        assert_eq!(child.get_var("x"), Some(&json!(42)));
        assert_eq!(grandchild.get_var("x"), Some(&json!(42)));
        assert!(child.get_func("inc", 1).is_some());
        assert!(grandchild.get_func("inc", 1).is_some());
        assert!(grandchild.get_func("inc", 0).is_none());
    }

    #[test]
    fn shadowing_does_not_mutate_parent() {
        let mut parent = Env::new();
        parent.set_var("x".to_string(), json!(1));
        parent.set_var("y".to_string(), json!(2));
        parent.set_func(
            "f".to_string(),
            0,
            FuncDef::User {
                params: vec![],
                body: Expr::Identity,
            },
        );

        let mut child = parent.child();
        child.set_var("x".to_string(), json!(99));
        child.set_func(
            "f".to_string(),
            0,
            FuncDef::User {
                params: vec!["shadow".to_string()],
                body: Expr::Identity,
            },
        );

        assert_eq!(child.get_var("x"), Some(&json!(99)));
        assert_eq!(child.get_var("y"), Some(&json!(2)));
        assert_eq!(parent.get_var("x"), Some(&json!(1)));

        match child.get_func("f", 0) {
            Some(FuncDef::User { params, .. }) => assert_eq!(params, &vec!["shadow".to_string()]),
            other => panic!("unexpected child func: {other:?}"),
        }
        match parent.get_func("f", 0) {
            Some(FuncDef::User { params, .. }) => assert!(params.is_empty()),
            other => panic!("unexpected parent func: {other:?}"),
        }
    }

    #[test]
    fn parent_mutation_after_child_is_isolated() {
        let mut parent = Env::new();
        parent.set_var("x".to_string(), json!("before"));

        let child = parent.child();
        parent.set_var("x".to_string(), json!("after"));

        assert_eq!(parent.get_var("x"), Some(&json!("after")));
        assert_eq!(child.get_var("x"), Some(&json!("before")));
    }

    #[test]
    fn cloned_env_is_isolated_on_mutation() {
        let mut env1 = Env::new();
        env1.set_var("a".to_string(), json!(10));
        let mut env2 = env1.clone();
        env2.set_var("a".to_string(), json!(20));
        env2.set_var("b".to_string(), json!(30));

        assert_eq!(env1.get_var("a"), Some(&json!(10)));
        assert_eq!(env1.get_var("b"), None);
        assert_eq!(env2.get_var("a"), Some(&json!(20)));
        assert_eq!(env2.get_var("b"), Some(&json!(30)));
    }
}
