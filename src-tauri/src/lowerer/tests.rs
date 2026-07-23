use crate::brap_graph::graph::BrapGraph;
use crate::brap_graph::ugen_nodes::{NodeId, NodeInput, NodeKind, UGenNode};
use crate::lowerer::lower::lower;
use crate::parser::parser::parse;
use NodeInput::{Const, Node};

fn lower_src(src: &str) -> Result<BrapGraph, String> {
    let items = parse(src.to_string()).expect("parse failed");
    lower(&items)
}

fn node(kind: NodeKind, inputs: Vec<NodeInput>) -> UGenNode {
    UGenNode { kind, inputs, span: None }
}

/// `sin(220 * 2)` — arithmetic on numbers folds during lowering; no Mul node exists.
#[test]
fn constant_folding_inside_call() {
    let g = lower_src("sin(220 * 2)\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(440.0)])]);
    assert_eq!(g.output, Some(NodeId(0)));
}

/// `let a = sin(2); a * a` — one oscillator, referenced twice. Sharing, not duplication.
#[test]
fn let_binding_shares_one_node() {
    let g = lower_src("let a = sin(2)\na * a\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(2.0)]),
        node(NodeKind::Mul, vec![Node(NodeId(0)), Node(NodeId(0))]),
    ]);
    assert_eq!(g.output, Some(NodeId(1)));
}

/// Each call site of a user fn expands to its own subgraph.
#[test]
fn function_calls_inline_separately() {
    let g = lower_src("fn voice(f) = sin(f) / 5\nvoice(220) + voice(330)\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Div, vec![Node(NodeId(0)), Const(5.0)]),
        node(NodeKind::Sin, vec![Const(330.0)]),
        node(NodeKind::Div, vec![Node(NodeId(2)), Const(5.0)]),
        node(NodeKind::Add, vec![Node(NodeId(1)), Node(NodeId(3))]),
    ]);
    assert_eq!(g.output, Some(NodeId(4)));
}

/// Two top-level signal expressions get summed into a single output.
#[test]
fn top_level_signals_sum_into_output() {
    let g = lower_src("sin(1)\nsin(2)\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(1.0)]),
        node(NodeKind::Sin, vec![Const(2.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
    ]);
    assert_eq!(g.output, Some(NodeId(2)));
}

/// `a >> f(b)` means `f(a, b)` — the piped value becomes the first argument.
#[test]
fn chain_pipes_lhs_as_first_argument() {
    let g = lower_src("fn gain(x, amt) = x * amt\nsin(440) >> gain(5)\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(440.0)]),
        node(NodeKind::Mul, vec![Node(NodeId(0)), Const(5.0)]),
    ]);
    assert_eq!(g.output, Some(NodeId(1)));
}

/// `a >> f` (bare identifier) is a zero-arg call receiving the piped value.
#[test]
fn chain_into_bare_identifier() {
    let g = lower_src("sin(4) >> saw\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(4.0)]),
        node(NodeKind::Saw, vec![Node(NodeId(0))]),
    ]);
    assert_eq!(g.output, Some(NodeId(1)));
}

/// A defaulted parameter fills in when the argument is omitted.
#[test]
fn default_param_fills_missing_argument() {
    let g = lower_src("fn v(f = 220) = sin(f)\nv()\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
    assert_eq!(g.output, Some(NodeId(0)));
}

#[test]
fn unbound_name_is_an_error() {
    let err = lower_src("boo * 2\n").unwrap_err();
    assert!(err.contains("unbound name: boo"), "got: {err}");
}

#[test]
fn recursive_function_hits_depth_guard() {
    let err = lower_src("fn boom(x) = boom(x)\nboom(1)\n").unwrap_err();
    assert!(err.contains("depth"), "got: {err}");
}

#[test]
fn function_used_as_signal_is_an_error() {
    let err = lower_src("fn f(x) = x\nf + 1\n").unwrap_err();
    assert!(err.contains("function"), "got: {err}");
}
