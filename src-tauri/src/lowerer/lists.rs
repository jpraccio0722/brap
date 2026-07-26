//! Compile-time list builtins.
//!
//! These all run during lowering and emit no nodes (except `sum`, which folds
//! numbers but emits `Add` nodes when the list holds signals). Lists are
//! immutable: every function returns a new list rather than mutating one.

use std::rc::Rc;

use crate::brap_graph::environment::Value;
use crate::brap_graph::ugen_nodes::NodeKind;
use crate::lowerer::lower::Lowerer;

fn as_list(func: &str, v: &Value) -> Result<Rc<Vec<Value>>, String> {
    match v {
        Value::List(items) => Ok(items.clone()),
        _ => Err(format!("{func} expects a list")),
    }
}

fn as_number(func: &str, what: &str, v: &Value) -> Result<f64, String> {
    match v {
        Value::Number(n) => Ok(*n),
        _ => Err(format!("{func}: {what} must be a compile-time number")),
    }
}

fn as_count(func: &str, what: &str, v: &Value) -> Result<i64, String> {
    let n = as_number(func, what, v)?;
    if n.fract() != 0.0 || !n.is_finite() {
        return Err(format!("{func}: {what} must be a whole number, got {n}"));
    }
    Ok(n as i64)
}

fn arity(func: &str, args: &[Value], allowed: &[usize]) -> Result<(), String> {
    if allowed.contains(&args.len()) {
        return Ok(());
    }
    let expected = allowed
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" or ");
    Err(format!(
        "{func} expects {expected} arguments, got {}",
        args.len()
    ))
}

fn list(items: Vec<Value>) -> Result<Option<Value>, String> {
    Ok(Some(Value::List(Rc::new(items))))
}

impl Lowerer {
    /// Advance the lowering RNG. Seeded once per eval, so re-running a program
    /// with `choice` or `scramble` in it gives a fresh result each time.
    pub fn next_rand(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng | 1;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform float in `[0, 1)`.
    fn next_unit(&mut self) -> f64 {
        (self.next_rand() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn next_index(&mut self, len: usize) -> usize {
        (self.next_rand() % len as u64) as usize
    }

    pub fn list_builtin(&mut self, func: &str, args: &[Value]) -> Result<Option<Value>, String> {
        match func {
            "len" => {
                arity(func, args, &[1])?;
                Ok(Some(Value::Number(as_list(func, &args[0])?.len() as f64)))
            }

            // `zip(a, b)` pairs elements positionally: [[a0, b0], [a1, b1], ...].
            // Lengths must match — at lowering time a mismatch is a mistake, not
            // something to silently truncate.
            "zip" => {
                if args.is_empty() {
                    return Err("zip expects at least one list".into());
                }
                let lists = args
                    .iter()
                    .map(|v| match v {
                        Value::List(items) => Ok(items.clone()),
                        _ => Err("zip expects every argument to be a list".to_string()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let len = lists[0].len();
                if let Some(i) = lists.iter().position(|l| l.len() != len) {
                    return Err(format!(
                        "zip: argument {} has length {}, but argument 1 has length {}",
                        i + 1,
                        lists[i].len(),
                        len
                    ));
                }

                let rows = (0..len)
                    .map(|i| Value::List(Rc::new(lists.iter().map(|l| l[i].clone()).collect())))
                    .collect();
                list(rows)
            }

            "rev" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                list(items.iter().rev().cloned().collect())
            }

            // `[a, b, c]` -> `[a, b, c, c, b, a]`: the sequence, then its mirror.
            "palindrome" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                let mut out: Vec<Value> = items.iter().cloned().collect();
                out.extend(items.iter().rev().cloned());
                list(out)
            }

            // Rotation amount defaults to 1 and wraps, so `rotl(l, len(l))` is
            // the identity and a negative amount rotates the other way.
            "rotl" | "rotr" => {
                arity(func, args, &[1, 2])?;
                let items = as_list(func, &args[0])?;
                if items.is_empty() {
                    return list(Vec::new());
                }
                let n = match args.get(1) {
                    None => 1,
                    Some(v) => as_count(func, "the rotation amount", v)?,
                };
                let n = if func == "rotr" { -n } else { n };
                let at = n.rem_euclid(items.len() as i64) as usize;
                let mut out: Vec<Value> = items[at..].to_vec();
                out.extend_from_slice(&items[..at]);
                list(out)
            }

            "push" => {
                arity(func, args, &[2])?;
                let items = as_list(func, &args[0])?;
                let mut out: Vec<Value> = items.iter().cloned().collect();
                out.push(args[1].clone());
                list(out)
            }

            // Removes the last element. Lists are immutable, so nothing is
            // returned "off the top" — index the list for that.
            "pop" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                if items.is_empty() {
                    return Err("pop: the list is already empty".into());
                }
                list(items[..items.len() - 1].to_vec())
            }

            "sort" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                let mut nums = items
                    .iter()
                    .map(|v| as_number(func, "every element", v))
                    .collect::<Result<Vec<_>, _>>()?;
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                list(nums.into_iter().map(Value::Number).collect())
            }

            "sum" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                let mut it = items.iter();
                let Some(first) = it.next() else {
                    return Ok(Some(Value::Number(0.0)));
                };
                // combine folds numbers and emits Add nodes for signals, so a
                // list of oscillators sums into the graph just like `for` does.
                let mut acc = first.clone();
                for v in it {
                    acc = self.combine(NodeKind::Add, |a, b| a + b, acc, v.clone())?;
                }
                Ok(Some(acc))
            }

            // Chunks of `n`; a short final chunk is kept.
            "split" => {
                arity(func, args, &[2])?;
                let items = as_list(func, &args[0])?;
                let n = as_count(func, "the chunk size", &args[1])?;
                if n < 1 {
                    return Err(format!("split: the chunk size must be at least 1, got {n}"));
                }
                let chunks = items
                    .chunks(n as usize)
                    .map(|c| Value::List(Rc::new(c.to_vec())))
                    .collect();
                list(chunks)
            }

            "choice" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                if items.is_empty() {
                    return Err("choice: the list is empty".into());
                }
                let i = self.next_index(items.len());
                Ok(Some(items[i].clone()))
            }

            // `wchoice(values, weights)` — parallel lists, like zip.
            "wchoice" => {
                arity(func, args, &[2])?;
                let items = as_list(func, &args[0])?;
                let weights = as_list(func, &args[1])?;
                if items.is_empty() {
                    return Err("wchoice: the list is empty".into());
                }
                if items.len() != weights.len() {
                    return Err(format!(
                        "wchoice: {} values but {} weights",
                        items.len(),
                        weights.len()
                    ));
                }
                let ws = weights
                    .iter()
                    .map(|v| as_number(func, "every weight", v))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(bad) = ws.iter().find(|w| **w < 0.0 || !w.is_finite()) {
                    return Err(format!("wchoice: weights must be finite and >= 0, got {bad}"));
                }
                let total: f64 = ws.iter().sum();
                if total <= 0.0 {
                    return Err("wchoice: the weights are all zero".into());
                }

                let mut point = self.next_unit() * total;
                for (i, w) in ws.iter().enumerate() {
                    point -= w;
                    if point < 0.0 {
                        return Ok(Some(items[i].clone()));
                    }
                }
                Ok(Some(items[items.len() - 1].clone()))
            }

            "scramble" => {
                arity(func, args, &[1])?;
                let items = as_list(func, &args[0])?;
                let mut out: Vec<Value> = items.iter().cloned().collect();
                // Fisher-Yates.
                for i in (1..out.len()).rev() {
                    let j = self.next_index(i + 1);
                    out.swap(i, j);
                }
                list(out)
            }

            // `filter(list, predicate)` — keeps elements the predicate answers
            // non-zero for. The predicate is an ordinary user `fn`.
            "filter" => {
                arity(func, args, &[2])?;
                let items = as_list(func, &args[0])?;
                let Value::Function(def) = &args[1] else {
                    return Err("filter: the second argument must be a function".into());
                };

                let mut out = Vec::new();
                for v in items.iter() {
                    let keep = self.apply("filter", def.clone(), vec![v.clone()])?;
                    match keep {
                        Value::Number(n) => {
                            if n != 0.0 {
                                out.push(v.clone());
                            }
                        }
                        _ => {
                            return Err(
                                "filter: the predicate must return a compile-time number".into()
                            );
                        }
                    }
                }
                list(out)
            }

            _ => Ok(None),
        }
    }
}
