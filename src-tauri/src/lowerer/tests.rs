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

/// A chain may break before the `>>`. The closing brace must not pick up a
/// statement terminator when the next line continues the expression.
#[test]
fn chain_continues_across_a_newline() {
    let g = lower_src("for i in 1..=2 { sin(i * 100) }\n  >> lowpass(800, 1)\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(100.0)]),
        node(NodeKind::Sin, vec![Const(200.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
        node(NodeKind::Lowpass, vec![Node(NodeId(2)), Const(800.0), Const(1.0)]),
    ]);
    assert_eq!(g.output, Some(NodeId(3)));
}

#[test]
fn if_true_lowers_only_the_then_branch() {
    let g = lower_src("if 1 { sin(220) } else { saw(220) }\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
    assert_eq!(g.output, Some(NodeId(0)));
}

/// The untaken branch emits nothing at all — not a node, not an orphan.
#[test]
fn if_false_lowers_only_the_else_branch() {
    let g = lower_src("if 0 { sin(220) } else { saw(220) }\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Saw, vec![Const(220.0)])]);
}

/// A missing `else` yields 0.0, the identity for the output sum.
#[test]
fn if_without_else_contributes_nothing_when_false() {
    let g = lower_src("if 0 { sin(220) }\n").unwrap();
    assert_eq!(g.nodes, vec![]);
    assert_eq!(g.output, None);
}

#[test]
fn else_if_chain() {
    let g = lower_src("if 0 { sin(1) } else if 1 { sin(2) } else { sin(3) }\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(2.0)])]);
}

#[test]
fn comparisons_fold_to_one_or_zero() {
    let g = lower_src("sin(1 < 2)\nsin(3 <= 2)\nsin(2 == 2)\nsin(2 != 2)\n").unwrap();
    let sines: Vec<_> = g.nodes.iter()
        .filter(|n| n.kind == NodeKind::Sin).map(|n| n.inputs.clone()).collect();
    assert_eq!(sines, vec![
        vec![Const(1.0)], vec![Const(0.0)], vec![Const(1.0)], vec![Const(0.0)],
    ]);
}

/// The real payoff: a branch chosen per iteration, resolved at lowering time.
#[test]
fn if_inside_for_selects_per_iteration() {
    let g = lower_src("for i in 1..=4 { if i % 2 == 0 { sin(i) } else { saw(i) } }\n").unwrap();
    let kinds: Vec<_> = g.nodes.iter()
        .filter(|n| n.kind != NodeKind::Add)
        .map(|n| (n.kind.clone(), n.inputs.clone())).collect();
    assert_eq!(kinds, vec![
        (NodeKind::Saw, vec![Const(1.0)]),
        (NodeKind::Sin, vec![Const(2.0)]),
        (NodeKind::Saw, vec![Const(3.0)]),
        (NodeKind::Sin, vec![Const(4.0)]),
    ]);
}

/// `else` on its own line must not pick up a statement terminator.
#[test]
fn multiline_else_parses() {
    let g = lower_src("if 0 {\n  sin(1)\n}\nelse {\n  sin(2)\n}\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(2.0)])]);
}

#[test]
fn comparison_precedence_below_arithmetic() {
    let g = lower_src("if 1 + 1 == 2 { sin(7) }\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(7.0)])]);
}

/// Adding `>` must not steal the pipeline operator.
#[test]
fn shift_right_still_beats_greater_than() {
    let g = lower_src("sin(4) >> saw\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(4.0)]),
        node(NodeKind::Saw, vec![Node(NodeId(0))]),
    ]);
}

#[test]
fn signal_condition_is_an_error() {
    let err = lower_src("if sin(2) { sin(1) }\n").unwrap_err();
    assert!(err.contains("compile-time number"), "got: {err}");
}

/// Audio-rate choice is arithmetic, not control flow — no new NodeKind needed.
#[test]
fn audio_rate_select_needs_no_new_nodes() {
    let g = lower_src(
        "fn select(g, a, b) = a * g + b * (1 - g)\nselect(sin(2), saw(110), square(110))\n"
    ).unwrap();
    let kinds: Vec<_> = g.nodes.iter().map(|n| n.kind.clone()).collect();
    assert_eq!(kinds, vec![
        NodeKind::Sin, NodeKind::Saw, NodeKind::Square,
        NodeKind::Mul, NodeKind::Sub, NodeKind::Mul, NodeKind::Add,
    ]);
}

/// `let ... in` binds for the body only, and shares the node rather than
/// rebuilding it.
#[test]
fn let_expr_shares_one_node() {
    let g = lower_src("fn voice(f) = let e = sin(f) in e * e\nvoice(220)\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Mul, vec![Node(NodeId(0)), Node(NodeId(0))]),
    ]);
}

/// The binding is scoped to the body.
#[test]
fn let_expr_does_not_leak() {
    let err = lower_src("let a = 2 in sin(a)\nsin(a)\n").unwrap_err();
    assert!(err.contains("unbound name: a"), "got: {err}");
}

/// The value is evaluated in the enclosing scope — `let` is not `letrec`.
#[test]
fn let_expr_value_sees_the_outer_binding() {
    let g = lower_src("let a = 2\nsin(let a = a * 10 in a)\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(20.0)])]);
}

/// Reordering the choices must not disturb the statement form of `let`.
#[test]
fn block_let_statement_still_works() {
    let g = lower_src("{\n  let a = 220\n  sin(a)\n}\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
}

#[test]
fn let_expr_works_at_top_level_and_in_blocks() {
    assert!(lower_src("let a = 220 in sin(a)\n").is_ok());
    assert!(lower_src("{ let a = 220 in sin(a) }\n").is_ok());
}

#[test]
fn for_over_a_list_literal() {
    let g = lower_src("for f in [220, 277, 330] { sin(f) }\n").unwrap();
    let sines: Vec<_> = g.nodes.iter().filter(|n| n.kind == NodeKind::Sin)
        .map(|n| n.inputs.clone()).collect();
    assert_eq!(sines, vec![vec![Const(220.0)], vec![Const(277.0)], vec![Const(330.0)]]);
}

#[test]
fn list_can_be_bound_and_indexed() {
    let g = lower_src("let scale = [220, 247, 277]\nsin(scale[2])\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(277.0)])]);
}

/// A range is a value, not special syntax bolted onto `for`.
#[test]
fn range_is_a_list_too() {
    let g = lower_src("let r = 1..=3\nfor i in r { sin(i) }\n").unwrap();
    let sines: Vec<_> = g.nodes.iter().filter(|n| n.kind == NodeKind::Sin)
        .map(|n| n.inputs.clone()).collect();
    assert_eq!(sines, vec![vec![Const(1.0)], vec![Const(2.0)], vec![Const(3.0)]]);
}

/// Something the old `Num ..= Num` grammar could not express.
#[test]
fn range_bounds_can_be_expressions() {
    let g = lower_src("let n = 2\nfor i in 1..=n * 2 { sin(i) }\n").unwrap();
    let sines: Vec<_> = g.nodes.iter().filter(|n| n.kind == NodeKind::Sin)
        .map(|n| n.inputs.clone()).collect();
    assert_eq!(sines, vec![
        vec![Const(1.0)], vec![Const(2.0)], vec![Const(3.0)], vec![Const(4.0)],
    ]);
}

/// Lists hold Values, so a list of oscillators works.
#[test]
fn lists_hold_signals_not_just_numbers() {
    let g = lower_src("for v in [sin(110), saw(220)] { v / 2 }\n").unwrap();
    let kinds: Vec<_> = g.nodes.iter().map(|n| n.kind.clone()).collect();
    assert_eq!(kinds, vec![
        NodeKind::Sin, NodeKind::Saw, NodeKind::Div, NodeKind::Div, NodeKind::Add,
    ]);
}

#[test]
fn nested_index() {
    let g = lower_src("let chords = [[220, 277], [247, 311]]\nsin(chords[1][0])\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(247.0)])]);
}

#[test]
fn multiline_list_without_trailing_comma() {
    let g = lower_src("let s = [\n  220,\n  330\n]\nsin(s[0])\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
}

#[test]
fn list_in_signal_position_is_an_error() {
    let err = lower_src("sin([1, 2])\n").unwrap_err();
    assert!(err.contains("cannot use a list as a signal"), "got: {err}");
}

#[test]
fn index_out_of_bounds_is_an_error() {
    let err = lower_src("let s = [1, 2]\nsin(s[5])\n").unwrap_err();
    assert!(err.contains("out of bounds"), "got: {err}");
}

#[test]
fn iterating_a_non_list_is_an_error() {
    let err = lower_src("for i in 5 { sin(i) }\n").unwrap_err();
    assert!(err.contains("list or range"), "got: {err}");
}

/// `len` is a compile-time value, so it folds into whatever consumes it.
#[test]
fn len_of_a_list_folds() {
    let g = lower_src("sin(len([220, 247, 277]))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(3.0)])]);
}

/// A range is a list, so `len` works on it too.
#[test]
fn len_of_a_range() {
    let g = lower_src("sin(len(1..=8))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(8.0)])]);
}

/// List builtins go through the same call path, so chaining works.
#[test]
fn len_can_be_chained() {
    let g = lower_src("sin([1, 2, 3] >> len)\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(3.0)])]);
}

/// `len` composes with ranges to iterate a list by index.
#[test]
fn len_drives_a_range() {
    let g = lower_src("let s = [110, 220]\nfor i in 1..=len(s) { sin(s[i - 1]) }\n").unwrap();
    let sines: Vec<_> = g.nodes.iter().filter(|n| n.kind == NodeKind::Sin)
        .map(|n| n.inputs.clone()).collect();
    assert_eq!(sines, vec![vec![Const(110.0)], vec![Const(220.0)]]);
}

/// The reason zip exists: walk two lists in step.
#[test]
fn zip_pairs_two_lists() {
    let g = lower_src(
        "let fs = [110, 220]\nlet amps = [0.5, 0.25]\n\
         for p in zip(fs, amps) { sin(p[0]) * p[1] }\n"
    ).unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(110.0)]),
        node(NodeKind::Mul, vec![Node(NodeId(0)), Const(0.5)]),
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Mul, vec![Node(NodeId(2)), Const(0.25)]),
        node(NodeKind::Add, vec![Node(NodeId(1)), Node(NodeId(3))]),
    ]);
    assert_eq!(g.output, Some(NodeId(4)));
}

/// zip is variadic.
#[test]
fn zip_handles_three_lists() {
    let g = lower_src("for t in zip([1, 2], [3, 4], [5, 6]) { sin(t[0] + t[1] + t[2]) }\n").unwrap();
    let sines: Vec<_> = g.nodes.iter().filter(|n| n.kind == NodeKind::Sin)
        .map(|n| n.inputs.clone()).collect();
    assert_eq!(sines, vec![vec![Const(9.0)], vec![Const(12.0)]]);
}

/// Signals survive a zip — the rows hold whatever Values went in.
#[test]
fn zip_carries_signals() {
    let g = lower_src("for p in zip([sin(110)], [0.5]) { p[0] * p[1] }\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(110.0)]),
        node(NodeKind::Mul, vec![Node(NodeId(0)), Const(0.5)]),
    ]);
}

#[test]
fn zip_rejects_length_mismatch() {
    let err = lower_src("for p in zip([1, 2, 3], [4, 5]) { sin(p[0]) }\n").unwrap_err();
    assert!(err.contains("length"), "got: {err}");
}

#[test]
fn zip_rejects_non_lists() {
    let err = lower_src("for p in zip([1, 2], 3) { sin(p[0]) }\n").unwrap_err();
    assert!(err.contains("every argument to be a list"), "got: {err}");
}

#[test]
fn len_rejects_non_lists() {
    let err = lower_src("sin(len(5))\n").unwrap_err();
    assert!(err.contains("len expects a list"), "got: {err}");
}

#[test]
fn len_rejects_wrong_arity() {
    let err = lower_src("sin(len([1], [2]))\n").unwrap_err();
    assert!(err.contains("1 argument"), "got: {err}");
}