use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use regex::Regex;
use serde_json::Value;

use crate::error::QfError;

use super::ast::Expr;
use super::env::Env;
use super::eval::{
    collect_paths, delete_path_segments, eval_inner, eval_stream, is_truthy,
    path_segments_to_value, sort_paths_for_deletion, value_type, EvalError, PathSegment,
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
        ("values", 0) => match input {
            Value::Object(m) => Ok(vec![Value::Array(m.values().cloned().collect())]),
            Value::Array(_) => Ok(vec![input.clone()]),
            _ => Err(EvalError::Qf(QfError::TypeError(format!(
                "{} is not iterable",
                value_type(input)
            )))),
        },
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
        ("sub", 2) | ("sub", 3) => {
            let patterns = eval_inner(&args[0], input, env)?;
            let replacements = eval_inner(&args[1], input, env)?;
            let flags_list = if args.len() > 2 {
                eval_inner(&args[2], input, env)?
                    .into_iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else {
                vec![String::new()]
            };
            let mut results = Vec::new();
            for pattern in &patterns {
                for replacement in &replacements {
                    for flags in &flags_list {
                        match (input, pattern, replacement) {
                            (Value::String(s), Value::String(p), Value::String(r)) => {
                                let re = build_regex(p, flags)?;
                                results.push(Value::String(re.replace(s, r.as_str()).to_string()));
                            }
                            _ => {
                                return Err(EvalError::Qf(QfError::TypeError(
                                    "sub requires strings".into(),
                                )))
                            }
                        }
                    }
                }
            }
            Ok(results)
        }
        ("gsub", 2) | ("gsub", 3) => {
            let patterns = eval_inner(&args[0], input, env)?;
            let replacements = eval_inner(&args[1], input, env)?;
            let flags_list = if args.len() > 2 {
                eval_inner(&args[2], input, env)?
                    .into_iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else {
                vec![String::new()]
            };
            let mut results = Vec::new();
            for pattern in &patterns {
                for replacement in &replacements {
                    for flags in &flags_list {
                        match (input, pattern, replacement) {
                            (Value::String(s), Value::String(p), Value::String(r)) => {
                                let re = build_regex(p, flags)?;
                                results
                                    .push(Value::String(re.replace_all(s, r.as_str()).to_string()));
                            }
                            _ => {
                                return Err(EvalError::Qf(QfError::TypeError(
                                    "gsub requires strings".into(),
                                )))
                            }
                        }
                    }
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
        "csv" => format_as_csv(input, b','),
        "tsv" => format_as_csv(input, b'\t'),
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

fn json_f64(f: f64) -> Value {
    if f.fract() == 0.0 && f.is_finite() && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
        Value::Number((f as i64).into())
    } else {
        serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null)
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

fn format_as_csv(input: &Value, delimiter: u8) -> Result<Vec<Value>, QfError> {
    match input {
        Value::Array(arr) => {
            let mut wtr = csv::WriterBuilder::new()
                .delimiter(delimiter)
                .from_writer(vec![]);
            let fields: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    v => v.to_string(),
                })
                .collect();
            wtr.write_record(&fields)
                .map_err(|e| QfError::Runtime(e.to_string()))?;
            let bytes = wtr
                .into_inner()
                .map_err(|e| QfError::Runtime(e.to_string()))?;
            let s = String::from_utf8(bytes).map_err(|e| QfError::Runtime(e.to_string()))?;
            Ok(vec![Value::String(s.trim_end().to_string())])
        }
        _ => Err(QfError::TypeError("@csv/@tsv requires array".into())),
    }
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
        "has",
        "in",
        "type",
        "infinite",
        "nan",
        "isinfinite",
        "isnan",
        "isnormal",
        "builtins",
        "select",
        "empty",
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
        "floor",
        "ceil",
        "round",
        "fabs",
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
}
