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

/// `for i in 1..=3 { sin(i * 110) }` unrolls into three oscillators, summed
/// left to right — the same graph `a + b + c` would produce. The Add nodes
/// interleave with the oscillators because the sum folds as it goes.
#[test]
fn for_loop_unrolls_and_sums() {
    let g = lower_src("for i in 1..=3 { sin(i * 110) }\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(110.0)]),
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
        node(NodeKind::Sin, vec![Const(330.0)]),
        node(NodeKind::Add, vec![Node(NodeId(2)), Node(NodeId(3))]),
    ]);
    assert_eq!(g.output, Some(NodeId(4)));
}

/// A `for` is an expression, so it composes with chains like any other signal.
#[test]
fn for_loop_is_an_expression() {
    let g = lower_src("for i in 1..=2 { sin(i * 100) } >> lowpass(800, 1)\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(100.0)]),
        node(NodeKind::Sin, vec![Const(200.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
        node(NodeKind::Lowpass, vec![Node(NodeId(2)), Const(800.0), Const(1.0)]),
    ]);
    assert_eq!(g.output, Some(NodeId(3)));
}

/// The loop variable is a compile-time number, so a body with no signal in it
/// folds all the way down to a constant: 1 + 2 + 3 + 4 = 10.
#[test]
fn for_loop_over_constants_folds() {
    let g = lower_src("sin(for i in 1..=4 { i })\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(10.0)])]);
}

/// Each iteration gets its own scope; the loop variable does not outlive it.
#[test]
fn loop_variable_does_not_leak() {
    let err = lower_src("for i in 1..=2 { sin(i) }\ni\n").unwrap_err();
    assert!(err.contains("unbound name: i"), "got: {err}");
}

/// A block body binds locals in its own scope and returns its tail expression.
#[test]
fn block_body_binds_locals() {
    let g = lower_src("for i in 1..=2 {\n  let f = i * 110\n  sin(f)\n}\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(110.0)]),
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
    ]);
    assert_eq!(g.output, Some(NodeId(2)));
}

/// A user function called from the loop body inlines once per iteration.
#[test]
fn for_loop_calls_user_function_per_iteration() {
    let g = lower_src("fn voice(f) = sin(f) / 4\nfor i in 1..=2 { voice(i * 200) }\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(200.0)]),
        node(NodeKind::Div, vec![Node(NodeId(0)), Const(4.0)]),
        node(NodeKind::Sin, vec![Const(400.0)]),
        node(NodeKind::Div, vec![Node(NodeId(2)), Const(4.0)]),
        node(NodeKind::Add, vec![Node(NodeId(1)), Node(NodeId(3))]),
    ]);
    assert_eq!(g.output, Some(NodeId(4)));
}

/// Nested loops unroll to the product of their ranges.
#[test]
fn nested_for_loops_unroll() {
    let g = lower_src("for i in 1..=2 { for j in 1..=2 { sin(i * 100 + j) } }\n").unwrap();
    let sines: Vec<_> = g.nodes.iter()
        .filter(|n| n.kind == NodeKind::Sin)
        .map(|n| n.inputs.clone())
        .collect();
    assert_eq!(sines, vec![
        vec![Const(101.0)], vec![Const(102.0)],
        vec![Const(201.0)], vec![Const(202.0)],
    ]);
}

#[test]
fn empty_range_is_an_error() {
    let err = lower_src("for i in 3..=1 { sin(i) }\n").unwrap_err();
    assert!(err.contains("empty"), "got: {err}");
}

#[test]
fn runaway_unroll_is_an_error() {
    let err = lower_src("for i in 1..=100000 { sin(i) }\n").unwrap_err();
    assert!(err.contains("limit"), "got: {err}");
}
