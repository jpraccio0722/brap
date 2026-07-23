use std::collections::HashMap;
use std::rc::Rc;
use crate::brap_graph::ugen_nodes::NodeId;
use crate::parser::parser::{Expr, Param};

#[derive(Clone)]
pub enum Value {
    Number(f64),
    Signal(NodeId),
    Function(Rc<FunctionDef>),
}

pub struct FunctionDef {
    pub params: Vec<Param>,
    pub body: Expr
}

pub struct Env {
    scopes: Vec<HashMap<String, Value>>
}

impl Env {
    pub fn new() -> Env {
        Env {
            scopes: vec![HashMap::new()]
        }
    }

    pub fn define(&mut self, name: &str, value: Value) {
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(name.to_string(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<Value> {
        self.scopes.iter().rev()
            .find_map(|scope| scope.get(name))
            .cloned()
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
        debug_assert!(!self.scopes.is_empty(), "popped the global scope");
    }
}