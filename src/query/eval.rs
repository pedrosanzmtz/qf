use serde_json::Value;

use crate::error::QfError;

use super::ast::*;
use super::builtins;
use super::env::{Env, FuncDef};

/// Evaluate a jq expression against an input value, producing zero or more outputs.
pub fn eval(expr: &Expr, input: &Value, env: &Env) -> Result<Vec<Value>, QfError> {
    match expr {
        Expr::Identity => Ok(vec![input.clone()]),

        Expr::RecurseAll => {
            let mut results = Vec::new();
            recurse_all(input, &mut results);
            Ok(results)
        }

        Expr::Field(name) => match input {
            Value::Object(map) => Ok(vec![map
                .get(name)
                .cloned()
                .unwrap_or(Value::Null)]),
            Value::Null => Ok(vec![Value::Null]),
            _ => Err(QfError::TypeError(format!(
                "cannot index {} with string \"{}\"",
                value_type(input),
                name
            ))),
        },

        Expr::OptionalField(name) => match input {
            Value::Object(map) => Ok(vec![map
                .get(name)
                .cloned()
                .unwrap_or(Value::Null)]),
            _ => Ok(vec![]),
        },

        Expr::Index(expr, idx_expr) => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in &vals {
                let indices = eval(idx_expr, input, env)?;
                for idx in &indices {
                    results.push(index_value(val, idx)?);
                }
            }
            Ok(results)
        }

        Expr::OptionalIndex(expr, idx_expr) => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in &vals {
                let indices = eval(idx_expr, input, env)?;
                for idx in &indices {
                    match index_value(val, idx) {
                        Ok(v) => results.push(v),
                        Err(_) => {}
                    }
                }
            }
            Ok(results)
        }

        Expr::Slice(expr, from, to) => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in &vals {
                let from_idx = match from {
                    Some(f) => {
                        let v = eval_one(f, input, env)?;
                        v.as_i64().unwrap_or(0) as isize
                    }
                    None => 0,
                };
                let to_idx = match to {
                    Some(t) => {
                        let v = eval_one(t, input, env)?;
                        Some(v.as_i64().unwrap_or(0) as isize)
                    }
                    None => None,
                };
                results.push(slice_value(val, from_idx, to_idx)?);
            }
            Ok(results)
        }

        Expr::Iterate(expr) => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in &vals {
                match val {
                    Value::Array(arr) => results.extend(arr.iter().cloned()),
                    Value::Object(map) => results.extend(map.values().cloned()),
                    Value::Null => {}
                    _ => {
                        return Err(QfError::TypeError(format!(
                            "cannot iterate over {}",
                            value_type(val)
                        )))
                    }
                }
            }
            Ok(results)
        }

        Expr::OptionalIterate(expr) => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in &vals {
                match val {
                    Value::Array(arr) => results.extend(arr.iter().cloned()),
                    Value::Object(map) => results.extend(map.values().cloned()),
                    _ => {}
                }
            }
            Ok(results)
        }

        Expr::Pipe(left, right) => {
            let left_results = eval(left, input, env)?;
            let mut results = Vec::new();
            for val in &left_results {
                results.extend(eval(right, val, env)?);
            }
            Ok(results)
        }

        Expr::Comma(left, right) => {
            let mut results = eval(left, input, env)?;
            results.extend(eval(right, input, env)?);
            Ok(results)
        }

        Expr::Literal(val) => Ok(vec![val.clone()]),

        Expr::StringLiteral(s) => Ok(vec![Value::String(s.clone())]),

        Expr::Neg(expr) => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in vals {
                match &val {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            results.push(Value::Number((-i).into()));
                        } else if let Some(f) = n.as_f64() {
                            results.push(json_f64(-f));
                        } else {
                            return Err(QfError::TypeError("cannot negate number".into()));
                        }
                    }
                    _ => {
                        return Err(QfError::TypeError(format!(
                            "cannot negate {}",
                            value_type(&val)
                        )))
                    }
                }
            }
            Ok(results)
        }

        Expr::BinOp(op, left, right) => {
            let left_vals = eval(left, input, env)?;
            let mut results = Vec::new();
            for lv in &left_vals {
                let right_vals = eval(right, input, env)?;
                for rv in &right_vals {
                    results.push(eval_binop(op, lv, rv)?);
                }
            }
            Ok(results)
        }

        Expr::Not(expr) => {
            let vals = eval(expr, input, env)?;
            Ok(vals
                .into_iter()
                .map(|v| Value::Bool(!is_truthy(&v)))
                .collect())
        }

        Expr::Alternative(left, right) => {
            let vals = eval(left, input, env)?;
            let non_null: Vec<_> = vals
                .into_iter()
                .filter(|v| !v.is_null() && v != &Value::Bool(false))
                .collect();
            if non_null.is_empty() {
                eval(right, input, env)
            } else {
                Ok(non_null)
            }
        }

        Expr::Try(expr, catch) => match eval(expr, input, env) {
            Ok(vals) => Ok(vals),
            Err(e) => {
                if let Some(catch_expr) = catch {
                    let err_val = Value::String(e.to_string());
                    eval(catch_expr, &err_val, env)
                } else {
                    Ok(vec![])
                }
            }
        },

        Expr::ArrayConstruct(inner) => {
            let vals = eval(inner, input, env)?;
            Ok(vec![Value::Array(vals)])
        }

        Expr::ObjectConstruct(entries) => {
            eval_object_construct(entries, input, env)
        }

        Expr::If {
            cond,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            let cond_vals = eval(cond, input, env)?;
            let mut results = Vec::new();
            for cv in &cond_vals {
                if is_truthy(cv) {
                    results.extend(eval(then_branch, input, env)?);
                } else {
                    let mut handled = false;
                    for (elif_cond, elif_body) in elif_branches {
                        let elif_vals = eval(elif_cond, input, env)?;
                        if elif_vals.iter().any(|v| is_truthy(v)) {
                            results.extend(eval(elif_body, input, env)?);
                            handled = true;
                            break;
                        }
                    }
                    if !handled {
                        if let Some(else_br) = else_branch {
                            results.extend(eval(else_br, input, env)?);
                        } else {
                            results.push(input.clone());
                        }
                    }
                }
            }
            Ok(results)
        }

        Expr::As {
            expr,
            pattern,
            body,
        } => {
            let vals = eval(expr, input, env)?;
            let mut results = Vec::new();
            for val in &vals {
                let mut child_env = env.child();
                bind_pattern(&mut child_env, pattern, val)?;
                results.extend(eval(body, input, &child_env)?);
            }
            Ok(results)
        }

        Expr::Reduce {
            expr,
            pattern,
            init,
            update,
        } => {
            let items = eval(expr, input, env)?;
            let mut acc = eval_one(init, input, env)?;
            for item in &items {
                let mut child_env = env.child();
                bind_pattern(&mut child_env, pattern, item)?;
                acc = eval_one(update, &acc, &child_env)?;
            }
            Ok(vec![acc])
        }

        Expr::Foreach {
            expr,
            pattern,
            init,
            update,
            extract,
        } => {
            let items = eval(expr, input, env)?;
            let mut acc = eval_one(init, input, env)?;
            let mut results = Vec::new();
            for item in &items {
                let mut child_env = env.child();
                bind_pattern(&mut child_env, pattern, item)?;
                acc = eval_one(update, &acc, &child_env)?;
                if let Some(ext) = extract {
                    results.extend(eval(ext, &acc, &child_env)?);
                } else {
                    results.push(acc.clone());
                }
            }
            Ok(results)
        }

        Expr::Label(name, body) => {
            match eval(body, input, env) {
                Ok(v) => Ok(v),
                Err(QfError::UserError(msg)) if msg.starts_with("__break__") => {
                    let break_name = msg.trim_start_matches("__break__");
                    if break_name == name {
                        Ok(vec![])
                    } else {
                        Err(QfError::UserError(msg))
                    }
                }
                Err(e) => Err(e),
            }
        }

        Expr::Break(name) => Err(QfError::UserError(format!("__break__{name}"))),

        Expr::FuncDef {
            name,
            params,
            body,
            rest,
        } => {
            let mut child_env = env.child();
            child_env.set_func(
                name.clone(),
                params.len(),
                FuncDef {
                    params: params.clone(),
                    body: (**body).clone(),
                },
            );
            eval(rest, input, &child_env)
        }

        Expr::FuncCall(name, args) => {
            // Check user-defined functions first
            if let Some(func) = env.get_func(name, args.len()) {
                let func = func.clone();
                let mut child_env = env.child();
                for (param, arg) in func.params.iter().zip(args.iter()) {
                    // In jq, function args are filters, not values.
                    // For simplicity, we evaluate the arg and bind the result.
                    // This handles the common case. Full jq would pass closures.
                    let val = eval_one(arg, input, env)?;
                    child_env.set_var(param.clone(), val);
                }
                return eval(&func.body, input, &child_env);
            }

            // Built-in functions
            builtins::call_builtin(name, args, input, env)
        }

        Expr::VarRef(name) => match env.get_var(name) {
            Some(val) => Ok(vec![val.clone()]),
            None => {
                // Special env vars
                if name == "ENV" {
                    let mut map = serde_json::Map::new();
                    for (k, v) in std::env::vars() {
                        map.insert(k, Value::String(v));
                    }
                    return Ok(vec![Value::Object(map)]);
                }
                if name == "__loc__" {
                    return Ok(vec![Value::Null]);
                }
                Err(QfError::UndefinedVariable(name.clone()))
            }
        },

        Expr::Assign(path_expr, val_expr) => {
            eval_assign(path_expr, val_expr, input, env, AssignMode::Set)
        }

        Expr::UpdateAssign(path_expr, update_expr) => {
            eval_assign(path_expr, update_expr, input, env, AssignMode::Update)
        }

        Expr::ArithAssign(op, path_expr, val_expr) => {
            eval_assign(
                path_expr,
                val_expr,
                input,
                env,
                AssignMode::ArithUpdate(op.clone()),
            )
        }

        Expr::AltAssign(path_expr, val_expr) => {
            eval_assign(path_expr, val_expr, input, env, AssignMode::Alt)
        }

        Expr::Format(name) => builtins::apply_format(name, input),

        Expr::Optional(expr) => match eval(expr, input, env) {
            Ok(v) => Ok(v),
            Err(_) => Ok(vec![]),
        },
    }
}

/// Evaluate an expression expecting exactly one output.
pub fn eval_one(expr: &Expr, input: &Value, env: &Env) -> Result<Value, QfError> {
    let mut vals = eval(expr, input, env)?;
    match vals.len() {
        0 => Ok(Value::Null),
        1 => Ok(vals.remove(0)),
        _ => Ok(vals.remove(0)),
    }
}

// ── Helpers ────────────────────────────────────────────────────

pub fn value_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub fn is_truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

fn json_f64(f: f64) -> Value {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn index_value(val: &Value, idx: &Value) -> Result<Value, QfError> {
    match (val, idx) {
        (Value::Array(arr), Value::Number(n)) => {
            let i = n.as_i64().unwrap_or(0);
            let i = if i < 0 {
                (arr.len() as i64 + i) as usize
            } else {
                i as usize
            };
            Ok(arr.get(i).cloned().unwrap_or(Value::Null))
        }
        (Value::Object(map), Value::String(key)) => {
            Ok(map.get(key).cloned().unwrap_or(Value::Null))
        }
        (Value::Null, _) => Ok(Value::Null),
        _ => Err(QfError::TypeError(format!(
            "cannot index {} with {}",
            value_type(val),
            value_type(idx)
        ))),
    }
}

fn slice_value(val: &Value, from: isize, to: Option<isize>) -> Result<Value, QfError> {
    match val {
        Value::Array(arr) => {
            let len = arr.len() as isize;
            let start = if from < 0 {
                (len + from).max(0) as usize
            } else {
                from.min(len) as usize
            };
            let end = match to {
                Some(t) => {
                    if t < 0 {
                        (len + t).max(0) as usize
                    } else {
                        t.min(len) as usize
                    }
                }
                None => len as usize,
            };
            if start >= end {
                Ok(Value::Array(vec![]))
            } else {
                Ok(Value::Array(arr[start..end].to_vec()))
            }
        }
        Value::String(s) => {
            let len = s.len() as isize;
            let start = if from < 0 {
                (len + from).max(0) as usize
            } else {
                from.min(len) as usize
            };
            let end = match to {
                Some(t) => {
                    if t < 0 {
                        (len + t).max(0) as usize
                    } else {
                        t.min(len) as usize
                    }
                }
                None => len as usize,
            };
            if start >= end {
                Ok(Value::String(String::new()))
            } else {
                Ok(Value::String(s[start..end].to_string()))
            }
        }
        _ => Err(QfError::TypeError(format!(
            "cannot slice {}",
            value_type(val)
        ))),
    }
}

fn eval_binop(op: &BinOp, left: &Value, right: &Value) -> Result<Value, QfError> {
    match op {
        BinOp::Add => add_values(left, right),
        BinOp::Sub => arith_op(left, right, |a, b| a - b),
        BinOp::Mul => mul_values(left, right),
        BinOp::Div => arith_op(left, right, |a, b| {
            if b == 0.0 {
                f64::NAN
            } else {
                a / b
            }
        }),
        BinOp::Mod => arith_op(left, right, |a, b| {
            if b == 0.0 {
                f64::NAN
            } else {
                a % b
            }
        }),
        BinOp::Eq => Ok(Value::Bool(values_equal(left, right))),
        BinOp::Ne => Ok(Value::Bool(!values_equal(left, right))),
        BinOp::Lt => Ok(Value::Bool(compare_values(left, right) == std::cmp::Ordering::Less)),
        BinOp::Le => Ok(Value::Bool(
            matches!(
                compare_values(left, right),
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal
            )
        )),
        BinOp::Gt => Ok(Value::Bool(
            compare_values(left, right) == std::cmp::Ordering::Greater,
        )),
        BinOp::Ge => Ok(Value::Bool(
            matches!(
                compare_values(left, right),
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
            )
        )),
        BinOp::And => Ok(Value::Bool(is_truthy(left) && is_truthy(right))),
        BinOp::Or => Ok(Value::Bool(is_truthy(left) || is_truthy(right))),
    }
}

fn add_values(left: &Value, right: &Value) -> Result<Value, QfError> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            let af = a.as_f64().unwrap_or(0.0);
            let bf = b.as_f64().unwrap_or(0.0);
            let sum = af + bf;
            if a.is_i64() && b.is_i64() {
                if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
                    if let Some(r) = ai.checked_add(bi) {
                        return Ok(Value::Number(r.into()));
                    }
                }
            }
            Ok(json_f64(sum))
        }
        (Value::String(a), Value::String(b)) => {
            Ok(Value::String(format!("{a}{b}")))
        }
        (Value::Array(a), Value::Array(b)) => {
            let mut result = a.clone();
            result.extend(b.iter().cloned());
            Ok(Value::Array(result))
        }
        (Value::Object(a), Value::Object(b)) => {
            let mut result = a.clone();
            for (k, v) in b {
                result.insert(k.clone(), v.clone());
            }
            Ok(Value::Object(result))
        }
        (Value::Null, x) | (x, Value::Null) => Ok(x.clone()),
        _ => Err(QfError::TypeError(format!(
            "cannot add {} and {}",
            value_type(left),
            value_type(right)
        ))),
    }
}

fn mul_values(left: &Value, right: &Value) -> Result<Value, QfError> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            if a.is_i64() && b.is_i64() {
                if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
                    if let Some(r) = ai.checked_mul(bi) {
                        return Ok(Value::Number(r.into()));
                    }
                }
            }
            let af = a.as_f64().unwrap_or(0.0);
            let bf = b.as_f64().unwrap_or(0.0);
            Ok(json_f64(af * bf))
        }
        // String * number = repeat
        (Value::String(s), Value::Number(n)) | (Value::Number(n), Value::String(s)) => {
            let count = n.as_i64().unwrap_or(0).max(0) as usize;
            Ok(Value::String(s.repeat(count)))
        }
        // Object * Object = recursive merge
        (Value::Object(a), Value::Object(b)) => {
            let mut result = a.clone();
            for (k, v) in b {
                if let Some(existing) = result.get(k) {
                    if existing.is_object() && v.is_object() {
                        result.insert(k.clone(), mul_values(existing, v)?);
                    } else {
                        result.insert(k.clone(), v.clone());
                    }
                } else {
                    result.insert(k.clone(), v.clone());
                }
            }
            Ok(Value::Object(result))
        }
        (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
        _ => Err(QfError::TypeError(format!(
            "cannot multiply {} and {}",
            value_type(left),
            value_type(right)
        ))),
    }
}

fn arith_op(
    left: &Value,
    right: &Value,
    f: impl Fn(f64, f64) -> f64,
) -> Result<Value, QfError> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            let af = a.as_f64().unwrap_or(0.0);
            let bf = b.as_f64().unwrap_or(0.0);
            let result = f(af, bf);
            // Keep integer if both were integers and result fits
            if a.is_i64() && b.is_i64() && result.fract() == 0.0 {
                if result >= i64::MIN as f64 && result <= i64::MAX as f64 {
                    return Ok(Value::Number((result as i64).into()));
                }
            }
            Ok(json_f64(result))
        }
        _ => Err(QfError::TypeError(format!(
            "cannot perform arithmetic on {} and {}",
            value_type(left),
            value_type(right)
        ))),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    a == b
}

pub fn compare_values_pub(a: &Value, b: &Value) -> std::cmp::Ordering {
    compare_values(a, b)
}

pub fn set_path_pub(val: &Value, path: &[PathSegment], new_val: Value) -> Result<Value, QfError> {
    set_path(val, path, new_val)
}

pub fn collect_paths_pub(
    expr: &Expr,
    input: &Value,
    env: &Env,
) -> Result<Vec<Vec<PathSegment>>, QfError> {
    collect_paths(expr, input, env)
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    // jq comparison order: null < false < true < number < string < array < object
    fn type_order(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Bool(false) => 1,
            Value::Bool(true) => 2,
            Value::Number(_) => 3,
            Value::String(_) => 4,
            Value::Array(_) => 5,
            Value::Object(_) => 6,
        }
    }

    let ta = type_order(a);
    let tb = type_order(b);
    if ta != tb {
        return ta.cmp(&tb);
    }

    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => {
            let af = a.as_f64().unwrap_or(0.0);
            let bf = b.as_f64().unwrap_or(0.0);
            af.partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Array(a), Value::Array(b)) => {
            for (x, y) in a.iter().zip(b.iter()) {
                let c = compare_values(x, y);
                if c != std::cmp::Ordering::Equal {
                    return c;
                }
            }
            a.len().cmp(&b.len())
        }
        (Value::Object(_), Value::Object(_)) => std::cmp::Ordering::Equal,
        _ => std::cmp::Ordering::Equal,
    }
}

fn recurse_all(val: &Value, results: &mut Vec<Value>) {
    results.push(val.clone());
    match val {
        Value::Array(arr) => {
            for item in arr {
                recurse_all(item, results);
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                recurse_all(v, results);
            }
        }
        _ => {}
    }
}

fn eval_object_construct(
    entries: &[ObjectEntry],
    input: &Value,
    env: &Env,
) -> Result<Vec<Value>, QfError> {
    // Start with a single empty object, then for each entry expand
    let mut current = vec![serde_json::Map::new()];

    for entry in entries {
        let mut next = Vec::new();
        for obj in &current {
            match entry {
                ObjectEntry::KeyValue(key, val_expr) => {
                    let key_str = match key {
                        ObjectKey::Ident(s) | ObjectKey::String(s) => s.clone(),
                        ObjectKey::Format(name) => {
                            let vals = builtins::apply_format(name, input)?;
                            vals.into_iter()
                                .next()
                                .and_then(|v| v.as_str().map(String::from))
                                .unwrap_or_default()
                        }
                    };
                    let vals = eval(val_expr, input, env)?;
                    for v in &vals {
                        let mut new_obj = obj.clone();
                        new_obj.insert(key_str.clone(), v.clone());
                        next.push(new_obj);
                    }
                }
                ObjectEntry::ComputedKeyValue(key_expr, val_expr) => {
                    let keys = eval(key_expr, input, env)?;
                    for k in &keys {
                        let key_str = match k {
                            Value::String(s) => s.clone(),
                            other => {
                                return Err(QfError::TypeError(format!(
                                    "object key must be string, got {}",
                                    value_type(other)
                                )))
                            }
                        };
                        let vals = eval(val_expr, input, env)?;
                        for v in &vals {
                            let mut new_obj = obj.clone();
                            new_obj.insert(key_str.clone(), v.clone());
                            next.push(new_obj);
                        }
                    }
                }
                ObjectEntry::Shorthand(name) => {
                    let val = match input {
                        Value::Object(map) => {
                            map.get(name).cloned().unwrap_or(Value::Null)
                        }
                        _ => Value::Null,
                    };
                    let mut new_obj = obj.clone();
                    new_obj.insert(name.clone(), val);
                    next.push(new_obj);
                }
                ObjectEntry::ShorthandVar(name) => {
                    let val = env
                        .get_var(name)
                        .cloned()
                        .unwrap_or(Value::Null);
                    let mut new_obj = obj.clone();
                    new_obj.insert(name.clone(), val);
                    next.push(new_obj);
                }
                ObjectEntry::ShorthandFormat(name) => {
                    let vals = builtins::apply_format(name, input)?;
                    for v in &vals {
                        let mut new_obj = obj.clone();
                        new_obj.insert(name.clone(), v.clone());
                        next.push(new_obj);
                    }
                }
            }
        }
        current = next;
    }

    Ok(current.into_iter().map(Value::Object).collect())
}

fn bind_pattern(env: &mut Env, pattern: &Pattern, value: &Value) -> Result<(), QfError> {
    match pattern {
        Pattern::Variable(name) => {
            env.set_var(name.clone(), value.clone());
            Ok(())
        }
        Pattern::Array(patterns) => match value {
            Value::Array(arr) => {
                for (i, pat) in patterns.iter().enumerate() {
                    let v = arr.get(i).cloned().unwrap_or(Value::Null);
                    bind_pattern(env, pat, &v)?;
                }
                Ok(())
            }
            _ => Err(QfError::TypeError(format!(
                "cannot destructure {} as array",
                value_type(value)
            ))),
        },
        Pattern::Object(fields) => match value {
            Value::Object(map) => {
                for (key, pat) in fields {
                    let v = map.get(key).cloned().unwrap_or(Value::Null);
                    bind_pattern(env, pat, &v)?;
                }
                Ok(())
            }
            _ => Err(QfError::TypeError(format!(
                "cannot destructure {} as object",
                value_type(value)
            ))),
        },
    }
}

// ── Assignment ─────────────────────────────────────────────────

#[derive(Clone)]
enum AssignMode {
    Set,
    Update,
    ArithUpdate(BinOp),
    Alt,
}

fn eval_assign(
    path_expr: &Expr,
    val_expr: &Expr,
    input: &Value,
    env: &Env,
    mode: AssignMode,
) -> Result<Vec<Value>, QfError> {
    // Get the paths that the path expression references
    let paths = collect_paths(path_expr, input, env)?;

    let mut result = input.clone();
    for path in &paths {
        match &mode {
            AssignMode::Set => {
                let new_val = eval_one(val_expr, input, env)?;
                result = set_path(&result, path, new_val)?;
            }
            AssignMode::Update => {
                let current = get_path(&result, path);
                let new_val = eval_one(val_expr, &current, env)?;
                result = set_path(&result, path, new_val)?;
            }
            AssignMode::ArithUpdate(op) => {
                let current = get_path(&result, path);
                let rhs = eval_one(val_expr, input, env)?;
                let new_val = eval_binop(op, &current, &rhs)?;
                result = set_path(&result, path, new_val)?;
            }
            AssignMode::Alt => {
                let current = get_path(&result, path);
                if current.is_null() || current == Value::Bool(false) {
                    let new_val = eval_one(val_expr, input, env)?;
                    result = set_path(&result, path, new_val)?;
                }
            }
        }
    }
    Ok(vec![result])
}

fn collect_paths(
    expr: &Expr,
    input: &Value,
    env: &Env,
) -> Result<Vec<Vec<PathSegment>>, QfError> {
    match expr {
        Expr::Identity => Ok(vec![vec![]]),
        Expr::Field(name) => Ok(vec![vec![PathSegment::Key(name.clone())]]),
        Expr::Pipe(left, right) => {
            let left_paths = collect_paths(left, input, env)?;
            let mut all_paths = Vec::new();
            for lp in &left_paths {
                let sub_val = get_path(input, lp);
                let right_paths = collect_paths(right, &sub_val, env)?;
                for rp in &right_paths {
                    let mut path = lp.clone();
                    path.extend(rp.iter().cloned());
                    all_paths.push(path);
                }
            }
            Ok(all_paths)
        }
        Expr::Index(base, idx_expr) => {
            let base_paths = collect_paths(base, input, env)?;
            let mut all = Vec::new();
            for bp in &base_paths {
                let sub_val = get_path(input, bp);
                let idx = eval_one(idx_expr, &sub_val, env)?;
                let seg = match &idx {
                    Value::Number(n) => PathSegment::Index(n.as_i64().unwrap_or(0)),
                    Value::String(s) => PathSegment::Key(s.clone()),
                    _ => continue,
                };
                let mut path = bp.clone();
                path.push(seg);
                all.push(path);
            }
            Ok(all)
        }
        Expr::Iterate(base) => {
            let base_paths = collect_paths(base, input, env)?;
            let mut all = Vec::new();
            for bp in &base_paths {
                let sub_val = get_path(input, bp);
                match &sub_val {
                    Value::Array(arr) => {
                        for i in 0..arr.len() {
                            let mut path = bp.clone();
                            path.push(PathSegment::Index(i as i64));
                            all.push(path);
                        }
                    }
                    Value::Object(map) => {
                        for k in map.keys() {
                            let mut path = bp.clone();
                            path.push(PathSegment::Key(k.clone()));
                            all.push(path);
                        }
                    }
                    _ => {}
                }
            }
            Ok(all)
        }
        _ => {
            // For complex expressions, fall back to a single identity path
            Ok(vec![vec![]])
        }
    }
}

#[derive(Debug, Clone)]
pub enum PathSegment {
    Key(String),
    Index(i64),
}

fn get_path(val: &Value, path: &[PathSegment]) -> Value {
    let mut current = val;
    for seg in path {
        match seg {
            PathSegment::Key(k) => {
                current = match current {
                    Value::Object(map) => map.get(k).unwrap_or(&Value::Null),
                    _ => return Value::Null,
                };
            }
            PathSegment::Index(i) => {
                current = match current {
                    Value::Array(arr) => {
                        let idx = if *i < 0 {
                            (arr.len() as i64 + i) as usize
                        } else {
                            *i as usize
                        };
                        arr.get(idx).unwrap_or(&Value::Null)
                    }
                    _ => return Value::Null,
                };
            }
        }
    }
    current.clone()
}

fn set_path(val: &Value, path: &[PathSegment], new_val: Value) -> Result<Value, QfError> {
    if path.is_empty() {
        return Ok(new_val);
    }

    let seg = &path[0];
    let rest = &path[1..];

    match seg {
        PathSegment::Key(k) => {
            let mut obj = match val {
                Value::Object(map) => map.clone(),
                Value::Null => serde_json::Map::new(),
                _ => return Err(QfError::TypeError("cannot set key on non-object".into())),
            };
            let sub = obj.get(k).cloned().unwrap_or(Value::Null);
            let updated = set_path(&sub, rest, new_val)?;
            obj.insert(k.clone(), updated);
            Ok(Value::Object(obj))
        }
        PathSegment::Index(i) => {
            let mut arr = match val {
                Value::Array(a) => a.clone(),
                Value::Null => Vec::new(),
                _ => return Err(QfError::TypeError("cannot set index on non-array".into())),
            };
            let idx = if *i < 0 {
                (arr.len() as i64 + i).max(0) as usize
            } else {
                *i as usize
            };
            while arr.len() <= idx {
                arr.push(Value::Null);
            }
            let sub = arr.get(idx).cloned().unwrap_or(Value::Null);
            let updated = set_path(&sub, rest, new_val)?;
            arr[idx] = updated;
            Ok(Value::Array(arr))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::lexer::Lexer;
    use crate::query::jq_parser::Parser;
    use serde_json::json;

    fn eval_expr(input_str: &str, query: &str) -> Vec<Value> {
        let input: Value = serde_json::from_str(input_str).unwrap();
        let mut lexer = Lexer::new(query);
        lexer.tokenize().unwrap();
        let mut parser = Parser::new(lexer.tokens);
        let expr = parser.parse().unwrap();
        let env = Env::new();
        eval(&expr, &input, &env).unwrap()
    }

    #[test]
    fn eval_identity() {
        assert_eq!(eval_expr(r#"{"a":1}"#, "."), vec![json!({"a": 1})]);
    }

    #[test]
    fn eval_field() {
        assert_eq!(eval_expr(r#"{"a":1}"#, ".a"), vec![json!(1)]);
    }

    #[test]
    fn eval_nested_field() {
        assert_eq!(
            eval_expr(r#"{"a":{"b":2}}"#, ".a.b"),
            vec![json!(2)]
        );
    }

    #[test]
    fn eval_index() {
        assert_eq!(eval_expr(r#"[10,20,30]"#, ".[1]"), vec![json!(20)]);
    }

    #[test]
    fn eval_iterate() {
        assert_eq!(
            eval_expr(r#"[1,2,3]"#, ".[]"),
            vec![json!(1), json!(2), json!(3)]
        );
    }

    #[test]
    fn eval_pipe() {
        assert_eq!(
            eval_expr(r#"{"a":{"b":3}}"#, ".a | .b"),
            vec![json!(3)]
        );
    }

    #[test]
    fn eval_comma() {
        assert_eq!(
            eval_expr(r#"{"a":1,"b":2}"#, ".a, .b"),
            vec![json!(1), json!(2)]
        );
    }

    #[test]
    fn eval_addition() {
        assert_eq!(eval_expr("null", "1 + 2"), vec![json!(3)]);
    }

    #[test]
    fn eval_string_concat() {
        assert_eq!(
            eval_expr(r#"{"a":"hello","b":" world"}"#, ".a + .b"),
            vec![json!("hello world")]
        );
    }

    #[test]
    fn eval_comparison() {
        assert_eq!(eval_expr("null", "1 < 2"), vec![json!(true)]);
        assert_eq!(eval_expr("null", "2 == 2"), vec![json!(true)]);
        assert_eq!(eval_expr("null", "3 != 3"), vec![json!(false)]);
    }

    #[test]
    fn eval_array_construct() {
        assert_eq!(
            eval_expr(r#"{"a":1,"b":2}"#, "[.a, .b]"),
            vec![json!([1, 2])]
        );
    }

    #[test]
    fn eval_object_construct() {
        let result = eval_expr(r#"{"x":1,"y":2}"#, r#"{a: .x, b: .y}"#);
        assert_eq!(result, vec![json!({"a": 1, "b": 2})]);
    }

    #[test]
    fn eval_if_then_else() {
        assert_eq!(
            eval_expr("null", "if true then 1 else 2 end"),
            vec![json!(1)]
        );
        assert_eq!(
            eval_expr("null", "if false then 1 else 2 end"),
            vec![json!(2)]
        );
    }

    #[test]
    fn eval_select() {
        assert_eq!(
            eval_expr(r#"[1,2,3,4,5]"#, "[.[] | select(. > 3)]"),
            vec![json!([4, 5])]
        );
    }

    #[test]
    fn eval_reduce() {
        assert_eq!(
            eval_expr(r#"[1,2,3,4,5]"#, "reduce .[] as $x (0; . + $x)"),
            vec![json!(15)]
        );
    }

    #[test]
    fn eval_alternative() {
        assert_eq!(
            eval_expr(r#"{"a": null}"#, ".a // 42"),
            vec![json!(42)]
        );
        assert_eq!(
            eval_expr(r#"{"a": 1}"#, ".a // 42"),
            vec![json!(1)]
        );
    }

    #[test]
    fn eval_missing_key_returns_null() {
        assert_eq!(
            eval_expr(r#"{"a": 1}"#, ".missing"),
            vec![json!(null)]
        );
    }

    #[test]
    fn eval_object_merge() {
        assert_eq!(
            eval_expr("null", r#"{"a":1} * {"b":2}"#),
            vec![json!({"a": 1, "b": 2})]
        );
    }

    #[test]
    fn eval_update_assign() {
        assert_eq!(
            eval_expr(r#"{"a":1}"#, ".a |= . + 1"),
            vec![json!({"a": 2})]
        );
    }

    #[test]
    fn eval_negative_index() {
        assert_eq!(
            eval_expr(r#"[1,2,3]"#, ".[-1]"),
            vec![json!(3)]
        );
    }

    #[test]
    fn eval_slice() {
        assert_eq!(
            eval_expr(r#"[1,2,3,4,5]"#, ".[2:4]"),
            vec![json!([3, 4])]
        );
    }

    #[test]
    fn eval_try() {
        assert_eq!(
            eval_expr(r#""hello""#, "try .foo catch \"err\""),
            vec![json!("err")]
        );
    }

    #[test]
    fn eval_as_variable() {
        assert_eq!(
            eval_expr(r#"{"a":1}"#, ".a as $x | $x + $x"),
            vec![json!(2)]
        );
    }

    #[test]
    fn eval_funcdef() {
        assert_eq!(
            eval_expr("null", "def double: . * 2; 5 | double"),
            vec![json!(10)]
        );
    }

    #[test]
    fn eval_logical() {
        assert_eq!(eval_expr("null", "true and false"), vec![json!(false)]);
        assert_eq!(eval_expr("null", "true or false"), vec![json!(true)]);
    }

    #[test]
    fn eval_iterate_with_pipe() {
        assert_eq!(
            eval_expr(
                r#"[{"name":"a"},{"name":"b"}]"#,
                "[.[] | .name]"
            ),
            vec![json!(["a", "b"])]
        );
    }
}
