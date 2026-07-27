use crate::scree_graph::ugen_nodes::{NodeId, UGenNode};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScreeGraph {
    pub nodes: Vec<UGenNode>,
    pub output: Option<NodeId>
}