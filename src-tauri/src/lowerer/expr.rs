use std::fmt::format;
use std::rc::Rc;

use crate::scree_graph::environment::{Env, Value};
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

            Expr::For { var, iter, body } => {
                let items = match self.expr(iter)? {
                    Value::List(items) => items,
                    _ => return Err(format!(
                        "for {}: expected a list or range to iterate over", var.0)),
                };
                if items.is_empty() {
                    return Err(format!("for {}: nothing to iterate over (empty)", var.0));
                }

                let mut acc: Option<Value> = None;
                for item in items.iter() {
                    self.env.push_scope();
                    self.env.define(&var.0, item.clone());
                    let iteration = self.expr(body);
                    self.env.pop_scope();

                    let v = iteration?;
                    acc = Some(match acc {
                        None => v,
                        // Several plays in one loop are not summed like audio:
                        // they all happen, and the loop as a whole finishes
                        // when the last of them does.
                        Some(Value::Play { ends_at: a }) => {
                            let Value::Play { ends_at: b } = v else {
                                return Err(format!(
                                    "for {}: a loop cannot mix plays with other values",
                                    var.0));
                            };
                            // One that never stops makes the whole loop never
                            // stop, so nothing may follow it.
                            Value::Play { ends_at: crate::lowerer::play::later_end(a, b) }
                        }
                        Some(prev) => self.combine(NodeKind::Add, |a, b| a + b, prev, v)?,
                    });
                }
                Ok(acc.expect("non-empty list yields at least one value"))
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
                items.get(i as usize).cloned().ok_or_else(|| format!(
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

            Expr::List(items) => {
                let vals = items.iter()
                    .map(|e| self.expr(e))
                    .collect::<Result<Vec<_>, _>>()?;
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
                Ok(Value::List(Rc::new(out)))
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
            Expr::Var(id) => match self.env.lookup(&id.0) {
                Some(v) => Ok(v),
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
            (l, r) => {
                let inputs = vec![self.as_input(l)?, self.as_input(r)?];
                Ok(Value::Signal(self.push_node(kind, inputs)))
            }
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
            Value::Rest => Err("cannot use a rest as a signal (rests belong in patterns)".into()),
            Value::Trigger => Err("cannot use a trigger as a signal (triggers belong in patterns)".into()),
            Value::Play { .. } => Err(
                "cannot use a play as a signal (it schedules notes, it is not audio)".into()),
        }
    }
}