//! Rewriting the names in one file so they can live beside every other file's.
//!
//! Expansion produces a single flat program, so two modules that both define
//! `kick` have to end up with two different names in it. Every definition a
//! module makes is filed under `<module>::<name>`, and every mention of it —
//! wherever it was written, and however it was written — is rewritten to that.
//!
//! The whole difficulty is knowing what is a mention. A parameter called `kick`
//! is not the imported `kick`, and neither is a `let` that shadows it, so this
//! walk carries the same scope stack the lowerer will: names bound by a `fn`'s
//! parameters, a `let`, a block's statements, or a `for` are left alone inside
//! the body they cover.

use std::collections::{HashMap, HashSet};

use crate::parser::parser::{Arg, Expr, Ident, ScreeItem, Statement};

/// What the names written in one file mean everywhere else.
pub struct Scope<'a> {
    /// Plain name → the name its definition is filed under in the program.
    /// Holds both what the file imported and what it defines itself.
    names: &'a HashMap<String, String>,
    /// The head of a qualified name → the prefix that module's definitions
    /// carry. `drums` → `lib::drums`, for `use lib::drums`.
    modules: &'a HashMap<String, String>,
    /// Names bound by the code being walked, innermost last. These shadow
    /// everything above: a parameter is not an import.
    locals: Vec<HashSet<String>>,
}

impl<'a> Scope<'a> {
    pub fn new(
        names: &'a HashMap<String, String>,
        modules: &'a HashMap<String, String>,
    ) -> Scope<'a> {
        Scope { names, modules, locals: Vec::new() }
    }

    fn push(&mut self, bound: impl IntoIterator<Item = String>) {
        self.locals.push(bound.into_iter().collect());
    }

    fn pop(&mut self) {
        self.locals.pop();
    }

    fn shadowed(&self, name: &str) -> bool {
        self.locals.iter().any(|scope| scope.contains(name))
    }

    /// What a written name refers to, or `None` when it refers to itself — a
    /// builtin, a parameter, a drawn pattern, or this file's own definition.
    fn resolve(&self, written: &str) -> Result<Option<String>, String> {
        // A qualified name can be neither shadowed nor local: nothing but a
        // `use` can put a `::` into scope.
        if let Some((head, rest)) = written.split_once("::") {
            let Some(prefix) = self.modules.get(head) else {
                return Err(format!(
                    "no module named `{head}` here — add `use {head}` at the top of \
                     the file, or drop the `{head}::` from `{written}`"
                ));
            };
            return Ok(Some(format!("{prefix}::{rest}")));
        }

        if self.shadowed(written) {
            return Ok(None);
        }

        Ok(self.names.get(written).cloned())
    }

    fn ident(&self, id: &mut Ident) -> Result<(), String> {
        if let Some(name) = self.resolve(&id.0)? {
            id.0 = name;
        }
        Ok(())
    }

    /// Rewrite one top-level item's body. The item's own name is the caller's
    /// business: only it knows whether this file is a module.
    pub fn item(&mut self, item: &mut ScreeItem) -> Result<(), String> {
        match item {
            ScreeItem::Function { params, body, .. } => {
                // Every parameter at once, ahead of the defaults: a default may
                // read an earlier parameter, and none of them is ever an import.
                self.push(params.iter().map(|p| p.name.0.clone()));
                let result = (|| {
                    for param in params.iter_mut() {
                        if let Some(default) = &mut param.default {
                            self.expr(default)?;
                        }
                    }
                    self.expr(body)
                })();
                self.pop();
                result
            }
            ScreeItem::Let { value, .. } => self.expr(value),
            ScreeItem::Expr(e) => self.expr(e),
            ScreeItem::Call { func, args } => {
                self.ident(func)?;
                self.args(args)
            }
            // Gone by the time anything is renamed: a `use` is what built the
            // tables this walk reads.
            ScreeItem::Use(_) => Ok(()),
        }
    }

    fn args(&mut self, args: &mut [Arg]) -> Result<(), String> {
        for arg in args {
            // Only the value. An argument's name is a parameter of the callee
            // — or a lane of an instrument — and belongs to whatever it names.
            self.expr(&mut arg.value)?;
        }
        Ok(())
    }

    fn expr(&mut self, expr: &mut Expr) -> Result<(), String> {
        match expr {
            Expr::Var(name) => self.ident(name),

            Expr::Call { func, args } => {
                self.ident(func)?;
                self.args(args)
            }

            Expr::Add { lhs, rhs }
            | Expr::Sub { lhs, rhs }
            | Expr::Mul { lhs, rhs }
            | Expr::Div { lhs, rhs }
            | Expr::Rem { lhs, rhs }
            | Expr::Chain { lhs, rhs }
            | Expr::Cmp { lhs, rhs, .. } => {
                self.expr(lhs)?;
                self.expr(rhs)
            }

            Expr::Range { lo, hi } => {
                self.expr(lo)?;
                self.expr(hi)
            }

            Expr::Index { base, index } => {
                self.expr(base)?;
                self.expr(index)
            }

            Expr::Neg { expr } => self.expr(expr),

            Expr::List(items) => {
                for item in items {
                    self.expr(&mut item.value)?;
                    // A length is an expression, so it may name an import too.
                    if let Some(length) = &mut item.length {
                        self.expr(length)?;
                    }
                }
                Ok(())
            }

            Expr::If { cond, then, otherwise } => {
                self.expr(cond)?;
                self.expr(then)?;
                match otherwise {
                    Some(otherwise) => self.expr(otherwise),
                    None => Ok(()),
                }
            }

            // The value is outside the binding, the body inside it: `let a = a`
            // reads the outer `a`, exactly as the lowerer evaluates it.
            Expr::Let { name, value, body } => {
                self.expr(value)?;
                self.push([name.0.clone()]);
                let result = self.expr(body);
                self.pop();
                result
            }

            Expr::For { var, iter, body } => {
                self.expr(iter)?;
                self.push([var.0.clone()]);
                let result = self.expr(body);
                self.pop();
                result
            }

            // A block's `let`s come into scope one at a time, so each one's
            // value still sees what was written above it and no more.
            Expr::Block { stmts, tail } => {
                self.push([]);
                let result = (|| {
                    for stmt in stmts.iter_mut() {
                        match stmt {
                            Statement::Expr(e) => self.expr(e)?,
                            Statement::Let { name, value } => {
                                self.expr(value)?;
                                let bound = name.0.clone();
                                self.locals
                                    .last_mut()
                                    .expect("just pushed")
                                    .insert(bound);
                            }
                        }
                    }
                    self.expr(tail)
                })();
                self.pop();
                result
            }

            Expr::Num(_) | Expr::Rest | Expr::Trigger => Ok(()),
        }
    }
}
