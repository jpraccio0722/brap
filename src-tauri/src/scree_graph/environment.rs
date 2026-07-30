use std::collections::HashMap;
use std::rc::Rc;
use crate::scree_graph::ugen_nodes::NodeId;
use crate::parser::parser::{Expr, Param};

#[derive(Clone)]
pub enum Value {
    Number(f64),
    Signal(NodeId),
    Function(Rc<FunctionDef>),
    List(Rc<Vec<Item>>),
    /// Patterns that sound at once rather than in turn — what `stack` builds.
    ///
    /// A variant of its own because nothing about the shape of a list says
    /// whether it is a sequence or a chord: `[[a, b], [c, d]]` is already two
    /// groups played one after the other, so layering needs a mark rather than
    /// a nesting. Only patterns read it; everywhere else it is an error, since
    /// there is no sensible `len` or index of two things happening together.
    Stack(Rc<Vec<Value>>),
    /// A silent step. Only meaningful inside a pattern.
    Rest,
    /// A sounding step carrying no value. Only meaningful inside a pattern.
    Trigger,
    /// What a `play` call evaluates to: a handle onto the binding it made.
    ///
    /// `ends_at` is the cycle, counted from the pattern origin, at which the
    /// binding falls silent — `None` for plain `play`, which never does. It is
    /// what `.then` needs to know when to start what follows.
    Play { ends_at: Option<f64> },
}

/// One element of a list, and the length `;` gave it.
///
/// The length rides along on the element instead of being desugared into
/// repeated copies, because the two are not the same thing: `[c4;3, e4]` is one
/// note held for three quarters of a cycle, and `[c4, c4, c4, e4]` is three
/// strikes. Only whoever reads the list knows which of those it wants — a
/// pattern sustains, a lane repeats, and `len` ignores it entirely and counts
/// the elements that were written.
#[derive(Clone)]
pub struct Item {
    pub value: Value,
    pub length: Option<f64>,
}

impl Item {
    /// An element with no length — every element of a list that predates `;`,
    /// and every one built by a builtin rather than written down.
    pub fn plain(value: Value) -> Item {
        Item { value, length: None }
    }

    /// The list a builtin returns, where lengths never apply.
    pub fn all(values: impl IntoIterator<Item = Value>) -> Rc<Vec<Item>> {
        Rc::new(values.into_iter().map(Item::plain).collect())
    }
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