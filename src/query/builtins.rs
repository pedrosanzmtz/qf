use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use regex::Regex;
use serde_json::Value;

use crate::error::QfError;

use super::ast::Expr;
use super::env::Env;
use super::eval::{
    collect_paths, delete_path_segments, eval_inner, eval_stream, is_truthy, json_f64,
    path_segments_to_value, set_path_pub, sort_paths_for_deletion, value_type, EvalError,
    PathSegment,
};

pub fn call_builtin(
    name: &str,
    args: &[Expr],
    input: &Value,
    env: &Env,
) -> Result<Vec<Value>, QfError> {
    let mut results = Vec::new();
    call_builtin_stream(name, args, input, env, &mut |v| {
        results.push(v);
        Ok(true)
    })
    .map_err(EvalError::into_qf_error)?;
    Ok(results)
}

pub(crate) fn call_builtin_stream(
    name: &str,
    args: &[Expr],
    input: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    match (name, args.len()) {
        ("select", 1) => eval_stream(&args[0], input, env, &mut |cond| {
            if is_truthy(&cond) {
                emit(input.clone())
            } else {
                Ok(true)
            }
        }),
        ("range", 1) => eval_stream(&args[0], input, env, &mut |n| {
            let end = n.as_f64().unwrap_or(0.0) as i64;
            for i in 0..end {
                if !emit(Value::Number(i.into()))? {
                    return Ok(false);
                }
            }
            Ok(true)
        }),
        ("range", 2) => eval_stream(&args[0], input, env, &mut |s| {
            let start = s.as_f64().unwrap_or(0.0) as i64;
            eval_stream(&args[1], input, env, &mut |e| {
                let end = e.as_f64().unwrap_or(0.0) as i64;
                for i in start..end {
                    if !emit(Value::Number(i.into()))? {
                        return Ok(false);
                    }
                }
                Ok(true)
            })
        }),
        ("range", 3) => eval_stream(&args[0], input, env, &mut |s| {
            let start = s.as_f64().unwrap_or(0.0);
            eval_stream(&args[1], input, env, &mut |e| {
                let end = e.as_f64().unwrap_or(0.0);
                eval_stream(&args[2], input, env, &mut |st| {
                    let step = st.as_f64().unwrap_or(1.0);
                    if step == 0.0 {
                        return Err(EvalError::Qf(QfError::Runtime(
                            "range step cannot be 0".into(),
                        )));
                    }
                    let mut i = start;
                    if step > 0.0 {
                        while i < end {
                            if !emit(json_f64(i))? {
                                return Ok(false);
                            }
                            i += step;
                        }
                    } else {
                        while i > end {
                            if !emit(json_f64(i))? {
                                return Ok(false);
                            }
                            i += step;
                        }
                    }
                    Ok(true)
                })
            })
        }),
        ("first", 1) => {
            let mut outer_cont = true;
            eval_stream(&args[0], input, env, &mut |v| {
                outer_cont = emit(v)?;
                Ok(false)
            })?;
            Ok(outer_cont)
        }
        ("last", 1) => {
            let mut last = None;
            eval_stream(&args[0], input, env, &mut |v| {
                last = Some(v);
                Ok(true)
            })?;
            if let Some(v) = last {
                emit(v)
            } else {
                Ok(true)
            }
        }
        ("limit", 2) => eval_stream(&args[0], input, env, &mut |n_val| {
            let count = n_val.as_i64().unwrap_or(0);
            if count < 0 {
                return Err(EvalError::Qf(QfError::Runtime(
                    "limit: count must be non-negative".into(),
                )));
            }
            let mut remaining = count as usize;
            if remaining == 0 {
                return Ok(true);
            }
            let mut outer_cont = true;
            eval_stream(&args[1], input, env, &mut |v| {
                if remaining > 0 {
                    remaining -= 1;
                    outer_cont = emit(v)?;
                    Ok(outer_cont && remaining > 0)
                } else {
                    Ok(false)
                }
            })?;
            Ok(outer_cont)
        }),
        ("recurse", 0) => eval_stream(&Expr::RecurseAll, input, env, emit),
        ("recurse", 1) => eval_recurse_stream(&args[0], input, env, emit),
        ("while", 2) => eval_while_stream(&args[0], &args[1], input, env, emit),
        ("repeat", 1) => eval_recurse_stream(&args[0], input, env, emit),
        ("any", 2) => {
            let mut found = false;
            eval_stream(&args[0], input, env, &mut |item| {
                let mut inner_cont = true;
                eval_stream(&args[1], &item, env, &mut |cond| {
                    if is_truthy(&cond) {
                        found = true;
                        inner_cont = false;
                        Ok(false)
                    } else {
                        Ok(true)
                    }
                })?;
                Ok(inner_cont)
            })?;
            emit(Value::Bool(found))
        }
        ("all", 2) => {
            let mut all_true = true;
            eval_stream(&args[0], input, env, &mut |item| {
                let mut inner_cont = true;
                eval_stream(&args[1], &item, env, &mut |cond| {
                    if !is_truthy(&cond) {
                        all_true = false;
                        inner_cont = false;
                        Ok(false)
                    } else {
                        Ok(true)
                    }
                })?;
                Ok(inner_cont)
            })?;
            emit(Value::Bool(all_true))
        }
        ("isempty", 1) => {
            let mut is_empty = true;
            eval_stream(&args[0], input, env, &mut |_| {
                is_empty = false;
                Ok(false)
            })?;
            emit(Value::Bool(is_empty))
        }
        ("IN", 1) => {
            let mut found = false;
            eval_stream(&args[0], input, env, &mut |v| {
                if *input == v {
                    found = true;
                    Ok(false)
                } else {
                    Ok(true)
                }
            })?;
            emit(Value::Bool(found))
        }
        ("nth", 2) => eval_nth_stream(&args[0], &args[1], input, env, emit),
        ("splits", 1) | ("splits", 2) => eval_splits_stream(args, input, env, emit),
        ("walk", 1) => eval_walk_stream(&args[0], input, env, emit),
        ("tostream", 0) => {
            let mut path = Vec::new();
            emit_tostream(input, &mut path, emit)
        }
        ("fromstream", 1) => eval_fromstream(&args[0], input, env, emit),
        _ => {
            let vals = call_builtin_vec(name, args, input, env)?;
            for v in vals {
                if !emit(v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

fn eval_recurse_stream(
    f: &Expr,
    val: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    if !emit(val.clone())? {
        return Ok(false);
    }
    eval_stream(f, val, env, &mut |next| {
        eval_recurse_stream(f, &next, env, emit)
    })
}

fn eval_while_stream(
    cond: &Expr,
    update: &Expr,
    val: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    let cond_vals = eval_inner(cond, val, env)?;
    for cv in cond_vals {
        if is_truthy(&cv) {
            if !emit(val.clone())? {
                return Ok(false);
            }
            let next_vals = eval_inner(update, val, env)?;
            for nv in next_vals {
                if !eval_while_stream(cond, update, &nv, env, emit)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn eval_nth_stream(
    n_expr: &Expr,
    gen_expr: &Expr,
    input: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    eval_stream(n_expr, input, env, &mut |n_val| {
        let target = match n_val {
            Value::Number(n) => n
                .as_i64()
                .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
            _ => {
                return Err(EvalError::Qf(QfError::TypeError(
                    "nth index must be a number".into(),
                )))
            }
        };
        if target < 0 {
            return Err(EvalError::Qf(QfError::Runtime(
                "nth doesn't support negative indices".into(),
            )));
        }
        let mut idx = 0i64;
        let mut outer_cont = true;
        eval_stream(gen_expr, input, env, &mut |v| {
            if idx == target {
                outer_cont = emit(v)?;
                Ok(false)
            } else {
                idx += 1;
                Ok(true)
            }
        })?;
        Ok(outer_cont)
    })
}

fn eval_splits_stream(
    args: &[Expr],
    input: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    let s = match input {
        Value::String(s) => s.as_str(),
        _ => {
            return Err(EvalError::Qf(QfError::TypeError(
                "splits requires string input".into(),
            )))
        }
    };
    let patterns = eval_inner(&args[0], input, env)?;
    let flags_list = if args.len() > 1 {
        eval_inner(&args[1], input, env)?
            .into_iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for pattern in &patterns {
        let p = pattern.as_str().ok_or_else(|| {
            EvalError::Qf(QfError::TypeError("splits pattern must be string".into()))
        })?;
        for flags in &flags_list {
            let re = build_regex(p, flags)?;
            for part in re.split(s) {
                if !emit(Value::String(part.to_string()))? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn eval_walk_stream(
    f: &Expr,
    val: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    match val {
        Value::Array(arr) => {
            let mut new_arr = Vec::with_capacity(arr.len());
            for item in arr {
                eval_walk_stream(f, item, env, &mut |v| {
                    new_arr.push(v);
                    Ok(true)
                })?;
            }
            eval_stream(f, &Value::Array(new_arr), env, emit)
        }
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                let mut walked = None;
                eval_walk_stream(f, v, env, &mut |wv| {
                    walked = Some(wv);
                    Ok(false)
                })?;
                if let Some(wv) = walked {
                    new_map.insert(k.clone(), wv);
                }
            }
            eval_stream(f, &Value::Object(new_map), env, emit)
        }
        _ => eval_stream(f, val, env, emit),
    }
}

fn emit_tostream(
    val: &Value,
    path: &mut Vec<Value>,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    match val {
        Value::Array(arr) if !arr.is_empty() => {
            for (i, item) in arr.iter().enumerate() {
                path.push(Value::Number(i.into()));
                if !emit_tostream(item, path, emit)? {
                    path.pop();
                    return Ok(false);
                }
                path.pop();
            }
            path.push(Value::Number((arr.len() - 1).into()));
            let cont = emit(Value::Array(vec![Value::Array(path.clone())]))?;
            path.pop();
            Ok(cont)
        }
        Value::Object(map) if !map.is_empty() => {
            let mut last_key = None;
            for (k, v) in map {
                path.push(Value::String(k.clone()));
                if !emit_tostream(v, path, emit)? {
                    path.pop();
                    return Ok(false);
                }
                path.pop();
                last_key = Some(k);
            }
            if let Some(lk) = last_key {
                path.push(Value::String(lk.clone()));
                let cont = emit(Value::Array(vec![Value::Array(path.clone())]))?;
                path.pop();
                Ok(cont)
            } else {
                Ok(true)
            }
        }
        _ => emit(Value::Array(vec![Value::Array(path.clone()), val.clone()])),
    }
}

fn value_to_path_segments(path: &[Value]) -> Result<Vec<PathSegment>, EvalError> {
    path.iter()
        .map(|seg| match seg {
            Value::String(s) => Ok(PathSegment::Key(s.clone())),
            Value::Number(n) => {
                let i = n.as_i64().ok_or_else(|| {
                    EvalError::Qf(QfError::TypeError(
                        "stream path index must be integer".into(),
                    ))
                })?;
                Ok(PathSegment::Index(i))
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "stream path segment must be string or number".into(),
            ))),
        })
        .collect()
}

fn eval_fromstream(
    expr: &Expr,
    input: &Value,
    env: &Env,
    emit: &mut dyn FnMut(Value) -> Result<bool, EvalError>,
) -> Result<bool, EvalError> {
    let mut acc: Option<Value> = None;
    let mut outer_cont = true;
    eval_stream(expr, input, env, &mut |item| {
        let arr = match item {
            Value::Array(a) => a,
            _ => {
                return Err(EvalError::Qf(QfError::TypeError(
                    "fromstream requires [path] or [path, value] arrays".into(),
                )))
            }
        };
        match arr.len() {
            2 => {
                let path = match &arr[0] {
                    Value::Array(p) => p,
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "fromstream path must be an array".into(),
                        )))
                    }
                };
                let val = arr[1].clone();
                if path.is_empty() {
                    acc = None;
                    outer_cont = emit(val)?;
                    Ok(outer_cont)
                } else {
                    let segments = value_to_path_segments(path)?;
                    let base = acc.take().unwrap_or(Value::Null);
                    acc = Some(set_path_pub(&base, &segments, val)?);
                    Ok(true)
                }
            }
            1 => {
                let path = match &arr[0] {
                    Value::Array(p) => p,
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "fromstream path must be an array".into(),
                        )))
                    }
                };
                if path.len() <= 1 {
                    if let Some(completed) = acc.take() {
                        outer_cont = emit(completed)?;
                        return Ok(outer_cont);
                    }
                }
                Ok(true)
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "fromstream requires [path] or [path, value] arrays".into(),
            ))),
        }
    })?;
    Ok(outer_cont)
}

fn call_builtin_vec(
    name: &str,
    args: &[Expr],
    input: &Value,
    env: &Env,
) -> Result<Vec<Value>, EvalError> {
    match (name, args.len()) {
        // ── Type / info ────────────────────────────────────
        ("length", 0) => Ok(vec![length(input)?]),
        ("utf8bytelength", 0) => match input {
            Value::String(s) => Ok(vec![Value::Number(s.len().into())]),
            _ => Ok(vec![length(input)?]),
        },
        ("keys" | "keys_unsorted", 0) => Ok(vec![keys(input, name == "keys")?]),
        ("values", 0) => {
            if input.is_null() {
                Ok(vec![])
            } else {
                Ok(vec![input.clone()])
            }
        }
        ("objects", 0) => Ok(if input.is_object() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("arrays", 0) => Ok(if input.is_array() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("strings", 0) => Ok(if input.is_string() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("numbers", 0) => Ok(if input.is_number() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("booleans", 0) => Ok(if input.is_boolean() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("nulls", 0) => Ok(if input.is_null() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("iterables", 0) => Ok(if input.is_array() || input.is_object() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("scalars", 0) => Ok(if !input.is_array() && !input.is_object() {
            vec![input.clone()]
        } else {
            vec![]
        }),
        ("has", 1) => {
            let keys = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for key in keys {
                match (input, &key) {
                    (Value::Object(m), Value::String(k)) => {
                        results.push(Value::Bool(m.contains_key(k)))
                    }
                    (Value::Array(a), Value::Number(n)) => {
                        let i = n.as_i64().unwrap_or(-1);
                        results.push(Value::Bool(i >= 0 && (i as usize) < a.len()));
                    }
                    (Value::Null, _) => results.push(Value::Bool(false)),
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "has() requires object/string or array/number".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("in", 1) => {
            let containers = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for container in containers {
                match (&container, input) {
                    (Value::Object(m), Value::String(k)) => {
                        results.push(Value::Bool(m.contains_key(k)))
                    }
                    (Value::Array(a), Value::Number(n)) => {
                        let i = n.as_i64().unwrap_or(-1);
                        results.push(Value::Bool(i >= 0 && (i as usize) < a.len()));
                    }
                    _ => results.push(Value::Bool(false)),
                }
            }
            Ok(results)
        }
        ("type", 0) => Ok(vec![Value::String(value_type(input).to_string())]),
        ("infinite", 0) => Ok(vec![json_f64(f64::INFINITY)]),
        ("nan", 0) => Ok(vec![json_f64(f64::NAN)]),
        ("isinfinite", 0) => match input {
            Value::Number(n) => Ok(vec![Value::Bool(
                n.as_f64().is_some_and(|f| f.is_infinite()),
            )]),
            _ => Ok(vec![Value::Bool(false)]),
        },
        ("isnan", 0) => match input {
            Value::Number(n) => Ok(vec![Value::Bool(n.as_f64().is_some_and(|f| f.is_nan()))]),
            _ => Ok(vec![Value::Bool(false)]),
        },
        ("isnormal", 0) => match input {
            Value::Number(n) => Ok(vec![Value::Bool(n.as_f64().is_some_and(|f| f.is_normal()))]),
            _ => Ok(vec![Value::Bool(false)]),
        },
        ("builtins", 0) => {
            let names = builtin_names();
            Ok(vec![Value::Array(
                names.into_iter().map(Value::String).collect(),
            )])
        }

        // ── Selection / filtering ──────────────────────────
        ("empty", 0) => Ok(vec![]),
        ("error", 0) => Err(EvalError::UserValue(input.clone())),
        ("error", 1) => {
            let msgs = eval_inner(&args[0], input, env)?;
            if let Some(msg) = msgs.into_iter().next() {
                Err(EvalError::UserValue(msg))
            } else {
                Ok(vec![])
            }
        }
        ("debug", 0) => {
            eprintln!("[\"DEBUG:\",{}]", input);
            Ok(vec![input.clone()])
        }
        ("debug", 1) => {
            let msgs = eval_inner(&args[0], input, env)?;
            for msg in msgs {
                eprintln!("[\"DEBUG:\",{},{}]", msg, input);
            }
            Ok(vec![input.clone()])
        }

        // ── Map / transform ────────────────────────────────
        ("map", 1) => match input {
            Value::Array(arr) => {
                let mut results = Vec::new();
                for item in arr {
                    results.extend(eval_inner(&args[0], item, env)?);
                }
                Ok(vec![Value::Array(results)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "map requires array, got {}",
                value_type(input)
            )))),
        },
        ("map_values", 1) => match input {
            Value::Object(map) => {
                let mut result = serde_json::Map::new();
                for (k, v) in map {
                    let vals = eval_inner(&args[0], v, env)?;
                    if let Some(new_v) = vals.into_iter().next() {
                        result.insert(k.clone(), new_v);
                    }
                }
                Ok(vec![Value::Object(result)])
            }
            Value::Array(arr) => {
                let mut results = Vec::new();
                for item in arr {
                    results.extend(eval_inner(&args[0], item, env)?);
                }
                Ok(vec![Value::Array(results)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "map_values requires object or array".into(),
            ))),
        },
        ("to_entries", 0) => match input {
            Value::Object(map) => {
                let entries: Vec<Value> = map
                    .iter()
                    .map(|(k, v)| {
                        let mut e = serde_json::Map::new();
                        e.insert("key".into(), Value::String(k.clone()));
                        e.insert("value".into(), v.clone());
                        Value::Object(e)
                    })
                    .collect();
                Ok(vec![Value::Array(entries)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "to_entries requires object".into(),
            ))),
        },
        ("from_entries", 0) => match input {
            Value::Array(arr) => {
                let mut map = serde_json::Map::new();
                for item in arr {
                    let key = item
                        .get("key")
                        .or_else(|| item.get("name"))
                        .and_then(|v| match v {
                            Value::String(s) => Some(s.clone()),
                            Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let val = item.get("value").cloned().unwrap_or(Value::Null);
                    map.insert(key, val);
                }
                Ok(vec![Value::Object(map)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "from_entries requires array".into(),
            ))),
        },
        ("with_entries", 1) => {
            // Equivalent to: to_entries | map(f) | from_entries
            let entries = call_builtin_vec("to_entries", &[], input, env)?;
            let mapped = call_builtin_vec("map", args, &entries[0], env)?;
            call_builtin_vec("from_entries", &[], &mapped[0], env)
        }
        ("transpose", 0) => match input {
            Value::Array(arr) => {
                if arr.is_empty() {
                    return Ok(vec![Value::Array(vec![])]);
                }
                let max_len = arr
                    .iter()
                    .filter_map(|v| v.as_array())
                    .map(|a| a.len())
                    .max()
                    .unwrap_or(0);
                let mut result = Vec::new();
                for i in 0..max_len {
                    let row: Vec<Value> = arr
                        .iter()
                        .map(|v| {
                            v.as_array()
                                .and_then(|a| a.get(i).cloned())
                                .unwrap_or(Value::Null)
                        })
                        .collect();
                    result.push(Value::Array(row));
                }
                Ok(vec![Value::Array(result)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "transpose requires array".into(),
            ))),
        },

        // ── Aggregation ────────────────────────────────────
        ("add", 0) => match input {
            Value::Array(arr) => {
                if arr.is_empty() {
                    return Ok(vec![Value::Null]);
                }
                let mut acc = arr[0].clone();
                for item in &arr[1..] {
                    let vals = eval_inner(
                        &Expr::BinOp(
                            super::ast::BinOp::Add,
                            Box::new(Expr::Identity),
                            Box::new(Expr::Literal(item.clone())),
                        ),
                        &acc,
                        env,
                    )?;
                    acc = vals.into_iter().next().unwrap_or(Value::Null);
                }
                Ok(vec![acc])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "add requires array".into(),
            ))),
        },
        ("any", 0) => match input {
            Value::Array(arr) => Ok(vec![Value::Bool(arr.iter().any(is_truthy))]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "any requires array".into(),
            ))),
        },
        ("any", 1) => match input {
            Value::Array(arr) => {
                for item in arr {
                    let vals = eval_inner(&args[0], item, env)?;
                    if vals.iter().any(is_truthy) {
                        return Ok(vec![Value::Bool(true)]);
                    }
                }
                Ok(vec![Value::Bool(false)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "any requires array".into(),
            ))),
        },
        ("all", 0) => match input {
            Value::Array(arr) => Ok(vec![Value::Bool(arr.iter().all(is_truthy))]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "all requires array".into(),
            ))),
        },
        ("all", 1) => match input {
            Value::Array(arr) => {
                for item in arr {
                    let vals = eval_inner(&args[0], item, env)?;
                    if vals.iter().any(|v| !is_truthy(v)) {
                        return Ok(vec![Value::Bool(false)]);
                    }
                }
                Ok(vec![Value::Bool(true)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "all requires array".into(),
            ))),
        },
        ("flatten", 0) => Ok(flatten(input, usize::MAX)?),
        ("flatten", 1) => {
            let depths = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for depth in depths {
                let d = depth.as_u64().unwrap_or(1) as usize;
                results.extend(flatten(input, d)?);
            }
            Ok(results)
        }

        // ── Sorting ────────────────────────────────────────
        ("sort", 0) => match input {
            Value::Array(arr) => {
                let mut sorted = arr.clone();
                sorted.sort_by(compare_values);
                Ok(vec![Value::Array(sorted)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "sort requires array".into(),
            ))),
        },
        ("sort_by", 1) => match input {
            Value::Array(arr) => {
                let mut indexed: Vec<(Value, Value)> = Vec::with_capacity(arr.len());
                for item in arr {
                    let keys = eval_inner(&args[0], item, env)?;
                    indexed.push((Value::Array(keys), item.clone()));
                }
                indexed.sort_by(|a, b| compare_values(&a.0, &b.0));
                Ok(vec![Value::Array(
                    indexed.into_iter().map(|(_, v)| v).collect(),
                )])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "sort_by requires array".into(),
            ))),
        },
        ("group_by", 1) => match input {
            Value::Array(arr) => {
                let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(arr.len());
                for item in arr {
                    let keys = eval_inner(&args[0], item, env)?;
                    keyed.push((Value::Array(keys), item.clone()));
                }
                keyed.sort_by(|a, b| compare_values(&a.0, &b.0));

                let mut groups: Vec<Value> = Vec::new();
                let mut current_key: Option<Value> = None;
                let mut current_group: Vec<Value> = Vec::new();

                for (key, val) in keyed {
                    if current_key.as_ref() == Some(&key) {
                        current_group.push(val);
                    } else {
                        if !current_group.is_empty() {
                            groups.push(Value::Array(std::mem::take(&mut current_group)));
                        }
                        current_key = Some(key);
                        current_group.push(val);
                    }
                }
                if !current_group.is_empty() {
                    groups.push(Value::Array(current_group));
                }
                Ok(vec![Value::Array(groups)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "group_by requires array".into(),
            ))),
        },
        ("unique", 0) => match input {
            Value::Array(arr) => {
                let mut sorted = arr.clone();
                sorted.sort_by(compare_values);
                sorted.dedup();
                Ok(vec![Value::Array(sorted)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "unique requires array".into(),
            ))),
        },
        ("unique_by", 1) => match input {
            Value::Array(arr) => {
                let mut seen = Vec::new();
                let mut result = Vec::new();
                for item in arr {
                    let keys = eval_inner(&args[0], item, env)?;
                    let key = Value::Array(keys);
                    if !seen.contains(&key) {
                        seen.push(key);
                        result.push(item.clone());
                    }
                }
                Ok(vec![Value::Array(result)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "unique_by requires array".into(),
            ))),
        },
        ("reverse", 0) => match input {
            Value::Array(arr) => {
                let mut rev = arr.clone();
                rev.reverse();
                Ok(vec![Value::Array(rev)])
            }
            Value::String(s) => Ok(vec![Value::String(s.chars().rev().collect())]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "reverse requires array or string".into(),
            ))),
        },
        ("min", 0) => match input {
            Value::Array(arr) if !arr.is_empty() => {
                let m = arr.iter().min_by(|a, b| compare_values(a, b)).unwrap();
                Ok(vec![m.clone()])
            }
            Value::Array(_) => Ok(vec![Value::Null]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "min requires array".into(),
            ))),
        },
        ("max", 0) => match input {
            Value::Array(arr) if !arr.is_empty() => {
                let m = arr.iter().max_by(|a, b| compare_values(a, b)).unwrap();
                Ok(vec![m.clone()])
            }
            Value::Array(_) => Ok(vec![Value::Null]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "max requires array".into(),
            ))),
        },
        ("min_by", 1) => match input {
            Value::Array(arr) if !arr.is_empty() => {
                let mut keyed: Vec<(Value, &Value)> = Vec::with_capacity(arr.len());
                for item in arr {
                    let keys = eval_inner(&args[0], item, env)?;
                    keyed.push((Value::Array(keys), item));
                }
                let (_, m) = keyed
                    .into_iter()
                    .min_by(|a, b| compare_values(&a.0, &b.0))
                    .unwrap();
                Ok(vec![m.clone()])
            }
            Value::Array(_) => Ok(vec![Value::Null]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "min_by requires array".into(),
            ))),
        },
        ("max_by", 1) => match input {
            Value::Array(arr) if !arr.is_empty() => {
                let mut keyed: Vec<(Value, &Value)> = Vec::with_capacity(arr.len());
                for item in arr {
                    let keys = eval_inner(&args[0], item, env)?;
                    keyed.push((Value::Array(keys), item));
                }
                let (_, m) = keyed
                    .into_iter()
                    .max_by(|a, b| compare_values(&a.0, &b.0))
                    .unwrap();
                Ok(vec![m.clone()])
            }
            Value::Array(_) => Ok(vec![Value::Null]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "max_by requires array".into(),
            ))),
        },

        // ── Searching / containment ────────────────────────
        ("contains", 1) => {
            let others = eval_inner(&args[0], input, env)?;
            Ok(others
                .into_iter()
                .map(|other| Value::Bool(value_contains(input, &other)))
                .collect())
        }
        ("inside", 1) => {
            let others = eval_inner(&args[0], input, env)?;
            Ok(others
                .into_iter()
                .map(|other| Value::Bool(value_contains(&other, input)))
                .collect())
        }
        ("indices" | "index", 1) => {
            let needles = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for needle in needles {
                let val = match input {
                    Value::Array(arr) => {
                        let indices: Vec<Value> = arr
                            .iter()
                            .enumerate()
                            .filter(|(_, v)| *v == &needle)
                            .map(|(i, _)| Value::Number(i.into()))
                            .collect();
                        if name == "index" {
                            indices.into_iter().next().unwrap_or(Value::Null)
                        } else {
                            Value::Array(indices)
                        }
                    }
                    Value::String(s) => {
                        if let Value::String(pat) = &needle {
                            let indices: Vec<Value> = s
                                .match_indices(pat.as_str())
                                .map(|(i, _)| Value::Number(i.into()))
                                .collect();
                            if name == "index" {
                                indices.into_iter().next().unwrap_or(Value::Null)
                            } else {
                                Value::Array(indices)
                            }
                        } else {
                            Value::Null
                        }
                    }
                    _ => Value::Null,
                };
                results.push(val);
            }
            Ok(results)
        }
        ("rindex", 1) => {
            let needles = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for needle in needles {
                let val = match input {
                    Value::Array(arr) => {
                        let idx = arr.iter().rposition(|v| v == &needle);
                        idx.map(|i| Value::Number(i.into())).unwrap_or(Value::Null)
                    }
                    Value::String(s) => {
                        if let Value::String(pat) = &needle {
                            let idx = s.rfind(pat.as_str());
                            idx.map(|i| Value::Number(i.into())).unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        }
                    }
                    _ => Value::Null,
                };
                results.push(val);
            }
            Ok(results)
        }

        // ── Type conversion ────────────────────────────────
        ("tostring", 0) => match input {
            Value::String(_) => Ok(vec![input.clone()]),
            Value::Null => Ok(vec![Value::String("null".into())]),
            Value::Bool(b) => Ok(vec![Value::String(b.to_string())]),
            Value::Number(n) => Ok(vec![Value::String(n.to_string())]),
            _ => Ok(vec![Value::String(
                serde_json::to_string(input).unwrap_or_default(),
            )]),
        },
        ("tonumber", 0) => match input {
            Value::Number(_) => Ok(vec![input.clone()]),
            Value::String(s) => {
                let n: f64 = s
                    .parse()
                    .map_err(|_| QfError::TypeError(format!("cannot convert \"{s}\" to number")))?;
                Ok(vec![json_f64(n)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "cannot convert {} to number",
                value_type(input)
            )))),
        },
        ("ascii_downcase", 0) => match input {
            Value::String(s) => Ok(vec![Value::String(s.to_ascii_lowercase())]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "ascii_downcase requires string".into(),
            ))),
        },
        ("ascii_upcase", 0) => match input {
            Value::String(s) => Ok(vec![Value::String(s.to_ascii_uppercase())]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "ascii_upcase requires string".into(),
            ))),
        },
        ("ltrimstr", 1) => {
            let prefixes = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for prefix in prefixes {
                match (input, &prefix) {
                    (Value::String(s), Value::String(p)) => results.push(Value::String(
                        s.strip_prefix(p.as_str()).unwrap_or(s).to_string(),
                    )),
                    _ => results.push(input.clone()),
                }
            }
            Ok(results)
        }
        ("rtrimstr", 1) => {
            let suffixes = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for suffix in suffixes {
                match (input, &suffix) {
                    (Value::String(s), Value::String(p)) => results.push(Value::String(
                        s.strip_suffix(p.as_str()).unwrap_or(s).to_string(),
                    )),
                    _ => results.push(input.clone()),
                }
            }
            Ok(results)
        }
        ("trim", 0) => match input {
            Value::String(s) => Ok(vec![Value::String(s.trim().to_string())]),
            _ => Ok(vec![input.clone()]),
        },
        ("split", 1) => {
            let seps = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for sep in seps {
                match (input, &sep) {
                    (Value::String(s), Value::String(p)) => {
                        let parts: Vec<Value> = s
                            .split(p.as_str())
                            .map(|part| Value::String(part.to_string()))
                            .collect();
                        results.push(Value::Array(parts));
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "split requires string args".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("join", 1) => {
            let seps = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for sep in seps {
                match (input, &sep) {
                    (Value::Array(arr), Value::String(s)) => {
                        let parts: Vec<String> = arr
                            .iter()
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Null => String::new(),
                                v => v.to_string(),
                            })
                            .collect();
                        results.push(Value::String(parts.join(s)));
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "join requires array and string".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("startswith", 1) => {
            let prefixes = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for prefix in prefixes {
                match (input, &prefix) {
                    (Value::String(s), Value::String(p)) => {
                        results.push(Value::Bool(s.starts_with(p.as_str())))
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "startswith requires strings".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("endswith", 1) => {
            let suffixes = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for suffix in suffixes {
                match (input, &suffix) {
                    (Value::String(s), Value::String(p)) => {
                        results.push(Value::Bool(s.ends_with(p.as_str())))
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "endswith requires strings".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("ascii", 0) => match input {
            Value::Number(n) => {
                let c = n.as_u64().unwrap_or(0) as u8 as char;
                Ok(vec![Value::String(c.to_string())])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "ascii requires number".into(),
            ))),
        },
        ("explode", 0) => match input {
            Value::String(s) => Ok(vec![Value::Array(
                s.chars()
                    .map(|c| Value::Number((c as u32).into()))
                    .collect(),
            )]),
            _ => Err(EvalError::Qf(QfError::TypeError(
                "explode requires string".into(),
            ))),
        },
        ("implode", 0) => match input {
            Value::Array(arr) => {
                let s: String = arr
                    .iter()
                    .filter_map(|v| v.as_u64().and_then(|n| char::from_u32(n as u32)))
                    .collect();
                Ok(vec![Value::String(s)])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "implode requires array".into(),
            ))),
        },

        // ── Regex ──────────────────────────────────────────
        ("test", 1) | ("test", 2) => {
            let patterns = eval_inner(&args[0], input, env)?;
            let flags_list = if args.len() > 1 {
                eval_inner(&args[1], input, env)?
                    .into_iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else {
                vec![String::new()]
            };
            let mut results = Vec::new();
            for pattern in &patterns {
                for flags in &flags_list {
                    match (input, pattern) {
                        (Value::String(s), Value::String(p)) => {
                            let re = build_regex(p, flags)?;
                            results.push(Value::Bool(re.is_match(s)));
                        }
                        _ => {
                            return Err(EvalError::Qf(QfError::TypeError(
                                "test requires string input and pattern".into(),
                            )))
                        }
                    }
                }
            }
            Ok(results)
        }
        ("match", 1) | ("match", 2) => {
            let patterns = eval_inner(&args[0], input, env)?;
            let flags_list = if args.len() > 1 {
                eval_inner(&args[1], input, env)?
                    .into_iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else {
                vec![String::new()]
            };
            let mut results = Vec::new();
            for pattern in &patterns {
                for flags in &flags_list {
                    match (input, pattern) {
                        (Value::String(s), Value::String(p)) => {
                            let re = build_regex(p, flags)?;
                            if let Some(m) = re.find(s) {
                                let mut result = serde_json::Map::new();
                                result.insert("offset".into(), Value::Number(m.start().into()));
                                result.insert("length".into(), Value::Number(m.len().into()));
                                result
                                    .insert("string".into(), Value::String(m.as_str().to_string()));
                                let captures: Vec<Value> = re
                                    .captures(s)
                                    .map(|caps| {
                                        (1..caps.len())
                                            .map(|i| {
                                                let mut cap = serde_json::Map::new();
                                                if let Some(m) = caps.get(i) {
                                                    cap.insert(
                                                        "offset".into(),
                                                        Value::Number(m.start().into()),
                                                    );
                                                    cap.insert(
                                                        "length".into(),
                                                        Value::Number(m.len().into()),
                                                    );
                                                    cap.insert(
                                                        "string".into(),
                                                        Value::String(m.as_str().to_string()),
                                                    );
                                                    cap.insert("name".into(), Value::Null);
                                                }
                                                Value::Object(cap)
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                result.insert("captures".into(), Value::Array(captures));
                                results.push(Value::Object(result));
                            } else {
                                results.push(Value::Null);
                            }
                        }
                        _ => {
                            return Err(EvalError::Qf(QfError::TypeError(
                                "match requires string".into(),
                            )))
                        }
                    }
                }
            }
            Ok(results)
        }
        ("capture", 1) | ("capture", 2) => {
            let patterns = eval_inner(&args[0], input, env)?;
            let flags_list = if args.len() > 1 {
                eval_inner(&args[1], input, env)?
                    .into_iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else {
                vec![String::new()]
            };
            let mut results = Vec::new();
            for pattern in &patterns {
                for flags in &flags_list {
                    match (input, pattern) {
                        (Value::String(s), Value::String(p)) => {
                            let re = build_regex(p, flags)?;
                            if let Some(caps) = re.captures(s) {
                                let mut result = serde_json::Map::new();
                                for name in re.capture_names().flatten() {
                                    if let Some(m) = caps.name(name) {
                                        result.insert(
                                            name.to_string(),
                                            Value::String(m.as_str().to_string()),
                                        );
                                    }
                                }
                                results.push(Value::Object(result));
                            } else {
                                results.push(Value::Null);
                            }
                        }
                        _ => {
                            return Err(EvalError::Qf(QfError::TypeError(
                                "capture requires string".into(),
                            )))
                        }
                    }
                }
            }
            Ok(results)
        }
        ("scan", 1) => {
            let patterns = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for pattern in &patterns {
                match (input, pattern) {
                    (Value::String(s), Value::String(p)) => {
                        let re = build_regex(p, "")?;
                        let matches: Vec<Value> = re
                            .find_iter(s)
                            .map(|m| Value::String(m.as_str().to_string()))
                            .collect();
                        results.push(Value::Array(matches));
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "scan requires string".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("sub", 2) | ("sub", 3) | ("gsub", 2) | ("gsub", 3) => {
            let s = match input {
                Value::String(s) => s.as_str(),
                _ => {
                    return Err(EvalError::Qf(QfError::TypeError(format!(
                        "{name} requires string input"
                    ))))
                }
            };
            let patterns = eval_inner(&args[0], input, env)?;
            let flags_list = if args.len() > 2 {
                eval_inner(&args[2], input, env)?
                    .into_iter()
                    .map(|v| {
                        v.as_str().map(String::from).ok_or_else(|| {
                            EvalError::Qf(QfError::TypeError(format!(
                                "{name} flags must be string"
                            )))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![String::new()]
            };
            let mut results = Vec::new();
            for pattern in &patterns {
                let p = pattern.as_str().ok_or_else(|| {
                    EvalError::Qf(QfError::TypeError(format!("{name} pattern must be string")))
                })?;
                for flags in &flags_list {
                    let re = build_regex(p, flags)?;
                    let global = name == "gsub" || flags.contains('g');
                    results.extend(eval_regex_replace(s, &re, &args[1], env, global)?);
                }
            }
            Ok(results)
        }

        // ── Selection helpers ──────────────────────────────
        ("first", 0) => match input {
            Value::Array(arr) => Ok(vec![arr.first().cloned().unwrap_or(Value::Null)]),
            _ => Ok(vec![input.clone()]),
        },
        ("last", 0) => match input {
            Value::Array(arr) => Ok(vec![arr.last().cloned().unwrap_or(Value::Null)]),
            _ => Ok(vec![input.clone()]),
        },
        ("nth", 1) => {
            let ns = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for n in ns {
                let idx = n.as_u64().unwrap_or(0) as usize;
                match input {
                    Value::Array(arr) => results.push(arr.get(idx).cloned().unwrap_or(Value::Null)),
                    _ => results.push(Value::Null),
                }
            }
            Ok(results)
        }
        ("until", 2) => {
            let mut val = input.clone();
            loop {
                let conds = eval_inner(&args[0], &val, env)?;
                if conds.iter().any(is_truthy) {
                    return Ok(vec![val]);
                }
                let next = eval_inner(&args[1], &val, env)?;
                if let Some(nv) = next.into_iter().next() {
                    val = nv;
                } else {
                    return Ok(vec![]);
                }
            }
        }

        // ── Math ───────────────────────────────────────────
        ("floor", 0) => num_op(input, f64::floor),
        ("ceil", 0) => num_op(input, f64::ceil),
        ("round", 0) => num_op(input, f64::round),
        ("fabs", 0) => num_op(input, f64::abs),
        ("abs", 0) => match input {
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    if let Some(a) = i.checked_abs() {
                        return Ok(vec![Value::Number(a.into())]);
                    }
                }
                num_op(input, f64::abs)
            }
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "number required, got {}",
                value_type(input)
            )))),
        },
        ("now", 0) => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            Ok(vec![json_f64(secs)])
        }
        ("todate", 0) => match input {
            Value::Number(n) => {
                let secs = n.as_f64().unwrap_or(0.0).floor() as i64;
                Ok(vec![Value::String(unix_to_iso8601(secs))])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "todate requires number, got {}",
                value_type(input)
            )))),
        },
        ("fromdate", 0) => match input {
            Value::String(s) => {
                let secs = iso8601_to_unix(s)?;
                Ok(vec![Value::Number(secs.into())])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "fromdate requires string, got {}",
                value_type(input)
            )))),
        },
        ("sqrt", 0) => num_op(input, f64::sqrt),
        ("log", 0) => num_op(input, f64::ln),
        ("log2", 0) => num_op(input, f64::log2),
        ("log10", 0) => num_op(input, f64::log10),
        ("exp", 0) => num_op(input, f64::exp),
        ("exp2", 0) => num_op(input, f64::exp2),
        ("pow", 2) => {
            let bases = eval_inner(&args[0], input, env)?;
            let exps = eval_inner(&args[1], input, env)?;
            let mut results = Vec::new();
            for base in &bases {
                for exp in &exps {
                    results.push(json_f64(
                        base.as_f64()
                            .unwrap_or(0.0)
                            .powf(exp.as_f64().unwrap_or(0.0)),
                    ));
                }
            }
            Ok(results)
        }
        ("sin", 0) => num_op(input, f64::sin),
        ("cos", 0) => num_op(input, f64::cos),
        ("tan", 0) => num_op(input, f64::tan),
        ("asin", 0) => num_op(input, f64::asin),
        ("acos", 0) => num_op(input, f64::acos),
        ("atan", 0) => num_op(input, f64::atan),
        ("atan2", 2) => {
            let ys = eval_inner(&args[0], input, env)?;
            let xs = eval_inner(&args[1], input, env)?;
            let mut results = Vec::new();
            for y in &ys {
                for x in &xs {
                    results.push(json_f64(
                        y.as_f64().unwrap_or(0.0).atan2(x.as_f64().unwrap_or(0.0)),
                    ));
                }
            }
            Ok(results)
        }

        // ── JSON ───────────────────────────────────────────
        ("tojson", 0) => Ok(vec![Value::String(
            serde_json::to_string(input).unwrap_or_default(),
        )]),
        ("fromjson", 0) => match input {
            Value::String(s) => {
                let v: Value = serde_json::from_str(s)
                    .map_err(|e| QfError::Runtime(format!("fromjson: {e}")))?;
                Ok(vec![v])
            }
            _ => Err(EvalError::Qf(QfError::TypeError(
                "fromjson requires string".into(),
            ))),
        },

        // ── Paths ──────────────────────────────────────────
        ("path", 1) => {
            let paths = collect_paths(&args[0], input, env)?;
            Ok(paths.into_iter().map(path_segments_to_value).collect())
        }
        ("paths", 0) => {
            let mut result = Vec::new();
            collect_all_paths(input, &mut vec![], &mut result);
            Ok(result)
        }
        ("paths", 1) => {
            let mut all_paths = Vec::new();
            collect_all_paths_filtered(input, &mut vec![], &mut all_paths, &args[0], env)?;
            Ok(all_paths)
        }
        ("leaf_paths", 0) => {
            let mut result = Vec::new();
            collect_leaf_paths(input, &mut vec![], &mut result);
            Ok(result)
        }
        ("getpath", 1) => {
            let paths = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for path in paths {
                match &path {
                    Value::Array(arr) => {
                        let mut current = input.clone();
                        for seg in arr {
                            current = match seg {
                                Value::String(k) => current
                                    .as_object()
                                    .and_then(|m| m.get(k).cloned())
                                    .unwrap_or(Value::Null),
                                Value::Number(n) => {
                                    let i = n.as_i64().unwrap_or(0);
                                    current
                                        .as_array()
                                        .and_then(|a| a.get(i as usize).cloned())
                                        .unwrap_or(Value::Null)
                                }
                                _ => Value::Null,
                            };
                        }
                        results.push(current);
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "getpath requires array".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }
        ("setpath", 2) => {
            let paths = eval_inner(&args[0], input, env)?;
            let vals = eval_inner(&args[1], input, env)?;
            let mut results = Vec::new();
            for path in &paths {
                for val in &vals {
                    match path {
                        Value::Array(arr) => {
                            let segments: Vec<PathSegment> = arr
                                .iter()
                                .filter_map(|v| match v {
                                    Value::String(s) => Some(PathSegment::Key(s.clone())),
                                    Value::Number(n) => {
                                        Some(PathSegment::Index(n.as_i64().unwrap_or(0)))
                                    }
                                    _ => None,
                                })
                                .collect();
                            results.push(super::eval::set_path_pub(input, &segments, val.clone())?);
                        }
                        _ => {
                            return Err(EvalError::Qf(QfError::TypeError(
                                "setpath requires array path".into(),
                            )))
                        }
                    }
                }
            }
            Ok(results)
        }
        ("delpaths", 1) => {
            let paths_vals = eval_inner(&args[0], input, env)?;
            let mut results = Vec::new();
            for paths_val in paths_vals {
                match &paths_val {
                    Value::Array(paths) => {
                        let mut seg_paths: Vec<Vec<PathSegment>> = Vec::with_capacity(paths.len());
                        for path in paths {
                            match path {
                                Value::Array(segs) => {
                                    let segs: Vec<PathSegment> = segs
                                        .iter()
                                        .filter_map(|v| match v {
                                            Value::String(s) => Some(PathSegment::Key(s.clone())),
                                            Value::Number(n) => {
                                                Some(PathSegment::Index(n.as_i64().unwrap_or(0)))
                                            }
                                            _ => None,
                                        })
                                        .collect();
                                    seg_paths.push(segs);
                                }
                                _ => {
                                    return Err(EvalError::Qf(QfError::TypeError(
                                        "delpaths: path must be array".into(),
                                    )))
                                }
                            }
                        }
                        sort_paths_for_deletion(&mut seg_paths);
                        let mut result = input.clone();
                        for path in &seg_paths {
                            result = delete_path_segments(&result, path);
                        }
                        results.push(result);
                    }
                    _ => {
                        return Err(EvalError::Qf(QfError::TypeError(
                            "delpaths requires array".into(),
                        )))
                    }
                }
            }
            Ok(results)
        }

        // ── Environment ────────────────────────────────────
        ("env", 0) => {
            let mut map = serde_json::Map::new();
            for (k, v) in std::env::vars() {
                map.insert(k, Value::String(v));
            }
            Ok(vec![Value::Object(map)])
        }

        // ── Not ────────────────────────────────────────────
        ("not", 0) => Ok(vec![Value::Bool(!is_truthy(input))]),

        // ── Null / input ───────────────────────────────────
        ("null", 0) => Ok(vec![Value::Null]),
        ("true", 0) => Ok(vec![Value::Bool(true)]),
        ("false", 0) => Ok(vec![Value::Bool(false)]),
        ("input", 0) => Ok(vec![Value::Null]), // simplified
        ("inputs", 0) => Ok(vec![]),           // simplified

        // ── Array manipulation ─────────────────────────────
        ("del", 1) => {
            let mut paths = collect_paths(&args[0], input, env)?;
            sort_paths_for_deletion(&mut paths);
            let mut result = input.clone();
            for path in &paths {
                result = delete_path_segments(&result, path);
            }
            Ok(vec![result])
        }

        _ => Err(EvalError::Qf(QfError::UndefinedFunction(
            name.to_string(),
            args.len(),
        ))),
    }
}

/// Apply a format string (@base64, @csv, etc.)
pub fn apply_format(name: &str, input: &Value) -> Result<Vec<Value>, QfError> {
    match name {
        "base64" => {
            let s = value_to_string(input);
            Ok(vec![Value::String(BASE64.encode(s.as_bytes()))])
        }
        "base64d" => match input {
            Value::String(s) => {
                let bytes = BASE64
                    .decode(s.as_bytes())
                    .map_err(|e| QfError::Runtime(format!("@base64d: {e}")))?;
                let decoded = String::from_utf8(bytes)
                    .map_err(|e| QfError::Runtime(format!("@base64d: {e}")))?;
                Ok(vec![Value::String(decoded)])
            }
            _ => Err(QfError::TypeError("@base64d requires string".into())),
        },
        "uri" => {
            let s = value_to_string(input);
            let encoded: String = s
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "-_.~".contains(c) {
                        c.to_string()
                    } else {
                        format!("%{:02X}", c as u32)
                    }
                })
                .collect();
            Ok(vec![Value::String(encoded)])
        }
        "csv" => format_as_csv(input),
        "tsv" => format_as_tsv(input),
        "sh" => format_as_sh(input),
        "html" => {
            let s = value_to_string(input);
            let escaped = s
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\'', "&#39;")
                .replace('"', "&quot;");
            Ok(vec![Value::String(escaped)])
        }
        "json" => Ok(vec![Value::String(
            serde_json::to_string(input).unwrap_or_default(),
        )]),
        "text" => Ok(vec![Value::String(value_to_string(input))]),
        _ => Err(QfError::Runtime(format!("unknown format: @{name}"))),
    }
}

// ── Helpers ────────────────────────────────────────────────

fn length(input: &Value) -> Result<Value, QfError> {
    match input {
        Value::Null => Ok(Value::Number(0.into())),
        Value::Bool(_) => Err(QfError::TypeError("boolean has no length".into())),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0).abs();
            Ok(json_f64(f))
        }
        Value::String(s) => Ok(Value::Number(s.chars().count().into())),
        Value::Array(a) => Ok(Value::Number(a.len().into())),
        Value::Object(m) => Ok(Value::Number(m.len().into())),
    }
}

fn keys(input: &Value, sort: bool) -> Result<Value, QfError> {
    match input {
        Value::Object(m) => {
            let mut ks: Vec<String> = m.keys().cloned().collect();
            if sort {
                ks.sort();
            }
            Ok(Value::Array(ks.into_iter().map(Value::String).collect()))
        }
        Value::Array(a) => Ok(Value::Array(
            (0..a.len()).map(|i| Value::Number(i.into())).collect(),
        )),
        _ => Err(QfError::TypeError(format!(
            "keys requires object or array, got {}",
            value_type(input)
        ))),
    }
}

fn flatten(input: &Value, depth: usize) -> Result<Vec<Value>, QfError> {
    match input {
        Value::Array(arr) => {
            let mut result = Vec::new();
            flatten_recursive(arr, depth, &mut result);
            Ok(vec![Value::Array(result)])
        }
        _ => Err(QfError::TypeError("flatten requires array".into())),
    }
}

fn flatten_recursive(arr: &[Value], depth: usize, result: &mut Vec<Value>) {
    for item in arr {
        if depth > 0 {
            if let Value::Array(inner) = item {
                flatten_recursive(inner, depth - 1, result);
                continue;
            }
        }
        result.push(item.clone());
    }
}

fn value_contains(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.contains(b.as_str()),
        (Value::Array(a), Value::Array(b)) => {
            b.iter().all(|bv| a.iter().any(|av| value_contains(av, bv)))
        }
        (Value::Object(a), Value::Object(b)) => b
            .iter()
            .all(|(k, bv)| a.get(k).is_some_and(|av| value_contains(av, bv))),
        _ => a == b,
    }
}

fn num_op(input: &Value, f: fn(f64) -> f64) -> Result<Vec<Value>, EvalError> {
    match input {
        Value::Number(n) => {
            let result = f(n.as_f64().unwrap_or(0.0));
            Ok(vec![json_f64(result)])
        }
        _ => Err(EvalError::Qf(QfError::TypeError(format!(
            "number required, got {}",
            value_type(input)
        )))),
    }
}

fn build_regex(pattern: &str, flags: &str) -> Result<Regex, QfError> {
    let mut pat = pattern.to_string();
    if flags.contains('x') {
        // Extended mode: strip comments and whitespace
        pat = pat
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .collect::<Vec<_>>()
            .join("");
    }
    let case_insensitive = flags.contains('i');
    let multiline = flags.contains('m');
    let dotall = flags.contains('s');

    let mut re_str = String::new();
    if case_insensitive || multiline || dotall {
        re_str.push_str("(?");
        if case_insensitive {
            re_str.push('i');
        }
        if multiline {
            re_str.push('m');
        }
        if dotall {
            re_str.push('s');
        }
        re_str.push(')');
    }
    re_str.push_str(&pat);

    Regex::new(&re_str).map_err(|e| QfError::Runtime(format!("invalid regex: {e}")))
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn format_as_csv(input: &Value) -> Result<Vec<Value>, QfError> {
    match input {
        Value::Array(arr) => {
            let mut parts = Vec::with_capacity(arr.len());
            for v in arr {
                match v {
                    Value::String(s) => parts.push(format!("\"{}\"", s.replace('"', "\"\""))),
                    Value::Null => parts.push(String::new()),
                    Value::Bool(b) => parts.push(b.to_string()),
                    Value::Number(n) => parts.push(n.to_string()),
                    _ => {
                        return Err(QfError::TypeError(format!(
                            "{} is not valid in a csv row",
                            value_type(v)
                        )));
                    }
                }
            }
            Ok(vec![Value::String(parts.join(","))])
        }
        _ => Err(QfError::TypeError("@csv requires array".into())),
    }
}

fn format_as_tsv(input: &Value) -> Result<Vec<Value>, QfError> {
    match input {
        Value::Array(arr) => {
            let mut parts = Vec::with_capacity(arr.len());
            for v in arr {
                match v {
                    Value::String(s) => {
                        let mut escaped = String::with_capacity(s.len());
                        for ch in s.chars() {
                            match ch {
                                '\\' => escaped.push_str("\\\\"),
                                '\t' => escaped.push_str("\\t"),
                                '\n' => escaped.push_str("\\n"),
                                '\r' => escaped.push_str("\\r"),
                                _ => escaped.push(ch),
                            }
                        }
                        parts.push(escaped);
                    }
                    Value::Null => parts.push(String::new()),
                    Value::Bool(b) => parts.push(b.to_string()),
                    Value::Number(n) => parts.push(n.to_string()),
                    _ => {
                        return Err(QfError::TypeError(format!(
                            "{} is not valid in a tsv row",
                            value_type(v)
                        )));
                    }
                }
            }
            Ok(vec![Value::String(parts.join("\t"))])
        }
        _ => Err(QfError::TypeError("@tsv requires array".into())),
    }
}

fn format_sh_scalar(v: &Value) -> Result<String, QfError> {
    match v {
        Value::String(s) => Ok(format!("'{}'", s.replace('\'', "'\\''"))),
        Value::Null => Ok("null".into()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(QfError::TypeError(format!(
            "{} cannot be escaped for shell",
            value_type(v)
        ))),
    }
}

fn format_as_sh(input: &Value) -> Result<Vec<Value>, QfError> {
    match input {
        Value::Array(arr) => {
            let parts = arr
                .iter()
                .map(format_sh_scalar)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![Value::String(parts.join(" "))])
        }
        _ => Ok(vec![Value::String(format_sh_scalar(input)?)]),
    }
}

fn eval_regex_replace(
    s: &str,
    re: &Regex,
    repl_expr: &Expr,
    env: &Env,
    global: bool,
) -> Result<Vec<Value>, EvalError> {
    let matches: Vec<regex::Captures<'_>> = if global {
        re.captures_iter(s).collect()
    } else {
        re.captures(s).into_iter().collect()
    };
    if matches.is_empty() {
        return Ok(vec![Value::String(s.to_string())]);
    }
    let mut partials = vec![String::new()];
    let mut last_end = 0;
    for caps in matches {
        let m = caps.get(0).unwrap();
        let prefix = &s[last_end..m.start()];
        let mut cap_obj = serde_json::Map::new();
        for name in re.capture_names().flatten() {
            let val = match caps.name(name) {
                Some(cm) => Value::String(cm.as_str().to_string()),
                None => Value::Null,
            };
            cap_obj.insert(name.to_string(), val);
        }
        let repl_vals = eval_inner(repl_expr, &Value::Object(cap_obj), env)?;
        let mut next_partials = Vec::with_capacity(partials.len() * repl_vals.len());
        for rv in repl_vals {
            let r_str = rv.as_str().ok_or_else(|| {
                EvalError::Qf(QfError::TypeError(
                    "sub/gsub replacement must be string".into(),
                ))
            })?;
            for p in &partials {
                next_partials.push(format!("{p}{prefix}{r_str}"));
            }
        }
        partials = next_partials;
        last_end = m.end();
    }
    let suffix = &s[last_end..];
    for p in &mut partials {
        p.push_str(suffix);
    }
    Ok(partials.into_iter().map(Value::String).collect())
}

fn unix_to_iso8601(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // Howard Hinnant's civil_from_days algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn iso8601_to_unix(s: &str) -> Result<i64, EvalError> {
    let err = || EvalError::Qf(QfError::Runtime(format!("invalid ISO 8601 date: {s}")));
    let s = s.strip_suffix('Z').ok_or_else(err)?;
    let (date_part, time_part) = s.split_once('T').ok_or_else(err)?;
    let d_parts: Vec<&str> = date_part.split('-').collect();
    let t_parts: Vec<&str> = time_part.split(':').collect();
    if d_parts.len() != 3 || t_parts.len() != 3 {
        return Err(err());
    }
    let y: i64 = d_parts[0].parse().map_err(|_| err())?;
    let m: i64 = d_parts[1].parse().map_err(|_| err())?;
    let d: i64 = d_parts[2].parse().map_err(|_| err())?;
    let hh: i64 = t_parts[0].parse().map_err(|_| err())?;
    let mm: i64 = t_parts[1].parse().map_err(|_| err())?;
    let sec_str = t_parts[2].split('.').next().unwrap_or(t_parts[2]);
    let ss: i64 = sec_str.parse().map_err(|_| err())?;

    if !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
        || !(0..=23).contains(&hh)
        || !(0..=59).contains(&mm)
        || !(0..=59).contains(&ss)
    {
        return Err(err());
    }

    // Howard Hinnant's days_from_civil algorithm
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    Ok(days * 86400 + hh * 3600 + mm * 60 + ss)
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    super::eval::compare_values_pub(a, b)
}

fn collect_all_paths(val: &Value, current: &mut Vec<Value>, result: &mut Vec<Value>) {
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(Value::Number(i.into()));
                result.push(Value::Array(current.clone()));
                collect_all_paths(item, current, result);
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(Value::String(k.clone()));
                result.push(Value::Array(current.clone()));
                collect_all_paths(v, current, result);
                current.pop();
            }
        }
        _ => {}
    }
}

fn collect_all_paths_filtered(
    val: &Value,
    current: &mut Vec<Value>,
    result: &mut Vec<Value>,
    filter: &Expr,
    env: &Env,
) -> Result<(), EvalError> {
    let filter_results = eval_inner(filter, val, env)?;
    if filter_results.iter().any(is_truthy) {
        result.push(Value::Array(current.clone()));
    }
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(Value::Number(i.into()));
                collect_all_paths_filtered(item, current, result, filter, env)?;
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(Value::String(k.clone()));
                collect_all_paths_filtered(v, current, result, filter, env)?;
                current.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_leaf_paths(val: &Value, current: &mut Vec<Value>, result: &mut Vec<Value>) {
    match val {
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                current.push(Value::Number(i.into()));
                collect_leaf_paths(item, current, result);
                current.pop();
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                current.push(Value::String(k.clone()));
                collect_leaf_paths(v, current, result);
                current.pop();
            }
        }
        _ => {
            result.push(Value::Array(current.clone()));
        }
    }
}

fn builtin_names() -> Vec<String> {
    vec![
        "length",
        "utf8bytelength",
        "keys",
        "keys_unsorted",
        "values",
        "objects",
        "arrays",
        "strings",
        "numbers",
        "booleans",
        "nulls",
        "iterables",
        "scalars",
        "has",
        "in",
        "IN",
        "type",
        "infinite",
        "nan",
        "isinfinite",
        "isnan",
        "isnormal",
        "builtins",
        "select",
        "empty",
        "isempty",
        "error",
        "debug",
        "map",
        "map_values",
        "to_entries",
        "from_entries",
        "with_entries",
        "transpose",
        "add",
        "any",
        "all",
        "flatten",
        "range",
        "sort",
        "sort_by",
        "group_by",
        "unique",
        "unique_by",
        "reverse",
        "min",
        "max",
        "min_by",
        "max_by",
        "contains",
        "inside",
        "indices",
        "index",
        "rindex",
        "tostring",
        "tonumber",
        "ascii_downcase",
        "ascii_upcase",
        "ltrimstr",
        "rtrimstr",
        "trim",
        "split",
        "splits",
        "join",
        "startswith",
        "endswith",
        "ascii",
        "explode",
        "implode",
        "test",
        "match",
        "capture",
        "scan",
        "sub",
        "gsub",
        "first",
        "last",
        "nth",
        "limit",
        "recurse",
        "until",
        "while",
        "repeat",
        "walk",
        "tostream",
        "fromstream",
        "floor",
        "ceil",
        "round",
        "fabs",
        "abs",
        "now",
        "todate",
        "fromdate",
        "sqrt",
        "log",
        "log2",
        "log10",
        "exp",
        "exp2",
        "pow",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "tojson",
        "fromjson",
        "path",
        "paths",
        "leaf_paths",
        "getpath",
        "setpath",
        "delpaths",
        "env",
        "not",
        "null",
        "true",
        "false",
        "input",
        "inputs",
        "del",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_length() {
        assert_eq!(length(&json!("hello")).unwrap(), json!(5));
        assert_eq!(length(&json!([1, 2, 3])).unwrap(), json!(3));
        assert_eq!(length(&json!({"a": 1, "b": 2})).unwrap(), json!(2));
        assert_eq!(length(&json!(null)).unwrap(), json!(0));
    }

    #[test]
    fn test_keys() {
        let result = keys(&json!({"b": 1, "a": 2}), true).unwrap();
        assert_eq!(result, json!(["a", "b"]));
    }

    #[test]
    fn test_format_base64() {
        let result = apply_format("base64", &json!("hello")).unwrap();
        assert_eq!(result, vec![json!("aGVsbG8=")]);
    }

    #[test]
    fn test_format_base64d() {
        let result = apply_format("base64d", &json!("aGVsbG8=")).unwrap();
        assert_eq!(result, vec![json!("hello")]);
    }

    #[test]
    fn test_format_html() {
        let result = apply_format("html", &json!("<b>test</b>")).unwrap();
        assert_eq!(result, vec![json!("&lt;b&gt;test&lt;/b&gt;")]);
    }

    #[test]
    fn test_contains() {
        assert!(value_contains(&json!("foobar"), &json!("foo")));
        assert!(value_contains(&json!([1, 2, 3]), &json!([2])));
        assert!(value_contains(&json!({"a": 1, "b": 2}), &json!({"a": 1})));
    }

    #[test]
    fn test_flatten() {
        let result = flatten(&json!([[1, 2], [3, [4, 5]]]), usize::MAX).unwrap();
        assert_eq!(result, vec![json!([1, 2, 3, 4, 5])]);
    }

    fn run_jq(input_str: &str, query: &str) -> Vec<Value> {
        let input: Value = serde_json::from_str(input_str).unwrap();
        let mut lexer = crate::query::lexer::Lexer::new(query);
        let tokens = lexer.tokenize().unwrap().to_vec();
        let expr = crate::query::jq_parser::Parser::new(tokens)
            .parse()
            .unwrap();
        let env = Env::new();
        super::super::eval::eval(&expr, &input, &env).unwrap()
    }

    #[test]
    fn test_values_and_type_selectors() {
        assert_eq!(run_jq("null", "values"), Vec::<Value>::new());
        assert_eq!(
            run_jq(r#"[1, null, "a", true, {}, []]"#, "[.[] | values]"),
            vec![json!([1, "a", true, {}, []])]
        );
        let mixed = r#"[null, true, false, 1, "s", [1], {"a":1}]"#;
        assert_eq!(run_jq(mixed, "[.[] | objects]"), vec![json!([{"a": 1}])]);
        assert_eq!(run_jq(mixed, "[.[] | arrays]"), vec![json!([[1]])]);
        assert_eq!(run_jq(mixed, "[.[] | strings]"), vec![json!(["s"])]);
        assert_eq!(run_jq(mixed, "[.[] | numbers]"), vec![json!([1])]);
        assert_eq!(
            run_jq(mixed, "[.[] | booleans]"),
            vec![json!([true, false])]
        );
        assert_eq!(run_jq(mixed, "[.[] | nulls]"), vec![json!([null])]);
        assert_eq!(
            run_jq(mixed, "[.[] | iterables]"),
            vec![json!([[1], {"a": 1}])]
        );
        assert_eq!(
            run_jq(mixed, "[.[] | scalars]"),
            vec![json!([null, true, false, 1, "s"])]
        );
    }

    #[test]
    fn test_format_csv_and_sh() {
        assert_eq!(
            run_jq(r#"[1, "a", "x\"y", null, true]"#, "@csv"),
            vec![json!("1,\"a\",\"x\"\"y\",,true")]
        );
        assert_eq!(
            run_jq(r#"["a", "b'c", 1, true, null]"#, "@sh"),
            vec![json!("'a' 'b'\\''c' 1 true null")]
        );
    }

    #[test]
    fn test_sub_and_gsub_flags_and_named_captures() {
        assert_eq!(
            run_jq(r#""Hello HELLO""#, r#"sub("hello"; "hi"; "i")"#),
            vec![json!("hi HELLO")]
        );
        assert_eq!(
            run_jq(r#""Hello HELLO""#, r#"sub("hello"; "hi"; "ig")"#),
            vec![json!("hi hi")]
        );
        assert_eq!(
            run_jq(r#""a1b2""#, r#"gsub("(?<d>[0-9])"; "[\(.d)]")"#),
            vec![json!("a[1]b[2]")]
        );
        assert_eq!(
            run_jq(
                r#""foo bar""#,
                r#"sub("(?<w>[a-z]+)"; "\(.w | ascii_upcase)")"#
            ),
            vec![json!("FOO bar")]
        );
    }

    #[test]
    fn test_map_values_empty() {
        assert_eq!(
            run_jq(r#"{"a": 1, "b": 2}"#, "map_values(empty)"),
            vec![json!({})]
        );
        assert_eq!(
            run_jq(r#"{"a": 1, "b": 3}"#, "map_values(select(. > 1))"),
            vec![json!({"b": 3})]
        );
    }

    #[test]
    fn test_abs_splits_any_all_isempty_in_nth() {
        assert_eq!(run_jq("-5", "abs"), vec![json!(5)]);
        assert_eq!(run_jq("-2.5", "abs"), vec![json!(2.5)]);
        assert_eq!(
            run_jq(r#""a, b; c""#, r#"[splits("[,;]\\s*")]"#),
            vec![json!(["a", "b", "c"])]
        );
        assert_eq!(
            run_jq(r#""aXbxc""#, r#"[splits("x"; "i")]"#),
            vec![json!(["a", "b", "c"])]
        );
        assert_eq!(
            run_jq("null", "any(range(1000000000); . == 5)"),
            vec![json!(true)]
        );
        assert_eq!(run_jq("null", "all(range(5); . < 5)"), vec![json!(true)]);
        assert_eq!(
            run_jq("null", "all(range(1000000000); . < 3)"),
            vec![json!(false)]
        );
        assert_eq!(run_jq("null", "isempty(empty)"), vec![json!(true)]);
        assert_eq!(run_jq("null", "isempty(1, 2)"), vec![json!(false)]);
        assert_eq!(run_jq("2", "IN(1, 2, 3)"), vec![json!(true)]);
        assert_eq!(run_jq("4", "IN(1, 2, 3)"), vec![json!(false)]);
        assert_eq!(run_jq("null", "nth(5; range(1000000000))"), vec![json!(5)]);
    }

    #[test]
    fn test_walk_tostream_fromstream() {
        assert_eq!(
            run_jq(
                r#"{"a": [1, 2], "b": {"c": 3}}"#,
                "walk(if type == \"number\" then . + 1 else . end)"
            ),
            vec![json!({"a": [2, 3], "b": {"c": 4}})]
        );
        assert_eq!(
            run_jq("[1, 2]", "[tostream]"),
            vec![json!([[[0], 1], [[1], 2], [[1]]])]
        );
        assert_eq!(
            run_jq(r#"{"a": 1, "b": [2, {}]}"#, "fromstream(tostream)"),
            vec![json!({"a": 1, "b": [2, {}]})]
        );
        assert_eq!(run_jq("42", "fromstream(tostream)"), vec![json!(42)]);
    }

    #[test]
    fn test_now_todate_fromdate() {
        let now_res = run_jq("null", "now");
        assert_eq!(now_res.len(), 1);
        assert!(now_res[0].as_f64().unwrap() > 1_700_000_000.0);

        assert_eq!(run_jq("0", "todate"), vec![json!("1970-01-01T00:00:00Z")]);
        assert_eq!(
            run_jq("1700000000", "todate"),
            vec![json!("2023-11-14T22:13:20Z")]
        );
        assert_eq!(
            run_jq(r#""2023-11-14T22:13:20Z""#, "fromdate"),
            vec![json!(1700000000)]
        );
        assert_eq!(run_jq("-86400", "todate | fromdate"), vec![json!(-86400)]);
    }

    #[test]
    fn test_builtins_list_contains_new_entries() {
        let names = builtin_names();
        for expected in [
            "abs",
            "objects",
            "arrays",
            "strings",
            "numbers",
            "booleans",
            "nulls",
            "iterables",
            "scalars",
            "splits",
            "isempty",
            "IN",
            "walk",
            "tostream",
            "fromstream",
            "now",
            "todate",
            "fromdate",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }
}
