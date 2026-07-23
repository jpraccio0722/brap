use crate::{brap_graph::{environment::Value, ugen_nodes::NodeKind}, lowerer::lower::Lowerer, parser::parser::{Expr, Ident}};



impl Lowerer {
    pub fn call(&mut self, func: &Ident, args: &[Expr]) -> Result<Value, String> {
        self.call_with(func, args, None)
    }

    pub fn call_with(&mut self, func: &Ident, args: &[Expr], piped: Option<Value>) -> Result<Value, String> {
        let mut arg_vals: Vec<Value> = Vec::with_capacity(args.len() + 1);

        if let Some(p) = piped {
            arg_vals.push(p);
        }
        
        for arg in args {
            arg_vals.push(self.expr(arg)?);
        }

        if let Some((kind, arity)) = self.builtin(&func.0) {
            if arg_vals.len() != arity {
               return Err(format!("{} expects {} inputs, got {}",
                                   func.0, arity, arg_vals.len()));
            }
            let inputs = arg_vals.into_iter()
                .map(|v| self.as_input(v))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::Signal(self.push_node(kind, inputs)));
        } 

        let Some(Value::Function(def)) = self.env.lookup(&func.0) else {
            return Err(format!("{} is not a function", func.0));
        };

        if arg_vals.len() > def.params.len() {
            return Err(format!("{} expects at most {} args, got {}",
                               func.0, def.params.len(), arg_vals.len()));
        }
        if self.depth >= 64 {
            return Err(format!("call depth exceeded inlining {} (recursive fn?)", func.0));
        }

        self.depth += 1;
        self.env.push_scope();
        let result = (|| {
            for (i, param) in def.params.iter().enumerate() {
                let v = match (arg_vals.get(i), &param.default) {
                    (Some(v), _) => v.clone(),
                    (None, Some(d)) => self.expr(d)?,
                    (None, None) => return Err(format!(
                        "{}: missing argument '{}'", func.0, param.name.0)),
                };
                self.env.define(&param.name.0, v);
            }
            self.expr(&def.body)
        })();
        self.env.pop_scope();
        self.depth -= 1;
        result
        
    }

    fn builtin(&self, func: &str) -> Option<(NodeKind, usize)> {
        match func {
            "sin" => Some((NodeKind::Sin, 1)),
            "saw" => Some((NodeKind::Saw, 1)),
            _ => None
        }
    }
}