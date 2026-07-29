use std::rc::Rc;
use crate::scree_graph::environment::{Env, FunctionDef, Value};
use crate::scree_graph::graph::ScreeGraph;
use crate::scree_graph::ugen_nodes::{NodeId, NodeInput, NodeKind, UGenNode};
use crate::parser::parser::ScreeItem;
use crate::pattern::patterns::Binding;

pub struct Lowerer {
    pub env: Env,
    pub graph: ScreeGraph,
    pub depth: usize,
    pub bindings: Vec<Binding>,
    /// Cycles after the origin that the next `play` should start at.
    ///
    /// Zero at the top level; `.then` raises it while inlining the function it
    /// was handed, which is the whole of "start when the last one finished".
    pub play_start: f64,
    /// Seeded per eval, so `choice` and `scramble` differ each time.
    pub rng: u64,
}

/// One eval produces two artifacts: the persistent graph, which is crossfaded
/// into the engine's slot, and the pattern bindings, which go to the scheduler.
pub struct Lowered {
    pub graph: ScreeGraph,
    pub bindings: Vec<Binding>,
}

/// A nonzero seed that changes between evals.
fn seed_from_clock() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1
}

pub fn lower(items: &Vec<ScreeItem>) -> Result<Lowered, String> {
    lower_inner(items, None)
}

/// Lower a single scheduler voice.
///
/// `dur` (the note length in seconds) is pre-bound as an ordinary number, so an
/// instrument can shape itself against the note it is playing — `env(a, d, s,
/// r, dur)`. Outside a voice the name is simply unbound.
pub fn lower_voice(items: &Vec<ScreeItem>, dur: f64) -> Result<Lowered, String> {
    lower_inner(items, Some(dur))
}

fn lower_inner(items: &Vec<ScreeItem>, dur: Option<f64>) -> Result<Lowered, String> {
    let mut lw = Lowerer {
        env: Env::new(),
        graph: ScreeGraph::default(),
        depth: 0,
        bindings: Vec::new(),
        play_start: 0.0,
        rng: seed_from_clock(),
    };

    if let Some(dur) = dur {
        lw.env.define("dur", Value::Number(dur));
    }

    for item in items {
        lw.item(item)?;
    }

    Ok(Lowered { graph: lw.graph, bindings: lw.bindings })
}

/// Lower a program that is only expected to produce a graph.
#[cfg(test)]
pub fn lower_graph(items: &Vec<ScreeItem>) -> Result<ScreeGraph, String> {
    lower(items).map(|l| l.graph)
}

impl Lowerer {

    fn item(&mut self, item: &ScreeItem) -> Result<(), String> {
        match item {
            ScreeItem::Function { name, params, body } => {
                self.env.define(&name.0.as_str(), Value::Function(
                    Rc::new(
                        FunctionDef { params: params.to_vec(), body: body.clone() }
                    )
                ));
                
                Ok(())
            }

            ScreeItem::Let { name, value } => {
                let v = self.expr(value)?;
                self.env.define(&name.0.as_str(), v);
                Ok(())
            }

            ScreeItem::Expr(e) => {
                let v = self.expr(e)?;
                if let Value::Signal(id) = v {
                    self.add_to_output(id);
                }
                Ok(())
            }

            ScreeItem::Call { func, args } => {
                let v = self.call(func, args)?;
                if let Value::Signal(id) = v {
                    self.add_to_output(id);
                }
                Ok(())
            }


            _ => Ok (())
        }
    }

    pub fn push_node(&mut self, kind: NodeKind, inputs: Vec<NodeInput>) -> NodeId {
        self.graph.nodes.push(UGenNode { kind, inputs, span: None });
        NodeId(self.graph.nodes.len() - 1)
    }

    pub fn add_to_output(&mut self, id: NodeId) {
        self.graph.output = Some(match self.graph.output {
            None => id,
            Some(prev) => self
                .push_node(NodeKind::Add, vec![NodeInput::Node(prev), NodeInput::Node(id)])
        })
    }
}