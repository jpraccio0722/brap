use chumsky::span::SimpleSpan;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);


#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Add, Chain, Div,
    Mul, Neg, Sin,
    Saw, Sub,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeInput {
    Const(f64),
    Node(NodeId)
}

#[derive(Clone, Debug, PartialEq)]
pub struct UGenNode {
    pub kind: NodeKind,
    pub inputs: Vec<NodeInput>,
    pub span: Option<SimpleSpan>
}