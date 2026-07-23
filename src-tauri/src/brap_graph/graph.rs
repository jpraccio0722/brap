use crate::brap_graph::ugen_nodes::{NodeId, UGenNode};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BrapGraph {
    pub nodes: Vec<UGenNode>,
    pub output: Option<NodeId>
}