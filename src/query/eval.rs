use serde_json::Value;

use crate::error::QfError;

use super::ast::*;
use super::builtins;
use super::env::{Env, FuncDef};

#[derive(Debug)]
pub(crate) enum EvalError {
    Qf(QfError),
    Break(String),
    UserValue(Value),
}

impl From<QfError> for EvalError {
    fn from(e: QfError) -> Self {
        EvalError::Qf(e)
    }
}

impl EvalError {
    pub(crate) fn into_qf_error(self) -> QfError {
        match self {
            EvalError::Qf(e) => e,
            EvalError::Break(name) => {
                QfError::Runtime(format!("break ${name} without matching label"))
            }
            EvalError::UserValue(v) => match v {
                Value::String(s) => QfError::UserError(s),
                other => QfError::UserError(other.to_string()),
            },
        }
    }

    pub(crate) fn to_catch_value(&self) -> Option<Value> {
        match self {
            EvalError::Break(_) => None,
            EvalError::UserValue(v) => Some(v.clone()),
            EvalError::Qf(e) => {
                let msg = match e {
                    QfError::TypeError(s)
                    | QfError::Runtime(s)
                    | QfError::UserError(s)
                    | QfError::Parse(s)
                    | QfError::Serialize(s) => s.clone(),
                    other => other.to_string(),
                };
                Some(Value::String(msg))
            }
        }
    }
}

/// Evaluate a jq expression against an input value, producing zero or more outputs.
pub fn eval(expr: &Expr, input: &Value, env: &Env) -> Result<Vec<Value>, QfError> {
    eval_inner(expr, input, env).map_err(EvalError::into_qf_error)
}

pub(crate) fn eval_inner(expr: &Expr, input: &Value, env: &Env) -> Result<Vec<Value>, EvalError> {
    let mut results = Vec::new();
    eval_stream(expr, input, env, &mut |v| {
        results.push(v);
        Ok(true)
    })?;
    Ok(results)
}

/// Evaluate a jq expression lazily, streaming outputs to `emit`.
/// Returns `Ok(false)` if `emit` requested early termination.
pub(crate) fn eval_stream(
    expr: &Expr,
    input: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    match expr {
        Expr::Identity => emit(input.clone()),

        Expr::RecurseAll => recurse_all_stream(input, emit),

        Expr::Field(name) => match input {
            Value::Object(map) => emit(map.get(name).cloned().unwrap_or(Value::Null)),
            Value::Null => emit(Value::Null),
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "cannot index {} with string \"{}\"",
                value_type(input),
                name
            )))),
        },

        Expr::OptionalField(name) => match input {
            Value::Object(map) => emit(map.get(name).cloned().unwrap_or(Value::Null)),
            Value::Null => emit(Value::Null),
            _ => Ok(true),
        },

        Expr::Index(expr, idx_expr) => eval_stream(expr, input, env, &mut |val| {
            eval_stream(idx_expr, input, env, &mut |idx| {
                emit(index_value(&val, &idx)?)
            })
        }),

        Expr::OptionalIndex(expr, idx_expr) => eval_stream(expr, input, env, &mut |val| {
            eval_stream(
                idx_expr,
                input,
                env,
                &mut |idx| match index_value(&val, &idx) {
                    Ok(v) => emit(v),
                    Err(_) => Ok(true),
                },
            )
        }),

        Expr::Slice(expr, from, to) => eval_stream(expr, input, env, &mut |val| {
            let from_vals = match from {
                Some(f) => eval_inner(f, input, env)?,
                None => vec![Value::Null],
            };
            for fv in &from_vals {
                let from_idx = match fv {
                    Value::Null => 0,
                    Value::Number(n) => n.as_i64().unwrap_or(0) as isize,
                    other => {
                        return Err(EvalError::Qf(QfError::TypeError(format!(
                            "slice start index must be number, got {}",
                            value_type(other)
                        ))));
                    }
                };
                let to_vals = match to {
                    Some(t) => eval_inner(t, input, env)?,
                    None => vec![Value::Null],
                };
                for tv in &to_vals {
                    let to_idx = match tv {
                        Value::Null => None,
                        Value::Number(n) => Some(n.as_i64().unwrap_or(0) as isize),
                        other => {
                            return Err(EvalError::Qf(QfError::TypeError(format!(
                                "slice end index must be number, got {}",
                                value_type(other)
                            ))));
                        }
                    };
                    if !emit(slice_value(&val, from_idx, to_idx)?)? {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }),

        Expr::Iterate(expr) => eval_stream(expr, input, env, &mut |val| match &val {
            Value::Array(arr) => {
                for item in arr {
                    if !emit(item.clone())? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::Object(map) => {
                for v in map.values() {
                    if !emit(v.clone())? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::Null => Ok(true),
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "cannot iterate over {}",
                value_type(&val)
            )))),
        }),

        Expr::OptionalIterate(expr) => eval_stream(expr, input, env, &mut |val| match &val {
            Value::Array(arr) => {
                for item in arr {
                    if !emit(item.clone())? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::Object(map) => {
                for v in map.values() {
                    if !emit(v.clone())? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(true),
        }),

        Expr::Pipe(left, right) => eval_stream(left, input, env, &mut |val| {
            eval_stream(right, &val, env, emit)
        }),

        Expr::Comma(left, right) => {
            if !eval_stream(left, input, env, emit)? {
                return Ok(false);
            }
            eval_stream(right, input, env, emit)
        }

        Expr::Literal(val) => emit(val.clone()),

        Expr::StringLiteral(s) => emit(Value::String(s.clone())),

        Expr::Neg(expr) => eval_stream(expr, input, env, &mut |val| {
            let negated = match &val {
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Value::Number((-i).into())
                    } else if let Some(f) = n.as_f64() {
                        json_f64(-f)
                    } else {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "cannot negate number".into(),
                        )));
                    }
                }
                _ => {
                    return Err(EvalError::Qf(QfError::TypeError(format!(
                        "cannot negate {}",
                        value_type(&val)
                    ))));
                }
            };
            emit(negated)
        }),

        Expr::BinOp(op, left, right) => match op {
            BinOp::And => eval_stream(left, input, env, &mut |lv| {
                if !is_truthy(&lv) {
                    emit(Value::Bool(false))
                } else {
                    eval_stream(right, input, env, &mut |rv| {
                        emit(Value::Bool(is_truthy(&rv)))
                    })
                }
            }),
            BinOp::Or => eval_stream(left, input, env, &mut |lv| {
                if is_truthy(&lv) {
                    emit(Value::Bool(true))
                } else {
                    eval_stream(right, input, env, &mut |rv| {
                        emit(Value::Bool(is_truthy(&rv)))
                    })
                }
            }),
            _ => eval_stream(left, input, env, &mut |lv| {
                eval_stream(right, input, env, &mut |rv| emit(eval_binop(op, &lv, &rv)?))
            }),
        },

        Expr::Not(expr) => {
            eval_stream(expr, input, env, &mut |v| emit(Value::Bool(!is_truthy(&v))))
        }

        Expr::Alternative(left, right) => {
            let mut any_truthy = false;
            match eval_stream(left, input, env, &mut |v| {
                if is_truthy(&v) {
                    any_truthy = true;
                    emit(v)
                } else {
                    Ok(true)
                }
            }) {
                Ok(false) => return Ok(false),
                Ok(true) => {}
                Err(e) if e.to_catch_value().is_some() => {
                    any_truthy = false;
                }
                Err(e) => return Err(e),
            }
            if !any_truthy {
                eval_stream(right, input, env, emit)
            } else {
                Ok(true)
            }
        }

        Expr::Try(expr, catch) => match eval_stream(expr, input, env, emit) {
            Ok(cont) => Ok(cont),
            Err(e) => match e.to_catch_value() {
                Some(err_val) => {
                    if let Some(catch_expr) = catch {
                        eval_stream(catch_expr, &err_val, env, emit)
                    } else {
                        Ok(true)
                    }
                }
                None => Err(e),
            },
        },

        Expr::ArrayConstruct(inner) => {
            let vals = eval_inner(inner, input, env)?;
            emit(Value::Array(vals))
        }

        Expr::ObjectConstruct(entries) => {
            let vals = eval_object_construct(entries, input, env)?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::If {
            cond,
            then_branch,
            elif_branches,
            else_branch,
        } => eval_if(
            cond,
            then_branch,
            elif_branches,
            else_branch,
            input,
            env,
            emit,
        ),

        Expr::As {
            expr,
            pattern,
            body,
        } => eval_stream(expr, input, env, &mut |val| {
            let mut child_env = env.child();
            bind_pattern(&mut child_env, pattern, &val)?;
            eval_stream(body, input, &child_env, emit)
        }),

        Expr::Reduce {
            expr,
            pattern,
            init,
            update,
        } => {
            let items = eval_inner(expr, input, env)?;
            let mut accs = eval_inner(init, input, env)?;
            for item in &items {
                let mut next_accs = Vec::new();
                for acc in &accs {
                    let mut child_env = env.child();
                    bind_pattern(&mut child_env, pattern, item)?;
                    next_accs.extend(eval_inner(update, acc, &child_env)?);
                }
                accs = next_accs;
            }
            for acc in accs {
                if !emit(acc)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::Foreach {
            expr,
            pattern,
            init,
            update,
            extract,
        } => {
            let items = eval_inner(expr, input, env)?;
            let mut accs = eval_inner(init, input, env)?;
            for item in &items {
                let mut next_accs = Vec::new();
                for acc in &accs {
                    let mut child_env = env.child();
                    bind_pattern(&mut child_env, pattern, item)?;
                    let updated = eval_inner(update, acc, &child_env)?;
                    for u in &updated {
                        if let Some(ext) = extract {
                            if !eval_stream(ext, u, &child_env, emit)? {
                                return Ok(false);
                            }
                        } else if !emit(u.clone())? {
                            return Ok(false);
                        }
                    }
                    next_accs.extend(updated);
                }
                accs = next_accs;
            }
            Ok(true)
        }

        Expr::Label(name, body) => match eval_stream(body, input, env, emit) {
            Ok(cont) => Ok(cont),
            Err(EvalError::Break(bname)) if bname == *name => Ok(true),
            Err(e) => Err(e),
        },

        Expr::Break(name) => Err(EvalError::Break(name.clone())),

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
                FuncDef::User {
                    params: params.clone(),
                    body: (**body).clone(),
                },
            );
            eval_stream(rest, input, &child_env, emit)
        }

        Expr::FuncCall(name, args) => {
            if let Some(func) = env.get_func(name, args.len()) {
                match func.clone() {
                    FuncDef::Closure {
                        expr,
                        env: closure_env,
                    } => {
                        return eval_stream(&expr, input, &closure_env, emit);
                    }
                    FuncDef::User { params, body } => {
                        let mut child_env = env.child();
                        for (param, arg) in params.iter().zip(args.iter()) {
                            child_env.set_func(
                                param.clone(),
                                0,
                                FuncDef::Closure {
                                    expr: arg.clone(),
                                    env: Box::new(env.clone()),
                                },
                            );
                        }
                        return eval_stream(&body, input, &child_env, emit);
                    }
                }
            }

            builtins::call_builtin_stream(name, args, input, env, emit)
        }

        Expr::VarRef(name) => match env.get_var(name) {
            Some(val) => emit(val.clone()),
            None => {
                if name == "ENV" {
                    let mut map = serde_json::Map::new();
                    for (k, v) in std::env::vars() {
                        map.insert(k, Value::String(v));
                    }
                    return emit(Value::Object(map));
                }
                if name == "__loc__" {
                    return emit(Value::Null);
                }
                Err(EvalError::Qf(QfError::UndefinedVariable(name.clone())))
            }
        },

        Expr::Assign(path_expr, val_expr) => {
            let vals = eval_assign(path_expr, val_expr, input, env, AssignMode::Set)?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::UpdateAssign(path_expr, update_expr) => {
            let vals = eval_assign(path_expr, update_expr, input, env, AssignMode::Update)?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::ArithAssign(op, path_expr, val_expr) => {
            let vals = eval_assign(
                path_expr,
                val_expr,
                input,
                env,
                AssignMode::ArithUpdate(op.clone()),
            )?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::AltAssign(path_expr, val_expr) => {
            let vals = eval_assign(path_expr, val_expr, input, env, AssignMode::Alt)?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::Format(name) => {
            let vals = builtins::apply_format(name, input)?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        Expr::Optional(expr) => match eval_stream(expr, input, env, emit) {
            Ok(cont) => Ok(cont),
            Err(e) if e.to_catch_value().is_some() => Ok(true),
            Err(e) => Err(e),
        },
    }
}

fn eval_if(
    cond: &Expr,
    then_branch: &Expr,
    elif_branches: &[(Expr, Expr)],
    else_branch: &Option<Box<Expr>>,
    input: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    eval_stream(cond, input, env, &mut |cv| {
        if is_truthy(&cv) {
            eval_stream(then_branch, input, env, emit)
        } else if let Some(((elif_cond, elif_body), rest_elifs)) = elif_branches.split_first() {
            eval_if(
                elif_cond,
                elif_body,
                rest_elifs,
                else_branch,
                input,
                env,
                emit,
            )
        } else if let Some(else_br) = else_branch {
            eval_stream(else_br, input, env, emit)
        } else {
            emit(input.clone())
        }
    })
}

/// Evaluate an expression expecting at most one output.
pub fn eval_one(expr: &Expr, input: &Value, env: &Env) -> Result<Value, QfError> {
    let mut vals = eval(expr, input, env)?;
    match vals.len() {
        0 => Ok(Value::Null),
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

pub(crate) fn json_f64(f: f64) -> Value {
    if f.fract() == 0.0 && f.is_finite() && f >= i64::MIN as f64 && f < -(i64::MIN as f64) {
        Value::Number((f as i64).into())
    } else {
        serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
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
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len() as isize;
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
                Ok(Value::String(chars[start..end].iter().collect()))
            }
        }
        Value::Null => Ok(Value::Null),
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
        BinOp::Div => {
            if let (Value::Number(_), Value::Number(b)) = (left, right) {
                if b.as_f64().unwrap_or(0.0) == 0.0 {
                    return Err(QfError::Runtime("cannot divide by zero".into()));
                }
            }
            arith_op(left, right, |a, b| a / b)
        }
        BinOp::Mod => {
            if let (Value::Number(_), Value::Number(b)) = (left, right) {
                if b.as_f64().unwrap_or(0.0) == 0.0 {
                    return Err(QfError::Runtime("cannot divide by zero".into()));
                }
            }
            arith_op(left, right, |a, b| a % b)
        }
        BinOp::Eq => Ok(Value::Bool(values_equal(left, right))),
        BinOp::Ne => Ok(Value::Bool(!values_equal(left, right))),
        BinOp::Lt => Ok(Value::Bool(
            compare_values(left, right) == std::cmp::Ordering::Less,
        )),
        BinOp::Le => Ok(Value::Bool(matches!(
            compare_values(left, right),
            std::cmp::Ordering::Less | std::cmp::Ordering::Equal
        ))),
        BinOp::Gt => Ok(Value::Bool(
            compare_values(left, right) == std::cmp::Ordering::Greater,
        )),
        BinOp::Ge => Ok(Value::Bool(matches!(
            compare_values(left, right),
            std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
        ))),
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
        (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
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

fn arith_op(left: &Value, right: &Value, f: impl Fn(f64, f64) -> f64) -> Result<Value, QfError> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            let af = a.as_f64().unwrap_or(0.0);
            let bf = b.as_f64().unwrap_or(0.0);
            Ok(json_f64(f(af, bf)))
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
    collect_paths(expr, input, env).map_err(EvalError::into_qf_error)
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
        (Value::Object(a), Value::Object(b)) => {
            let mut ka: Vec<&String> = a.keys().collect();
            let mut kb: Vec<&String> = b.keys().collect();
            ka.sort();
            kb.sort();
            match ka.cmp(&kb) {
                std::cmp::Ordering::Equal => {
                    for k in ka {
                        let c = compare_values(&a[k], &b[k]);
                        if c != std::cmp::Ordering::Equal {
                            return c;
                        }
                    }
                    std::cmp::Ordering::Equal
                }
                ord => ord,
            }
        }
        _ => std::cmp::Ordering::Equal,
    }
}

fn recurse_all_stream(
    val: &Value,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    if !emit(val.clone())? {
        return Ok(false);
    }
    match val {
        Value::Array(arr) => {
            for item in arr {
                if !recurse_all_stream(item, emit)? {
                    return Ok(false);
                }
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                if !recurse_all_stream(v, emit)? {
                    return Ok(false);
                }
            }
        }
        _ => {}
    }
    Ok(true)
}

fn eval_object_construct(
    entries: &[ObjectEntry],
    input: &Value,
    env: &Env,
) -> Result<Vec<Value>, EvalError> {
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
                    let vals = eval_inner(val_expr, input, env)?;
                    for v in &vals {
                        let mut new_obj = obj.clone();
                        new_obj.insert(key_str.clone(), v.clone());
                        next.push(new_obj);
                    }
                }
                ObjectEntry::ComputedKeyValue(key_expr, val_expr) => {
                    let keys = eval_inner(key_expr, input, env)?;
                    for k in &keys {
                        let key_str = match k {
                            Value::String(s) => s.clone(),
                            other => {
                                return Err(EvalError::Qf(QfError::TypeError(format!(
                                    "object key must be string, got {}",
                                    value_type(other)
                                ))))
                            }
                        };
                        let vals = eval_inner(val_expr, input, env)?;
                        for v in &vals {
                            let mut new_obj = obj.clone();
                            new_obj.insert(key_str.clone(), v.clone());
                            next.push(new_obj);
                        }
                    }
                }
                ObjectEntry::Shorthand(name) => {
                    let val = match input {
                        Value::Object(map) => map.get(name).cloned().unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    let mut new_obj = obj.clone();
                    new_obj.insert(name.clone(), val);
                    next.push(new_obj);
                }
                ObjectEntry::ShorthandVar(name) => {
                    let val = env.get_var(name).cloned().unwrap_or(Value::Null);
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
) -> Result<Vec<Value>, EvalError> {
    let paths = collect_paths(path_expr, input, env)?;

    match mode {
        AssignMode::Set => {
            let vals = eval_inner(val_expr, input, env)?;
            let mut results = Vec::new();
            for new_val in vals {
                let mut result = input.clone();
                for path in &paths {
                    result = set_path(&result, path, new_val.clone())?;
                }
                results.push(result);
            }
            Ok(results)
        }
        AssignMode::Update => {
            let mut states: Vec<(Value, Vec<Vec<PathSegment>>)> = vec![(input.clone(), Vec::new())];
            for path in &paths {
                let mut next_states = Vec::new();
                for (state, deletions) in states {
                    let current = get_path(&state, path);
                    let new_vals = eval_inner(val_expr, &current, env)?;
                    if new_vals.is_empty() {
                        let mut new_del = deletions;
                        new_del.push(path.clone());
                        next_states.push((state, new_del));
                    } else {
                        for nv in new_vals {
                            let updated = set_path(&state, path, nv)?;
                            next_states.push((updated, deletions.clone()));
                        }
                    }
                }
                states = next_states;
            }
            let mut results = Vec::new();
            for (mut val, mut deletions) in states {
                if !deletions.is_empty() {
                    sort_paths_for_deletion(&mut deletions);
                    for del_path in &deletions {
                        val = delete_path_segments(&val, del_path);
                    }
                }
                results.push(val);
            }
            Ok(results)
        }
        AssignMode::ArithUpdate(op) => {
            let rhs_vals = eval_inner(val_expr, input, env)?;
            let mut results = Vec::new();
            for rhs in rhs_vals {
                let mut result = input.clone();
                for path in &paths {
                    let current = get_path(&result, path);
                    let new_val = eval_binop(&op, &current, &rhs)?;
                    result = set_path(&result, path, new_val)?;
                }
                results.push(result);
            }
            Ok(results)
        }
        AssignMode::Alt => {
            let rhs_vals = eval_inner(val_expr, input, env)?;
            let mut results = Vec::new();
            for rhs in rhs_vals {
                let mut result = input.clone();
                for path in &paths {
                    let current = get_path(&result, path);
                    if !is_truthy(&current) {
                        result = set_path(&result, path, rhs.clone())?;
                    }
                }
                results.push(result);
            }
            Ok(results)
        }
    }
}

pub(crate) fn collect_paths(
    expr: &Expr,
    input: &Value,
    env: &Env,
) -> Result<Vec<Vec<PathSegment>>, EvalError> {
    match expr {
        Expr::Identity => Ok(vec![vec![]]),
        Expr::Literal(Value::Null) => Ok(vec![]),
        Expr::RecurseAll => {
            let mut all = Vec::new();
            collect_recurse_all_paths(input, &mut vec![], &mut all);
            Ok(all)
        }
        Expr::Field(name) => match input {
            Value::Object(_) | Value::Null => Ok(vec![vec![PathSegment::Key(name.clone())]]),
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "cannot index {} with string \"{}\"",
                value_type(input),
                name
            )))),
        },
        Expr::OptionalField(name) => match input {
            Value::Object(_) | Value::Null => Ok(vec![vec![PathSegment::Key(name.clone())]]),
            _ => Ok(vec![]),
        },
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
        Expr::Comma(left, right) => {
            let mut all_paths = collect_paths(left, input, env)?;
            all_paths.extend(collect_paths(right, input, env)?);
            Ok(all_paths)
        }
        Expr::Index(base, idx_expr) => {
            let base_paths = collect_paths(base, input, env)?;
            let indices = eval_inner(idx_expr, input, env)?;
            let mut all = Vec::new();
            for bp in &base_paths {
                let sub_val = get_path(input, bp);
                for idx in &indices {
                    let seg = match (&sub_val, idx) {
                        (Value::Array(_) | Value::Null, Value::Number(n)) => {
                            PathSegment::Index(n.as_i64().unwrap_or(0))
                        }
                        (Value::Object(_) | Value::Null, Value::String(s)) => {
                            PathSegment::Key(s.clone())
                        }
                        _ => {
                            return Err(EvalError::Qf(QfError::TypeError(format!(
                                "cannot index {} with {}",
                                value_type(&sub_val),
                                value_type(idx)
                            ))));
                        }
                    };
                    let mut path = bp.clone();
                    path.push(seg);
                    all.push(path);
                }
            }
            Ok(all)
        }
        Expr::OptionalIndex(base, idx_expr) => {
            let base_paths = match collect_paths(base, input, env) {
                Ok(p) => p,
                Err(EvalError::Break(b)) => return Err(EvalError::Break(b)),
                Err(_) => return Ok(vec![]),
            };
            let indices = match eval_inner(idx_expr, input, env) {
                Ok(i) => i,
                Err(EvalError::Break(b)) => return Err(EvalError::Break(b)),
                Err(_) => return Ok(vec![]),
            };
            let mut all = Vec::new();
            for bp in &base_paths {
                let sub_val = get_path(input, bp);
                for idx in &indices {
                    let seg = match (&sub_val, idx) {
                        (Value::Array(_) | Value::Null, Value::Number(n)) => {
                            PathSegment::Index(n.as_i64().unwrap_or(0))
                        }
                        (Value::Object(_) | Value::Null, Value::String(s)) => {
                            PathSegment::Key(s.clone())
                        }
                        _ => continue,
                    };
                    let mut path = bp.clone();
                    path.push(seg);
                    all.push(path);
                }
            }
            Ok(all)
        }
        Expr::Slice(base, from, to) => {
            let base_paths = collect_paths(base, input, env)?;
            let from_vals = match from {
                Some(f) => eval_inner(f, input, env)?,
                None => vec![Value::Null],
            };
            let to_vals = match to {
                Some(t) => eval_inner(t, input, env)?,
                None => vec![Value::Null],
            };
            let mut all = Vec::new();
            for bp in &base_paths {
                let sub_val = get_path(input, bp);
                if !matches!(sub_val, Value::Array(_) | Value::String(_) | Value::Null) {
                    return Err(EvalError::Qf(QfError::TypeError(format!(
                        "cannot slice {}",
                        value_type(&sub_val)
                    ))));
                }
                for fv in &from_vals {
                    let start = match fv {
                        Value::Null => None,
                        Value::Number(n) => Some(n.as_i64().unwrap_or(0)),
                        other => {
                            return Err(EvalError::Qf(QfError::TypeError(format!(
                                "slice start index must be number, got {}",
                                value_type(other)
                            ))));
                        }
                    };
                    for tv in &to_vals {
                        let end = match tv {
                            Value::Null => None,
                            Value::Number(n) => Some(n.as_i64().unwrap_or(0)),
                            other => {
                                return Err(EvalError::Qf(QfError::TypeError(format!(
                                    "slice end index must be number, got {}",
                                    value_type(other)
                                ))));
                            }
                        };
                        let mut path = bp.clone();
                        path.push(PathSegment::Slice(start, end));
                        all.push(path);
                    }
                }
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
                    Value::Null => {}
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(format!(
                            "cannot iterate over {}",
                            value_type(&sub_val)
                        ))));
                    }
                }
            }
            Ok(all)
        }
        Expr::OptionalIterate(base) => {
            let base_paths = match collect_paths(base, input, env) {
                Ok(p) => p,
                Err(EvalError::Break(b)) => return Err(EvalError::Break(b)),
                Err(_) => return Ok(vec![]),
            };
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
        Expr::If {
            cond,
            then_branch,
            elif_branches,
            else_branch,
        } => collect_paths_if(cond, then_branch, elif_branches, else_branch, input, env),
        Expr::Alternative(left, right) => match eval_inner(left, input, env) {
            Ok(vals) if vals.iter().any(is_truthy) => collect_paths(left, input, env),
            Ok(_) => collect_paths(right, input, env),
            Err(EvalError::Break(b)) => Err(EvalError::Break(b)),
            Err(_) => collect_paths(right, input, env),
        },
        Expr::Try(inner, catch) => match collect_paths(inner, input, env) {
            Ok(paths) => Ok(paths),
            Err(e) => match e.to_catch_value() {
                Some(err_val) => {
                    if let Some(catch_expr) = catch {
                        collect_paths(catch_expr, &err_val, env)
                    } else {
                        Ok(vec![])
                    }
                }
                None => Err(e),
            },
        },
        Expr::Optional(inner) => match collect_paths(inner, input, env) {
            Ok(paths) => Ok(paths),
            Err(e) if e.to_catch_value().is_some() => Ok(vec![]),
            Err(e) => Err(e),
        },
        Expr::As {
            expr,
            pattern,
            body,
        } => {
            let vals = eval_inner(expr, input, env)?;
            let mut all = Vec::new();
            for val in &vals {
                let mut child_env = env.child();
                bind_pattern(&mut child_env, pattern, val)?;
                all.extend(collect_paths(body, input, &child_env)?);
            }
            Ok(all)
        }
        Expr::FuncCall(name, args) => {
            if let Some(func) = env.get_func(name, args.len()) {
                match func.clone() {
                    FuncDef::Closure {
                        expr,
                        env: closure_env,
                    } => return collect_paths(&expr, input, &closure_env),
                    FuncDef::User { params, body } => {
                        let mut child_env = env.child();
                        for (param, arg) in params.iter().zip(args.iter()) {
                            child_env.set_func(
                                param.clone(),
                                0,
                                FuncDef::Closure {
                                    expr: arg.clone(),
                                    env: Box::new(env.clone()),
                                },
                            );
                        }
                        return collect_paths(&body, input, &child_env);
                    }
                }
            }
            match (name.as_str(), args.len()) {
                ("select", 1) => {
                    let conds = eval_inner(&args[0], input, env)?;
                    let mut all = Vec::new();
                    for c in &conds {
                        if is_truthy(c) {
                            all.push(vec![]);
                        }
                    }
                    Ok(all)
                }
                ("empty", 0) | ("null", 0) => Ok(vec![]),
                ("getpath", 1) => {
                    let path_vals = eval_inner(&args[0], input, env)?;
                    let mut all = Vec::new();
                    for pv in path_vals {
                        match pv {
                            Value::Array(arr) => {
                                let mut segs = Vec::with_capacity(arr.len());
                                for seg in arr {
                                    match seg {
                                        Value::String(s) => segs.push(PathSegment::Key(s)),
                                        Value::Number(n) => {
                                            segs.push(PathSegment::Index(n.as_i64().unwrap_or(0)))
                                        }
                                        _ => {
                                            return Err(EvalError::Qf(QfError::Runtime(
                                                "Invalid path expression".into(),
                                            )));
                                        }
                                    }
                                }
                                all.push(segs);
                            }
                            _ => {
                                return Err(EvalError::Qf(QfError::TypeError(
                                    "getpath requires array".into(),
                                )));
                            }
                        }
                    }
                    Ok(all)
                }
                ("recurse", 0) => {
                    let mut all = Vec::new();
                    collect_recurse_all_paths(input, &mut vec![], &mut all);
                    Ok(all)
                }
                ("recurse", 1) => {
                    let mut all = Vec::new();
                    collect_recurse_fn_paths(&args[0], input, &[], env, &mut all)?;
                    Ok(all)
                }
                ("first", 1) => {
                    let paths = collect_paths(&args[0], input, env)?;
                    Ok(paths.into_iter().take(1).collect())
                }
                ("last", 1) => {
                    let paths = collect_paths(&args[0], input, env)?;
                    Ok(paths.into_iter().last().into_iter().collect())
                }
                ("limit", 2) => {
                    let counts = eval_inner(&args[0], input, env)?;
                    let mut all = Vec::new();
                    for c in counts {
                        let count = c.as_i64().unwrap_or(0).max(0) as usize;
                        let paths = collect_paths(&args[1], input, env)?;
                        all.extend(paths.into_iter().take(count));
                    }
                    Ok(all)
                }
                ("paths", 0) => {
                    let mut all = Vec::new();
                    collect_nonempty_paths(input, &mut vec![], &mut all);
                    Ok(all)
                }
                ("paths", 1) => {
                    let mut all = Vec::new();
                    collect_nonempty_paths_filtered(input, &mut vec![], &mut all, &args[0], env)?;
                    Ok(all)
                }
                ("leaf_paths", 0) => {
                    let mut all = Vec::new();
                    collect_leaf_path_segments(input, &mut vec![], &mut all);
                    Ok(all)
                }
                _ => Err(EvalError::Qf(QfError::Runtime(
                    "Invalid path expression".into(),
                ))),
            }
        }
        _ => Err(EvalError::Qf(QfError::Runtime(
            "Invalid path expression".into(),
        ))),
    }
}

fn collect_paths_if(
    cond: &Expr,
    then_branch: &Expr,
    elif_branches: &[(Expr, Expr)],
    else_branch: &Option<Box<Expr>>,
    input: &Value,
    env: &Env,
) -> Result<Vec<Vec<PathSegment>>, EvalError> {
    let cond_vals = eval_inner(cond, input, env)?;
    let mut all = Vec::new();
    for cv in &cond_vals {
        if is_truthy(cv) {
            all.extend(collect_paths(then_branch, input, env)?);
        } else if let Some(((elif_cond, elif_body), rest_elifs)) = elif_branches.split_first() {
            all.extend(collect_paths_if(
                elif_cond,
                elif_body,
                rest_elifs,
                else_branch,
                input,
                env,
            )?);
        } else if let Some(else_br) = else_branch {
            all.extend(collect_paths(else_br, input, env)?);
        } else {
            all.push(vec![]);
        }
    }
    Ok(all)
}

fn collect_recurse_all_paths(
    val: &Value,
    current: &mut Vec<PathSegment>,
    out: &mut Vec<Vec<PathSegment>>,
) {
    out.push(current.clone());
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(PathSegment::Index(i as i64));
                collect_recurse_all_paths(item, current, out);
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(PathSegment::Key(k.clone()));
                collect_recurse_all_paths(v, current, out);
                current.pop();
            }
        }
        _ => {}
    }
}

fn collect_recurse_fn_paths(
    f: &Expr,
    val: &Value,
    prefix: &[PathSegment],
    env: &Env,
    out: &mut Vec<Vec<PathSegment>>,
) -> Result<(), EvalError> {
    out.push(prefix.to_vec());
    let sub_paths = collect_paths(f, val, env)?;
    for sp in sub_paths {
        let next_val = get_path(val, &sp);
        let mut next_prefix = prefix.to_vec();
        next_prefix.extend(sp);
        collect_recurse_fn_paths(f, &next_val, &next_prefix, env, out)?;
    }
    Ok(())
}

fn collect_nonempty_paths(
    val: &Value,
    current: &mut Vec<PathSegment>,
    out: &mut Vec<Vec<PathSegment>>,
) {
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(PathSegment::Index(i as i64));
                out.push(current.clone());
                collect_nonempty_paths(item, current, out);
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(PathSegment::Key(k.clone()));
                out.push(current.clone());
                collect_nonempty_paths(v, current, out);
                current.pop();
            }
        }
        _ => {}
    }
}

fn collect_nonempty_paths_filtered(
    val: &Value,
    current: &mut Vec<PathSegment>,
    out: &mut Vec<Vec<PathSegment>>,
    filter: &Expr,
    env: &Env,
) -> Result<(), EvalError> {
    let filter_vals = eval_inner(filter, val, env)?;
    if filter_vals.iter().any(is_truthy) {
        out.push(current.clone());
    }
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(PathSegment::Index(i as i64));
                collect_nonempty_paths_filtered(item, current, out, filter, env)?;
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(PathSegment::Key(k.clone()));
                collect_nonempty_paths_filtered(v, current, out, filter, env)?;
                current.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_leaf_path_segments(
    val: &Value,
    current: &mut Vec<PathSegment>,
    out: &mut Vec<Vec<PathSegment>>,
) {
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(PathSegment::Index(i as i64));
                collect_leaf_path_segments(item, current, out);
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(PathSegment::Key(k.clone()));
                collect_leaf_path_segments(v, current, out);
                current.pop();
            }
        }
        _ => out.push(current.clone()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Key(String),
    Index(i64),
    Slice(Option<i64>, Option<i64>),
}

pub(crate) fn path_segments_to_value(path: Vec<PathSegment>) -> Value {
    Value::Array(
        path.into_iter()
            .map(|seg| match seg {
                PathSegment::Key(k) => Value::String(k),
                PathSegment::Index(i) => Value::Number(i.into()),
                PathSegment::Slice(start, end) => {
                    let mut m = serde_json::Map::new();
                    m.insert(
                        "start".into(),
                        start
                            .map(|s| Value::Number(s.into()))
                            .unwrap_or(Value::Null),
                    );
                    m.insert(
                        "end".into(),
                        end.map(|e| Value::Number(e.into())).unwrap_or(Value::Null),
                    );
                    Value::Object(m)
                }
            })
            .collect(),
    )
}

pub(crate) fn sort_paths_for_deletion(paths: &mut Vec<Vec<PathSegment>>) {
    paths.sort_by(|a, b| {
        for (sa, sb) in a.iter().zip(b.iter()) {
            match (sa, sb) {
                (PathSegment::Index(ia), PathSegment::Index(ib)) => {
                    let c = ib.cmp(ia);
                    if c != std::cmp::Ordering::Equal {
                        return c;
                    }
                }
                (PathSegment::Key(ka), PathSegment::Key(kb)) => {
                    let c = kb.cmp(ka);
                    if c != std::cmp::Ordering::Equal {
                        return c;
                    }
                }
                (PathSegment::Slice(sa_from, sa_to), PathSegment::Slice(sb_from, sb_to)) => {
                    let c = sb_from.cmp(sa_from).then_with(|| sb_to.cmp(sa_to));
                    if c != std::cmp::Ordering::Equal {
                        return c;
                    }
                }
                (PathSegment::Key(_), _) => return std::cmp::Ordering::Less,
                (_, PathSegment::Key(_)) => return std::cmp::Ordering::Greater,
                (PathSegment::Index(_), _) => return std::cmp::Ordering::Less,
                (_, PathSegment::Index(_)) => return std::cmp::Ordering::Greater,
            }
        }
        b.len().cmp(&a.len())
    });
    paths.dedup();
}

pub(crate) fn delete_path_segments(val: &Value, path: &[PathSegment]) -> Value {
    if path.is_empty() {
        return Value::Null;
    }
    let seg = &path[0];
    let rest = &path[1..];
    match (val, seg) {
        (Value::Object(map), PathSegment::Key(key)) => {
            if rest.is_empty() {
                let mut new_map = map.clone();
                new_map.remove(key);
                Value::Object(new_map)
            } else {
                let mut new_map = map.clone();
                if let Some(child) = map.get(key) {
                    new_map.insert(key.clone(), delete_path_segments(child, rest));
                }
                Value::Object(new_map)
            }
        }
        (Value::Array(arr), PathSegment::Index(i)) => {
            let idx = if *i < 0 {
                let resolved = arr.len() as i64 + i;
                if resolved < 0 {
                    return val.clone();
                }
                resolved as usize
            } else {
                *i as usize
            };
            if idx >= arr.len() {
                return val.clone();
            }
            if rest.is_empty() {
                let mut new_arr = arr.clone();
                new_arr.remove(idx);
                Value::Array(new_arr)
            } else {
                let mut new_arr = arr.clone();
                new_arr[idx] = delete_path_segments(&arr[idx], rest);
                Value::Array(new_arr)
            }
        }
        (Value::Array(arr), PathSegment::Slice(from, to)) => {
            let len = arr.len() as isize;
            let f = from.unwrap_or(0) as isize;
            let start = if f < 0 {
                (len + f).max(0) as usize
            } else {
                f.min(len) as usize
            };
            let end = match to {
                Some(t) => {
                    let t = *t as isize;
                    if t < 0 {
                        (len + t).max(0) as usize
                    } else {
                        t.min(len) as usize
                    }
                }
                None => len as usize,
            };
            if start >= end {
                return val.clone();
            }
            if rest.is_empty() {
                let mut new_arr = arr.clone();
                new_arr.drain(start..end);
                Value::Array(new_arr)
            } else {
                val.clone()
            }
        }
        _ => val.clone(),
    }
}

fn resolve_slice_bounds(len: usize, from: Option<i64>, to: Option<i64>) -> (usize, usize) {
    let l = len as isize;
    let f = from.unwrap_or(0) as isize;
    let start = if f < 0 {
        (l + f).max(0) as usize
    } else {
        f.min(l) as usize
    };
    let end = match to {
        Some(t) => {
            let t = t as isize;
            if t < 0 {
                (l + t).max(0) as usize
            } else {
                t.min(l) as usize
            }
        }
        None => len,
    };
    (start, end)
}

fn get_path(val: &Value, path: &[PathSegment]) -> Value {
    let mut current = val.clone();
    for seg in path {
        match seg {
            PathSegment::Key(k) => {
                current = match &current {
                    Value::Object(map) => map.get(k).cloned().unwrap_or(Value::Null),
                    _ => return Value::Null,
                };
            }
            PathSegment::Index(i) => {
                current = match &current {
                    Value::Array(arr) => {
                        let idx = if *i < 0 {
                            (arr.len() as i64 + i) as usize
                        } else {
                            *i as usize
                        };
                        arr.get(idx).cloned().unwrap_or(Value::Null)
                    }
                    _ => return Value::Null,
                };
            }
            PathSegment::Slice(from, to) => {
                current = match &current {
                    Value::Array(arr) => {
                        let (start, end) = resolve_slice_bounds(arr.len(), *from, *to);
                        if start >= end {
                            Value::Array(vec![])
                        } else {
                            Value::Array(arr[start..end].to_vec())
                        }
                    }
                    Value::String(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let (start, end) = resolve_slice_bounds(chars.len(), *from, *to);
                        if start >= end {
                            Value::String(String::new())
                        } else {
                            Value::String(chars[start..end].iter().collect())
                        }
                    }
                    _ => return Value::Null,
                };
            }
        }
    }
    current
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
        PathSegment::Slice(from, to) => {
            let mut arr = match val {
                Value::Array(a) => a.clone(),
                Value::Null => Vec::new(),
                _ => return Err(QfError::TypeError("cannot set slice on non-array".into())),
            };
            let (start, end) = resolve_slice_bounds(arr.len(), *from, *to);
            let replacement = match new_val {
                Value::Array(r) => r,
                _ => {
                    return Err(QfError::TypeError(
                        "cannot assign non-array to array slice".into(),
                    ))
                }
            };
            let end = end.max(start);
            arr.splice(start..end, replacement);
            Ok(Value::Array(arr))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::jq_parser::Parser;
    use crate::query::lexer::Lexer;
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
        assert_eq!(eval_expr(r#"{"a":{"b":2}}"#, ".a.b"), vec![json!(2)]);
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
        assert_eq!(eval_expr(r#"{"a":{"b":3}}"#, ".a | .b"), vec![json!(3)]);
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
        assert_eq!(eval_expr(r#"{"a": null}"#, ".a // 42"), vec![json!(42)]);
        assert_eq!(eval_expr(r#"{"a": 1}"#, ".a // 42"), vec![json!(1)]);
    }

    #[test]
    fn eval_missing_key_returns_null() {
        assert_eq!(eval_expr(r#"{"a": 1}"#, ".missing"), vec![json!(null)]);
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
        assert_eq!(eval_expr(r#"[1,2,3]"#, ".[-1]"), vec![json!(3)]);
    }

    #[test]
    fn eval_slice() {
        assert_eq!(eval_expr(r#"[1,2,3,4,5]"#, ".[2:4]"), vec![json!([3, 4])]);
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

    fn eval_expr_res(input_str: &str, query: &str) -> Result<Vec<Value>, QfError> {
        let input: Value = serde_json::from_str(input_str).unwrap();
        let mut lexer = Lexer::new(query);
        lexer.tokenize()?;
        let mut parser = Parser::new(lexer.tokens);
        let expr = parser.parse()?;
        let env = Env::default();
        eval(&expr, &input, &env)
    }

    #[test]
    fn eval_iterate_with_pipe() {
        assert_eq!(
            eval_expr(r#"[{"name":"a"},{"name":"b"}]"#, "[.[] | .name]"),
            vec![json!(["a", "b"])]
        );
    }

    // ── #8: UTF-8 string slicing ───────────────────────────────
    #[test]
    fn eval_utf8_slice() {
        assert_eq!(eval_expr(r#""héllo""#, ".[0:2]"), vec![json!("hé")]);
        assert_eq!(eval_expr(r#""héllo""#, ".[1:4]"), vec![json!("éll")]);
        assert_eq!(eval_expr(r#""héllo""#, ".[-2:]"), vec![json!("lo")]);
        assert_eq!(eval_expr(r#""🎉a🌍b""#, ".[1:3]"), vec![json!("a🌍")]);
        assert_eq!(eval_expr(r#""héllo""#, ".[1:null]"), vec![json!("éllo")]);
    }

    // ── #4: Path expressions ───────────────────────────────────
    #[test]
    fn eval_path_recurse_all() {
        assert_eq!(
            eval_expr(r#"{"a":[1,2]}"#, "[path(..)]"),
            vec![json!([[], ["a"], ["a", 0], ["a", 1]])]
        );
    }

    #[test]
    fn eval_del_select_reverse_indices() {
        assert_eq!(
            eval_expr(r#"[1,2,3,2,4]"#, "del(.[] | select(. == 2))"),
            vec![json!([1, 3, 4])]
        );
    }

    #[test]
    fn eval_update_assign_empty_deletes() {
        assert_eq!(
            eval_expr(r#"{"a":1,"b":2}"#, ".a |= empty"),
            vec![json!({"b": 2})]
        );
        assert_eq!(
            eval_expr(r#"[1,2,3]"#, ".[] |= if . == 2 then empty else . * 10 end"),
            vec![json!([10, 30])]
        );
    }

    #[test]
    fn eval_path_variants() {
        assert_eq!(
            eval_expr(r#"{"a":1,"b":2}"#, "[path(.a, .b)]"),
            vec![json!([["a"], ["b"]])]
        );
        assert_eq!(
            eval_expr(r#"[10,20,30,40]"#, "path(.[1:3])"),
            vec![json!([{"start": 1, "end": 3}])]
        );
        assert_eq!(
            eval_expr(r#"[0,1,2,3,4]"#, "del(.[1:3])"),
            vec![json!([0, 3, 4])]
        );
        assert_eq!(eval_expr("123", "[path(.a?)]"), vec![json!([])]);
        assert_eq!(
            eval_expr(
                r#"{"a":true,"b":1,"c":2}"#,
                "path(if .a then .b else .c end)"
            ),
            vec![json!(["b"])]
        );
        assert_eq!(
            eval_expr(r#"{"a":null,"b":2}"#, "path(.a // .b)"),
            vec![json!(["b"])]
        );
        assert_eq!(
            eval_expr("123", "[path(try .a catch empty)]"),
            vec![json!([])]
        );
        assert_eq!(
            eval_expr(r#"{"a":1}"#, r#"path("a" as $k | .[$k])"#),
            vec![json!(["a"])]
        );
        assert_eq!(
            eval_expr(r#"{"a":[10]}"#, r#"path(getpath(["a", 0]))"#),
            vec![json!(["a", 0])]
        );
        assert_eq!(eval_expr("null", "[path(null)]"), vec![json!([])]);
    }

    #[test]
    fn eval_invalid_path_expression_errors() {
        let err = eval_expr_res("null", "path(1 + 2)").unwrap_err();
        assert!(matches!(err, QfError::Runtime(ref s) if s == "Invalid path expression"));
    }

    // ── #5: Generator semantics ────────────────────────────────
    #[test]
    fn eval_generator_semantics() {
        assert_eq!(
            eval_expr("1", "[select(true, false, true)]"),
            vec![json!([1, 1])]
        );
        assert_eq!(eval_expr("1", "[select(empty)]"), vec![json!([])]);
        assert_eq!(
            eval_expr(
                "null",
                "[if false then 1 elif (true, false) then 2 else 3 end]"
            ),
            vec![json!([2, 3])]
        );
        assert_eq!(
            eval_expr("null", "[if false then 1 elif empty then 2 else 3 end]"),
            vec![json!([])]
        );
        assert_eq!(
            eval_expr("[1,2]", "[reduce .[] as $x (empty; . + $x)]"),
            vec![json!([])]
        );
        assert_eq!(
            eval_expr("[1,2]", "[reduce .[] as $x (0, 10; . + $x)]"),
            vec![json!([3, 13])]
        );
        assert_eq!(
            eval_expr("[1,2]", "[foreach .[] as $x (empty; . + $x)]"),
            vec![json!([])]
        );
        assert_eq!(
            eval_expr(r#"{"a":0}"#, "[.a = (1, 2)]"),
            vec![json!([{"a": 1}, {"a": 2}])]
        );
        assert_eq!(eval_expr(r#"{"a":0}"#, "[.a = empty]"), vec![json!([])]);
        assert_eq!(
            eval_expr(r#"{"a":1}"#, r#"[has("a", "b")]"#),
            vec![json!([true, false])]
        );
        assert_eq!(eval_expr(r#"{"a":1}"#, "[has(empty)]"), vec![json!([])]);
        assert_eq!(
            eval_expr(r#""a,b;c""#, r#"[split(",", ";")]"#),
            vec![json!([["a", "b;c"], ["a,b", "c"]])]
        );
        assert_eq!(eval_expr(r#""a,b""#, "[split(empty)]"), vec![json!([])]);
    }

    // ── #6: Function params as closures ────────────────────────
    #[test]
    fn eval_func_params_as_closures() {
        assert_eq!(
            eval_expr("[1,2,3]", "def f(g): [.[] | g]; f(. + 1)"),
            vec![json!([2, 3, 4])]
        );
        assert_eq!(
            eval_expr(
                "[1,2]",
                "def f(g): 10 as $x | [.[] | g]; 1 as $x | f(. + $x)"
            ),
            vec![json!([2, 3])]
        );
        assert_eq!(
            eval_expr(
                "[1,2,3]",
                "def mymap(f): if length == 0 then [empty] else [.[0] | f] + (.[1:] | mymap(f)) end; mymap(. * 2)"
            ),
            vec![json!([2, 4, 6])]
        );
    }

    // ── #7: Error & control-flow semantics ─────────────────────
    #[test]
    fn eval_try_catch_json_value_and_prefixes() {
        assert_eq!(
            eval_expr("null", r#"try error({"code": 42}) catch .code"#),
            vec![json!(42)]
        );
        assert_eq!(
            eval_expr("null", "try error([1, 2]) catch length"),
            vec![json!(2)]
        );
        let msg = eval_expr("1", "try .foo catch .");
        assert_eq!(msg, vec![json!("cannot index number with string \"foo\"")]);
    }

    #[test]
    fn eval_alternative_suppresses_lhs_error() {
        assert_eq!(eval_expr("null", r#"error("x") // 42"#), vec![json!(42)]);
    }

    #[test]
    fn eval_logical_short_circuit() {
        assert_eq!(
            eval_expr("null", r#"false and error("x")"#),
            vec![json!(false)]
        );
        assert_eq!(
            eval_expr("null", r#"true or error("x")"#),
            vec![json!(true)]
        );
    }

    #[test]
    fn eval_div_mod_by_zero_errors() {
        assert!(eval_expr_res("null", "1 / 0").is_err());
        assert!(eval_expr_res("null", "1 % 0").is_err());
    }

    #[test]
    fn eval_break_not_caught_by_try_or_optional() {
        let env = Env::default();
        let try_break = Expr::Label(
            "out".into(),
            Box::new(Expr::Try(
                Box::new(Expr::Break("out".into())),
                Some(Box::new(Expr::StringLiteral("caught".into()))),
            )),
        );
        assert_eq!(
            eval(&try_break, &Value::Null, &env).unwrap(),
            Vec::<Value>::new()
        );

        let opt_break = Expr::Label(
            "out".into(),
            Box::new(Expr::Optional(Box::new(Expr::Break("out".into())))),
        );
        assert_eq!(
            eval(&opt_break, &Value::Null, &env).unwrap(),
            Vec::<Value>::new()
        );

        let fake_break = Expr::Label(
            "out".into(),
            Box::new(Expr::Try(
                Box::new(Expr::FuncCall(
                    "error".into(),
                    vec![Expr::StringLiteral("__break__out".into())],
                )),
                Some(Box::new(Expr::Identity)),
            )),
        );
        assert_eq!(
            eval(&fake_break, &Value::Null, &env).unwrap(),
            vec![json!("__break__out")]
        );
    }

    // ── #19: Laziness & recurse(f) ─────────────────────────────
    #[test]
    fn eval_recurse_preserves_null_and_errors() {
        assert_eq!(
            eval_expr(
                r#"{"a":{"a":null}}"#,
                "[recurse(if . == null then empty else .a end)]"
            ),
            vec![json!([{"a": {"a": null}}, {"a": null}, null])]
        );
        assert!(eval_expr_res("1", r#"recurse(error("boom"))"#).is_err());
    }

    #[test]
    fn eval_lazy_short_circuit_generators() {
        assert_eq!(
            eval_expr("null", "first(range(1000000000))"),
            vec![json!(0)]
        );
        assert_eq!(
            eval_expr("null", "[limit(5; range(1000000000))]"),
            vec![json!([0, 1, 2, 3, 4])]
        );
        assert_eq!(
            eval_expr("1", "[limit(5; repeat(. + 1))]"),
            vec![json!([1, 2, 3, 4, 5])]
        );
        assert_eq!(eval_expr("1", "first(recurse(. + 1))"), vec![json!(1)]);
    }

    // ── #21 & #22: Object comparison & Number fidelity ─────────
    #[test]
    fn eval_object_comparison_and_sort() {
        assert_eq!(eval_expr("null", "{} < {\"a\": 1}"), vec![json!(true)]);
        assert_eq!(eval_expr("null", "{\"a\": 1} < {}"), vec![json!(false)]);
        assert_eq!(
            eval_expr("null", "{\"a\": 1} < {\"b\": 0}"),
            vec![json!(true)]
        );
        assert_eq!(
            eval_expr("null", "{\"a\": 1} < {\"a\": 2}"),
            vec![json!(true)]
        );
        assert_eq!(
            eval_expr("null", "{\"b\": 1, \"a\": 2} == {\"a\": 2, \"b\": 1}"),
            vec![json!(true)]
        );
        assert_eq!(
            eval_expr(r#"[{"b":1},{"a":2},{}]"#, "sort"),
            vec![json!([{}, {"a": 2}, {"b": 1}])]
        );
    }

    #[test]
    fn eval_integral_float_normalization() {
        let sum = eval_expr("null", "1.5 + 2.5");
        assert_eq!(sum, vec![json!(4)]);
        assert!(sum[0].is_i64());

        let big = eval_expr("null", "67499775000.0 + 0.0");
        assert_eq!(big, vec![json!(67499775000_i64)]);
        assert!(big[0].is_i64());

        let added = eval_expr("[1.5, 2.5]", "add");
        assert_eq!(added, vec![json!(4)]);
        assert!(added[0].is_i64());

        let prod = eval_expr("null", "2.5 * 4.0");
        assert_eq!(prod, vec![json!(10)]);
        assert!(prod[0].is_i64());

        let quot = eval_expr("null", "6.0 / 2.0");
        assert_eq!(quot, vec![json!(3)]);
        assert!(quot[0].is_i64());

        let diff = eval_expr("null", "10.0 - 3.0");
        assert_eq!(diff, vec![json!(7)]);
        assert!(diff[0].is_i64());

        let neg = eval_expr("-5.0", "-.");
        assert_eq!(neg, vec![json!(5)]);
        assert!(neg[0].is_i64());

        // Non-integral floats remain floats
        let frac = eval_expr("null", "1.5 + 2.25");
        assert_eq!(frac, vec![json!(3.75)]);
    }
}
