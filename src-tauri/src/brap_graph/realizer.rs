use fundsp::prelude64::*;

use crate::brap_graph::{graph::BrapGraph, ugen_nodes::NodeInput};


pub fn realize(graph: &BrapGraph) -> Net {
    let mut net = Net::new(0, 1);
    let mut ids: Vec<fundsp::net::NodeId> = Vec::with_capacity(graph.nodes.len());

    for n in &graph.nodes {
        let unit: Box<dyn AudioUnit> = match n.kind {
            super::ugen_nodes::NodeKind::Add => Box::new(pass() + pass()),
            super::ugen_nodes::NodeKind::Chain => todo!(),
            super::ugen_nodes::NodeKind::Div => Box::new(map(|i: &Frame<f32, U2>| i[0] / i[1])),
            super::ugen_nodes::NodeKind::Mul => Box::new(pass() * pass()),
            super::ugen_nodes::NodeKind::Neg => Box::new(-pass()),
            super::ugen_nodes::NodeKind::Sin => Box::new(sine()),
            super::ugen_nodes::NodeKind::Saw => Box::new(saw()),
            super::ugen_nodes::NodeKind::Sub => Box::new(pass() - pass()),
        };

        let fid = net.push(unit);

        for (port, input) in n.inputs.iter().enumerate() {

            match input {
                NodeInput::Node(id) => net.connect(ids[id.0], 0, fid, port),
                NodeInput::Const(v) => { 
                    let c = net.push(Box::new(dc(*v as f32)));
                    net.connect(c, 0, fid, port);
                }
            }
        }

        ids.push(fid);
    }

    if let Some(out) = graph.output {
        net.pipe_output(ids[out.0]);
    }

    net
}