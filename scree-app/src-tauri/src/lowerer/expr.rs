use std::fmt::format;
use std::rc::Rc;

use crate::scree_graph::environment::{Env, Item, Length, Value};
use crate::scree_graph::ugen_nodes::{NodeInput, NodeKind};
use crate::lowerer::lower::Lowerer;
use crate::parser::parser::{CmpOp, Expr, Statement};
use crate::parser::parser::Range;

const MAX_UNROLL: usize = 1024;

impl Lowerer {
    pub fn expr(&mut self, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Add { lhs, rhs } =>
                self.binop(NodeKind::Add, |a, b| a + b, lhs, rhs),
            
            Expr::Block { stmts , tail } => {
                self.env.push_scope();
                
                let result = (|| {
                    for stmt in stmts {
                        match stmt {
                            Statement::Let { name, value } => {
                                let v = self.expr(value)?;
                                self.env.define(&name.0, v);
                            }
                            Statement::Expr(e) => { self.expr(e)?; }
                        }
                    }
                    self.expr(tail)
                })();

                self.env.pop_scope();
                result
            }
            
            Expr::Call { func, args } => 
                self.call(func, args),
            
            Expr::Chain { lhs , rhs } => {
                let piped = self.expr(lhs)?;
                match rhs.as_ref() {
                    Expr::Call { func, args } => 
                        self.call_with(func, args, Some(piped)),
                    Expr::Var(func) => 
                        self.call_with(func, &vec![], Some(piped)),
                    _ => Err("right side of chain must be a function call or variable".into())
                }
            }

            Expr::Cmp { op, lhs, rhs } => {
                let a = self.number(lhs, "comparison")?;
                let b = self.number(rhs, "comparison")?;
                let truth = match op {
                    CmpOp::Lt => a < b,   CmpOp::Le => a <= b,
                    CmpOp::Gt => a > b,   CmpOp::Ge => a >= b,
                    CmpOp::Eq => a == b,  CmpOp::Ne => a != b,
                };
                Ok(Value::Number(if truth { 1.0 } else { 0.0 }))
            }
            
            Expr::Div { lhs, rhs } =>
                self.binop(NodeKind::Div, |a, b| a / b, lhs, rhs),

            Expr::For { var, iter, body, length } => {
                let items = match self.expr(iter)? {
                    Value::List(items) => items,
                    _ => return Err(format!(
                        "for {}: expected a list or range to iterate over", var.0)),
                };
                if items.is_empty() {
                    return Err(format!("for {}: nothing to iterate over (empty)", var.0));
                }

                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    self.env.push_scope();
                    self.env.define(&var.0, item.value.clone());
                    // The length is read inside the loop's scope, like the body
                    // it belongs to, so a step may be as long as the element it
                    // was built from: `for i in 1..=4 { f4;i }` is four notes
                    // each longer than the last.
                    let iteration = self.expr(body).and_then(|value| match length {
                        None => Ok(Item::plain(value)),
                        Some(e) => self.length(e).map(|l| Item { value, length: Some(l) }),
                    });
                    self.env.pop_scope();
                    out.push(iteration?);
                }

                // A `;` describes a step of a sequence, so it means something
                // only where the loop is collecting one. The two other things a
                // loop can be are settled below by what the body produced, and
                // neither of them has steps: voices are summed and plays are
                // scheduled, and a length would be silently dropped by both.
                if length.is_some() {
                    if let Some(what) = out.iter().find_map(|item| match item.value {
                        Value::Play { .. } => Some("plays"),
                        Value::Signal(_) => Some("audio"),
                        _ => None,
                    }) {
                        return Err(format!(
                            "for {}: a `;` length is how long a step lasts, and this loop \
                             answers with {what} rather than a sequence of steps", var.0));
                    }
                }

                // What the body produced decides what the loop is. A loop over
                // oscillators is voices to be heard at once, so it sums; a loop
                // over values is a list being built, so it collects. Deciding
                // from the values rather than from a keyword is what lets one
                // `for` be both without either having to be spelled specially.
                if out.iter().any(|item| matches!(item.value, Value::Play { .. })) {
                    // Plays are neither summed nor collected: they all happen,
                    // and the loop as a whole finishes when the last does.
                    let mut ends_at = Some(self.play_start);
                    let mut first = usize::MAX;
                    let mut last = 0usize;
                    for item in &out {
                        let Value::Play { ends_at: end, first: f, last: l, .. } = &item.value else {
                            return Err(format!(
                                "for {}: a loop cannot mix plays with other values", var.0));
                        };
                        // One that never stops makes the whole loop never stop,
                        // so nothing may follow it.
                        ends_at = crate::lowerer::play::later_end(ends_at, *end);
                        first = first.min(*f);
                        last = last.max(*l);
                    }
                    return Ok(Value::Play {
                        starts_at: self.play_start,
                        ends_at,
                        first: first.min(last),
                        last,
                        // Like `play_all`: the passes are the loop's own, so
                        // the chain begins here.
                        chain_first: first.min(last),
                        // Like `play_all`: a loop over plays has no one
                        // instrument, unless it went round exactly once.
                        template: (last == first + 1).then_some(first),
                    });
                }

                if out.iter().any(|item| matches!(item.value, Value::Signal(_))) {
                    // Any signal at all makes the loop audio: a number among
                    // them is a constant to be added, which is what `combine`
                    // already does.
                    let mut acc: Option<Value> = None;
                    for item in out {
                        acc = Some(match acc {
                            None => item.value,
                            Some(prev) =>
                                self.combine(NodeKind::Add, |a, b| a + b, prev, item.value)?,
                        });
                    }
                    return Ok(acc.expect("non-empty list yields at least one value"));
                }

                Ok(Value::List(Rc::new(out)))
            }

            Expr::If { cond, then, otherwise } => {
                if self.number(cond, "if condition")? != 0.0 {
                    self.expr(then)
                } else {
                    match otherwise {
                        Some(e) => self.expr(e),
                        None => Ok(Value::Number(0.0)),
                    }
                }
            }

            Expr::Index { base, index } => {
                let items = match self.expr(base)? {
                    Value::List(items) => items,
                    _ => return Err("cannot index a value that is not a list".into()),
                };
                let i = self.number(index, "list index")?;
                if i < 0.0 || i.fract() != 0.0 {
                    return Err(format!("list index must be a whole number >= 0, got {i}"));
                }
                items.get(i as usize).map(|it| it.value.clone()).ok_or_else(|| format!(
                    "list index {i} out of bounds (length {})", items.len()))
            }            
            
            // `value` is evaluated before the scope is pushed, so a binding
            // never sees itself: `let a = a * 2 in ...` reads the outer `a`.
            Expr::Let { name, value, body } => {
                let v = self.expr(value)?;
                self.env.push_scope();
                self.env.define(&name.0, v);
                let result = self.expr(body);
                self.env.pop_scope();
                result
            }

            // A `;` length is folded here, alongside the element it belongs to,
            // and checked for being something a length may be at all. Which of
            // the two kinds it turned out to be is carried rather than resolved:
            // what a length goes on to *mean* is not this layer's business — a
            // pattern reads a share as time and a written value as beats, a lane
            // reads either as a count of notes, and `len` reads neither.
            Expr::List(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for item in items.iter() {
                    let value = self.expr(&item.value)?;
                    let length = match &item.length {
                        None => None,
                        Some(e) => Some(self.length(e)?),
                    };
                    vals.push(Item { value, length });
                }
                Ok(Value::List(Rc::new(vals)))
            }
            
            Expr::Mul { lhs, rhs } =>
                self.binop(NodeKind::Mul, |a, b| a * b, lhs, rhs),
            
            Expr::Neg { expr } => match self.expr(expr)? {
                Value::Number(n) => Ok(Value::Number(-n)),
                v => {
                    let input = self.as_input(v)?;
                    Ok(Value::Signal(self.push_node(NodeKind::Neg, vec![input])))
                }
            }
            
            Expr::Num(n) => Ok(Value::Number(*n)),

            // `load` takes its path syntactically — see `lowerer::sample` — so
            // a string that reaches here is one written somewhere a string
            // cannot go.
            Expr::Str(s) => Err(format!(
                "a string is only meaningful as the path in `{}(\"...\")`, and \"{s}\" \
                 is somewhere else", crate::samples::LOAD)),

            Expr::Rest => Ok(Value::Rest),

            Expr::Trigger => Ok(Value::Trigger),

            Expr::Range { lo, hi } => {
                let lo = self.number(lo, "range start")?;
                let hi = self.number(hi, "range end")?;
                if lo.fract() != 0.0 || hi.fract() != 0.0 {
                    return Err(format!("range bounds must be whole numbers, got {lo}..={hi}"));
                }
                let count = if hi < lo { 0 } else { (hi - lo + 1.0) as usize };
                if count > MAX_UNROLL {
                    return Err(format!(
                        "range {lo}..={hi} expands to {count} items (limit {MAX_UNROLL})"));
                }
                let mut out = Vec::with_capacity(count);
                let mut i = lo;
                while i <= hi { out.push(Value::Number(i)); i += 1.0; }
                Ok(Value::List(Item::all(out)))
            }

            Expr::Rem { lhs, rhs } => {
                let a = self.number(lhs, "%")?;
                let b = self.number(rhs, "%")?;
                Ok(Value::Number(a % b))
            }
            
            Expr::Sub { lhs, rhs } =>
                self.binop(NodeKind::Sub, |a, b| a - b, lhs, rhs),

            // A name the environment does not know may still be a note: `c4`,
            // `as3`, `af1`. Bindings win, so a user `let` or parameter shadows
            // a note name rather than colliding with it.
            //
            // Written values and the tuplet marker resolve the same way and for
            // the same reason. They cannot collide with note names — those
            // require an octave digit — so the order between them is free, and
            // a parameter named `e` still shadows the eighth as it always did.
            Expr::Var(id) => match self.env.lookup(&id.0) {
                Some(v) => Ok(v),
                None if id.0 == crate::lang::TUPLET => Ok(Value::Tuplet),
                None if crate::lang::duration(&id.0).is_some() => Ok(Value::Duration(
                    crate::lang::duration(&id.0).expect("just matched"),
                )),
                None => match crate::lang::note(&id.0) {
                    crate::lang::NoteName::Note(n) => Ok(Value::Number(n)),
                    crate::lang::NoteName::OctaveOutOfRange(octave) => Err(format!(
                        "note {} has octave {octave}, outside {}..={}",
                        id.0,
                        crate::lang::MIN_NOTE_OCTAVE,
                        crate::lang::MAX_NOTE_OCTAVE
                    )),
                    crate::lang::NoteName::NotANote => {
                        Err(format!("unbound name: {}", id.0))
                    }
                },
            }

        }
    }

    fn binop(&mut self, kind: NodeKind, fold: fn(f64, f64) -> f64,
             lhs: &Expr, rhs: &Expr) -> Result<Value, String> {
        let l = self.expr(lhs)?;
        let r = self.expr(rhs)?;

        self.combine(kind, fold, l, r)
    }

    pub fn combine(&mut self, kind: NodeKind, fold: fn(f64, f64) -> f64,
               l: Value, r: Value) -> Result<Value, String> {

        match (l, r) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(fold(a, b))),
            // A tie: `h + e` is a half held into an eighth. Only addition — the
            // rest of the arithmetic has no reading on a written value, and
            // subtracting one from another is not something notation can draw.
            (Value::Duration(a), Value::Duration(b)) if kind == NodeKind::Add => {
                Ok(Value::Duration(a.add(b)))
            }
            (Value::Duration(a), Value::Duration(b)) => Err(format!(
                "written note values can only be added, which ties them — `{a} + {b}` \
                 is one held into the other. There is no other arithmetic notation \
                 can draw on them")),
            (l, r) => {
                let inputs = vec![self.as_input(l)?, self.as_input(r)?];
                Ok(Value::Signal(self.push_node(kind, inputs)))
            }
        }
    }

    /// What a `;` gave the step in front of it.
    ///
    /// One reading for both places a length can be written — an element of a
    /// list, and the body of a `for` collecting one — so `[f4;e]` and
    /// `for i in 0..=0 { f4;e }` cannot drift apart in what `e` means there.
    fn length(&mut self, e: &Expr) -> Result<Length, String> {
        match self.expr(e)? {
            Value::Number(n) => {
                if !n.is_finite() || n <= 0.0 {
                    return Err(format!("a `;` length must be a positive number, got {n}"));
                }
                Ok(Length::Ratio(n))
            }
            Value::Duration(b) => Ok(Length::Beats(b)),
            Value::Tuplet => Ok(Length::Tuplet),
            _ => Err("a `;` length needs a compile-time number or a written note value, \
                      got a signal".to_string()),
        }
    }

    fn number(&mut self, e: &Expr, what: &str) -> Result<f64, String> {
    match self.expr(e)? {
        Value::Number(n) => Ok(n),
        _ => Err(format!(
            "{what} needs a compile-time number, got a signal \
             (use select(gate, a, b) to choose at audio rate)")),
    }
}

    pub fn as_input(&self, v: Value) -> Result<NodeInput, String> {
        match v {
            Value::Number(n) => Ok(NodeInput::Const(n)),
            Value::Signal(id) => Ok(NodeInput::Node(id)),
            Value::Function(_) => Err("cannot use a function as a signal".into()),
            Value::List(_) => Err("cannot use a list as a signal (iterate it with `for`)".into()),
            Value::Stack(_) => Err(
                "cannot use a stack as a signal (a stack is layered patterns, not audio — \
                 sum oscillators with `+` to hear them at once)".into()),
            Value::Buffer(_) => Err(
                "cannot use a buffer as a signal — read it at a position with \
                 `sample(buffer, position)`".into()),
            Value::Rest => Err("cannot use a rest as a signal (rests belong in patterns)".into()),
            Value::Trigger => Err("cannot use a trigger as a signal (triggers belong in patterns)".into()),
            Value::Duration(b) => Err(format!(
                "cannot use `{b}` as a signal — a written note value says how long a \
                 step lasts, and only means anything inside a pattern")),
            Value::Tuplet => Err(
                "cannot use `t` as a signal — it marks a group inside a pattern as a \
                 tuplet".into()),
            Value::Play { .. } => Err(
                "cannot use a play as a signal (it schedules notes, it is not audio)".into()),
            Value::Rate(_) => Err(
                "cannot use a rate as a signal — `accel` says how fast a pattern runs, \
                 and is only meaningful as `play`'s rate".into()),
        }
    }
}