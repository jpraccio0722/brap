use crate::brap_graph::environment::{Env, Value};
use crate::brap_graph::ugen_nodes::{NodeInput, NodeKind};
use crate::lowerer::lower::Lowerer;
use crate::parser::parser::Expr;

impl Lowerer {
    pub fn expr(&mut self, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Ad { lhs, rhs } =>
                self.binop(NodeKind::Add, |a, b| a + b, lhs, rhs),
            
            Expr::Block { stmts , tail } => Ok(Value::Number(0.0)), // TODO not implemented
            
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
            
            Expr::Div { lhs, rhs } =>
                self.binop(NodeKind::Div, |a, b| a / b, lhs, rhs),
            
            Expr::Let { name, value, body } => Ok(Value::Number(0.0)), // TODO not implemented
            
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
            
            Expr::Sub { lhs, rhs } =>
                self.binop(NodeKind::Sub, |a, b| a - b, lhs, rhs),

            Expr::Var(id) => self.env.lookup(&id.0).ok_or_else(|| format!("unbound name: {}", id.0))

        }
    }

    fn binop(&mut self, kind: NodeKind, fold: fn(f64, f64) -> f64,
             lhs: &Expr, rhs: &Expr) -> Result<Value, String> {
        let l = self.expr(lhs)?;
        let r = self.expr(rhs)?;

        match(l, r) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(fold(a, b))),
            (l, r) => {
                let inputs = vec![self.as_input(l)?, self.as_input(r)?];
                Ok(Value::Signal(self.push_node(kind, inputs)))
            }
        }
    }

    pub fn as_input(&self, v: Value) -> Result<NodeInput, String> {
        match v {
            Value::Number(n) => Ok(NodeInput::Const(n)),
            Value::Signal(id) => Ok(NodeInput::Node(id)),
            Value::Function(_) => Err("cannot use a function as a signal".into())
        }
    }
}