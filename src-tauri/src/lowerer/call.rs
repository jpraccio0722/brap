use std::rc::Rc;

use crate::{brap_graph::{environment::Value, ugen_nodes::NodeKind}, lowerer::lower::Lowerer, parser::parser::{Expr, Ident}};


impl Lowerer {
    pub fn call(&mut self, func: &Ident, args: &[Expr]) -> Result<Value, String> {
        self.call_with(func, args, None)
    }

    pub fn call_with(&mut self, func: &Ident, args: &[Expr], piped: Option<Value>) -> Result<Value, String> {
        // Before evaluation: play needs the instrument argument syntactically.
        if Lowerer::is_play(&func.0) {
            return self.play(args, piped);
        }

        let mut arg_vals: Vec<Value> = Vec::with_capacity(args.len() + 1);

        if let Some(p) = piped {
            arg_vals.push(p);
        }
        
        for arg in args {
            arg_vals.push(self.expr(arg)?);
        }

        if let Some(v) = self.list_builtin(&func.0, &arg_vals)? {
            return Ok(v);
        }

        if let Some(v) = self.math_builtin(&func.0, &arg_vals)? {
            return Ok(v);
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

        self.apply(&func.0, def, arg_vals)
    }

    /// Inline a user function against already-evaluated arguments.
    ///
    /// Split out of `call_with` so list builtins that take a function — `filter`
    /// — can invoke one without having the original `Expr`s.
    pub fn apply(
        &mut self,
        name: &str,
        def: Rc<crate::brap_graph::environment::FunctionDef>,
        arg_vals: Vec<Value>,
    ) -> Result<Value, String> {
        if arg_vals.len() > def.params.len() {
            return Err(format!("{} expects at most {} args, got {}",
                               name, def.params.len(), arg_vals.len()));
        }
        if self.depth >= 64 {
            return Err(format!("call depth exceeded inlining {} (recursive fn?)", name));
        }

        self.depth += 1;
        self.env.push_scope();
        let result = (|| {
            for (i, param) in def.params.iter().enumerate() {
                let v = match (arg_vals.get(i), &param.default) {
                    (Some(v), _) => v.clone(),
                    (None, Some(d)) => self.expr(d)?,
                    (None, None) => return Err(format!(
                        "{}: missing argument '{}'", name, param.name.0)),
                };
                self.env.define(&param.name.0, v);
            }
            self.expr(&def.body)
        })();
        self.env.pop_scope();
        self.depth -= 1;
        result
    }

    /// Resolve a UGen name to its node kind and arity.
    ///
    /// Both come from the `lang::UGENS` table, which the editor also serves to
    /// the frontend for completion — so a UGen cannot be callable without also
    /// being completable, and its arity cannot disagree with its documented
    /// parameter list.
    fn builtin(&self, func: &str) -> Option<(NodeKind, usize)> {
        crate::lang::ugen(func).map(|u| (u.kind, u.params.len()))
    }
}

#[cfg(test)]
mod tests {
    use crate::lowerer::lower::lower;
    use crate::parser::parser::parse;

    fn lower_err(src: &str) -> String {
        let items = parse(src.to_string()).expect("parse failed");
        match lower(&items) {
            Err(e) => e,
            Ok(_) => panic!("expected {src} to fail lowering"),
        }
    }

    /// The arity message is derived from the table now; it must still name the
    /// same count the hand-written match arm enforced.
    #[test]
    fn wrong_arity_reports_the_expected_count() {
        assert_eq!(lower_err("lowpass(sin(220), 400)\n"), "lowpass expects 3 inputs, got 2");
        assert_eq!(lower_err("sin(220, 330)\n"), "sin expects 1 inputs, got 2");
        assert_eq!(lower_err("noise(1)\n"), "noise expects 0 inputs, got 1");
    }

    /// A name in neither table is still an ordinary unbound-function error.
    #[test]
    fn unknown_names_are_not_functions() {
        assert_eq!(lower_err("lowpas(sin(220), 400, 1)\n"), "lowpas is not a function");
    }

    /// Every UGen in the table really lowers, at its documented arity.
    #[test]
    fn every_ugen_lowers_at_its_documented_arity() {
        for u in crate::lang::UGENS {
            let args = vec!["1"; u.params.len()].join(", ");
            let src = format!("{}({})\n", u.name, args);
            let items = parse(src.clone()).unwrap_or_else(|e| panic!("{src} failed to parse: {e}"));
            assert!(lower(&items).is_ok(), "{src} failed to lower");
        }
    }
}
