use std::rc::Rc;
use crate::brap_graph::environment::{Env, FunctionDef, Value};
use crate::brap_graph::graph::BrapGraph;
use crate::brap_graph::ugen_nodes::{NodeId, NodeInput, NodeKind, UGenNode};
use crate::parser::parser::BrapItem;
use crate::pattern::patterns::Binding;

pub struct Lowerer {
    pub env: Env,
    pub graph: BrapGraph,
    pub depth: usize,
    pub bindings: Vec<Binding>,
    /// Seeded per eval, so `choice` and `scramble` differ each time.
    pub rng: u64,
}

/// One eval produces two artifacts: the persistent graph, which is crossfaded
/// into the engine's slot, and the pattern bindings, which go to the scheduler.
pub struct Lowered {
    pub graph: BrapGraph,
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

pub fn lower(items: &Vec<BrapItem>) -> Result<Lowered, String> {
    let mut lw = Lowerer {
        env: Env::new(),
        graph: BrapGraph::default(),
        depth: 0,
        bindings: Vec::new(),
        rng: seed_from_clock(),
    };

    for item in items {
        lw.item(item)?;
    }

    Ok(Lowered { graph: lw.graph, bindings: lw.bindings })
}

/// Lower a program that is only expected to produce a graph.
#[cfg(test)]
pub fn lower_graph(items: &Vec<BrapItem>) -> Result<BrapGraph, String> {
    lower(items).map(|l| l.graph)
}

impl Lowerer {

    fn item(&mut self, item: &BrapItem) -> Result<(), String> {
        match item {
            BrapItem::Function { name, params, body } => {
                self.env.define(&name.0.as_str(), Value::Function(
                    Rc::new(
                        FunctionDef { params: params.to_vec(), body: body.clone() }
                    )
                ));
                
                Ok(())
            }

            BrapItem::Let { name, value } => {
                let v = self.expr(value)?;
                self.env.define(&name.0.as_str(), v);
                Ok(())
            }

            BrapItem::Expr(e) => {
                let v = self.expr(e)?;
                if let Value::Signal(id) = v {
                    self.add_to_output(id);
                }
                Ok(())
            }

            BrapItem::Call { func, args } => {
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