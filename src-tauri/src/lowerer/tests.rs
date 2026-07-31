use crate::scree_graph::graph::ScreeGraph;
use crate::scree_graph::ugen_nodes::{NodeId, NodeInput, NodeKind, UGenNode};
use crate::lowerer::lower::lower;
use crate::parser::parser::parse;
use NodeInput::{Const, Node};

fn lower_src(src: &str) -> Result<ScreeGraph, String> {
    let items = parse(src.to_string()).expect("parse failed");
    lower(&items).map(|l| l.graph)
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
/// left to right — the same graph `a + b + c` would produce. Every oscillator
/// is built before any of the sums, because the loop runs its body out in full
/// before deciding it is audio rather than a list.
#[test]
fn for_loop_unrolls_and_sums() {
    let g = lower_src("for i in 1..=3 { sin(i * 110) }\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(110.0)]),
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Sin, vec![Const(330.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
        node(NodeKind::Add, vec![Node(NodeId(3)), Node(NodeId(2))]),
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

/// A body with no signal in it is values, not voices, so the loop collects
/// them. Adding them up is `sum`'s job, and saying so is the difference
/// between building a list and mixing one down.
#[test]
fn for_loop_over_constants_collects() {
    let g = lower_src("sin(sum(for i in 1..=4 { i }))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(10.0)])]);

    // Unsummed it is a list, which is not something to listen to.
    let err = lower_src("sin(for i in 1..=4 { i })\n").unwrap_err();
    assert!(err.contains("cannot use a list as a signal"), "got: {err}");
}

/// The loop that could not be written before: values in, list out. Read back
/// through a lane, which is where a built list is most likely to be going.
#[test]
fn for_loop_builds_a_list() {
    let bs = bindings_of(&format!(
        "{BASS}fn cutoffs() = for i in 0..=3 {{ (i + 1) * 100 }}\n\
         play([220, 330], bass, cut: cutoffs())\n"));

    assert_eq!(
        bs[0].lanes[0].pattern.values(),
        vec![Some(100.0), Some(200.0), Some(300.0), Some(400.0)]);
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
/// A rest is a value, so it occupies a slot in a list like any element.
#[test]
fn rest_is_an_element_of_a_list() {
    let g = lower_src("sin(len([220, `, 330, `]))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(4.0)])]);
}

/// Rests are only meaningful in patterns; using one as audio is an error.
#[test]
fn rest_used_as_a_signal_is_an_error() {
    let err = lower_src("sin(`)\n").unwrap_err();
    assert!(err.contains("rest"), "got: {err}");
}

#[test]
fn rest_in_arithmetic_is_an_error() {
    let err = lower_src("sin(220) * `\n").unwrap_err();
    assert!(err.contains("rest"), "got: {err}");
}

/// A bare rest contributes nothing, like any non-signal top-level value.
#[test]
fn bare_rest_produces_no_output() {
    let g = lower_src("`\n").unwrap();
    assert_eq!(g.nodes, vec![]);
    assert_eq!(g.output, None);
}

/// Rests must not confuse the newline-to-terminator pass.
#[test]
fn rests_survive_a_multiline_list() {
    let g = lower_src("sin(len([\n  220,\n  `,\n  330\n]))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(3.0)])]);
}

/// Indexing reaches a rest, and it still refuses to be audio.
#[test]
fn indexing_a_rest_still_errors_as_a_signal() {
    let err = lower_src("let p = [220, `]\nsin(p[1])\n").unwrap_err();
    assert!(err.contains("rest"), "got: {err}");
}
#[test]
fn list_binding_then_statement() {
    let g = lower_src("let s = [220, 330]\nsin(s[0])\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
}

#[test]
fn pattern_shaped_list_binding() {
    let g = lower_src("let p = [220, `, 330, `]\nsin(len(p))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(4.0)])]);
}

// ---- play / patterns ----

use crate::lowerer::lower::lower as lower_full;
use crate::pattern::pattern::{Pattern, Step};

fn bindings_of(src: &str) -> Vec<crate::pattern::patterns::Binding> {
    let items = parse(src.to_string()).expect("parse failed");
    lower_full(&items).expect("lower failed").bindings
}

fn play_err(src: &str) -> String {
    let items = parse(src.to_string()).expect("parse failed");
    match lower_full(&items) {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn play_binds_a_pattern_to_an_instrument() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay([220, `, 330, `], kick)\n");
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].instrument, "kick");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(220.0), Step::Rest, Step::Value(330.0), Step::Rest,
    ]));
}

/// Rate is a playback property of `play`, not a pattern transformation.
#[test]
fn play_rate_wraps_the_pattern() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay([220, 330], kick, 2)\n");
    assert_eq!(bs[0].pattern, Pattern::Fast(2.0,
        Box::new(Pattern::seq(vec![Step::Value(220.0), Step::Value(330.0)]))));
}

/// A fractional rate slows the pattern down.
#[test]
fn play_rate_below_one_slows_down() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay([220], kick, 0.5)\n");
    assert_eq!(bs[0].pattern, Pattern::Fast(0.5,
        Box::new(Pattern::seq(vec![Step::Value(220.0)]))));
}

/// Rate 1 is the default and adds no wrapper.
#[test]
fn omitted_rate_is_one() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay([220], kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![Step::Value(220.0)]));
}

/// Nested lists subdivide their slot.
#[test]
fn nested_list_becomes_a_group() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay([220, [330, 440]], kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(220.0),
        Step::Group(Box::new(Pattern::seq(vec![
            Step::Value(330.0), Step::Value(440.0),
        ]))),
    ]));
}

/// Layering is just two bindings — no `stack` needed.
#[test]
fn multiple_plays_layer() {
    let bs = bindings_of(
        "fn kick(f) = sin(f)\nfn hat(f) = saw(f)\n\
         play([220, `], kick)\nplay([880, 880, 880], hat)\n");
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[0].instrument, "kick");
    assert_eq!(bs[1].instrument, "hat");
}

/// Patterns compose with the rest of the language.
#[test]
fn pattern_elements_are_ordinary_expressions() {
    let bs = bindings_of("fn kick(f) = sin(f)\nlet r = 110\nplay([r, r * 2, `], kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(110.0), Step::Value(220.0), Step::Rest,
    ]));
}

/// `play` works through the pipe.
#[test]
fn play_accepts_a_piped_pattern() {
    let bs = bindings_of("fn kick(f) = sin(f)\n[220, 330] >> play(kick)\n");
    assert_eq!(bs[0].instrument, "kick");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(220.0), Step::Value(330.0),
    ]));
}

#[test]
fn play_pipes_with_a_rate() {
    let bs = bindings_of("fn kick(f) = sin(f)\n[220] >> play(kick, 4)\n");
    assert_eq!(bs[0].pattern, Pattern::Fast(4.0,
        Box::new(Pattern::seq(vec![Step::Value(220.0)]))));
}

/// Bindings can be generated in a loop.
#[test]
fn play_inside_a_for_makes_several_bindings() {
    let bs = bindings_of(
        "fn kick(f) = sin(f)\nfor i in 1..=3 { play([110 * i], kick, i) }\n");
    assert_eq!(bs.len(), 3);
    assert_eq!(bs[2].pattern, Pattern::Fast(3.0,
        Box::new(Pattern::seq(vec![Step::Value(330.0)]))));
}

/// A program can have both a persistent graph and patterns.
#[test]
fn graph_and_bindings_coexist() {
    let items = parse("fn kick(f) = sin(f)\nplay([220], kick)\nsin(55) / 8\n".to_string())
        .unwrap();
    let out = lower_full(&items).unwrap();
    assert_eq!(out.bindings.len(), 1);
    assert!(out.graph.output.is_some(), "the drone should still reach the output");
}

/// `play` contributes nothing to the audio output itself.
#[test]
fn play_alone_produces_no_graph_output() {
    let items = parse("fn kick(f) = sin(f)\nplay([220], kick)\n".to_string()).unwrap();
    let out = lower_full(&items).unwrap();
    assert_eq!(out.graph.output, None);
}

// ---- play errors ----

#[test]
fn play_rejects_an_unknown_instrument() {
    let err = play_err("play([220], ghost)\n");
    assert!(err.contains("not a function"), "got: {err}");
}

#[test]
fn play_rejects_a_non_identifier_instrument() {
    let err = play_err("fn kick(f) = sin(f)\nplay([220], kick(1))\n");
    assert!(err.contains("plain function name"), "got: {err}");
}

#[test]
fn play_rejects_a_signal_in_a_pattern() {
    let err = play_err("fn kick(f) = sin(f)\nplay([sin(220)], kick)\n");
    assert!(err.contains("signal"), "got: {err}");
}

#[test]
fn play_rejects_a_non_positive_rate() {
    let err = play_err("fn kick(f) = sin(f)\nplay([220], kick, 0)\n");
    assert!(err.contains("positive"), "got: {err}");
}

#[test]
fn play_rejects_a_signal_rate() {
    let err = play_err("fn kick(f) = sin(f)\nplay([220], kick, sin(2))\n");
    assert!(err.contains("compile-time number"), "got: {err}");
}

// ---- play_once / playn ----

/// `play` loops; the whole of the difference is the binding's `cycles`.
#[test]
fn play_loops_forever() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay([220], kick)\n");
    assert_eq!(bs[0].cycles, None);
}

#[test]
fn play_once_bounds_the_binding_to_one_cycle() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplay_once([220, 330], kick)\n");
    assert_eq!(bs[0].instrument, "kick");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(220.0), Step::Value(330.0),
    ]));
    assert_eq!(bs[0].cycles, Some(1.0));
}

#[test]
fn playn_bounds_the_binding_to_its_count() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplayn([220], kick, 4)\n");
    assert_eq!(bs[0].cycles, Some(4.0));
    assert_eq!(bs[0].pattern, Pattern::seq(vec![Step::Value(220.0)]));
}

/// Repeats count passes of the pattern, so a rate that packs two passes into
/// each cycle halves the number of cycles they take.
#[test]
fn a_rate_shortens_the_window_it_speeds_up() {
    let bs = bindings_of("fn kick(f) = sin(f)\nplayn([220], kick, 4, 2)\n");
    assert_eq!(bs[0].cycles, Some(2.0));
    assert_eq!(bs[0].pattern, Pattern::Fast(2.0,
        Box::new(Pattern::seq(vec![Step::Value(220.0)]))));

    let bs = bindings_of("fn kick(f) = sin(f)\nplay_once([220], kick, 0.5)\n");
    assert_eq!(bs[0].cycles, Some(2.0), "a half-speed pass takes two cycles");
}

/// Both take a piped pattern and lanes, like `play`.
#[test]
fn the_bounded_plays_pipe_and_take_lanes() {
    let bs = bindings_of(&format!("{BASS}[220, 330] >> play_once(bass, cut: [400, 2000])\n"));
    assert_eq!(bs[0].cycles, Some(1.0));
    assert_eq!(bs[0].lanes.len(), 1);

    let bs = bindings_of(&format!("{BASS}[220] >> playn(bass, 3, legato: 0.5)\n"));
    assert_eq!(bs[0].cycles, Some(3.0));
    assert_eq!(bs[0].lanes.len(), 1);
}

#[test]
fn playn_needs_a_count() {
    let err = play_err("fn kick(f) = sin(f)\nplayn([220], kick)\n");
    assert!(err.contains("number of repeats"), "got: {err}");
}

#[test]
fn playn_rejects_a_count_below_one() {
    for src in ["playn([220], kick, 0)\n", "playn([220], kick, -2)\n"] {
        let err = play_err(&format!("fn kick(f) = sin(f)\n{src}"));
        assert!(err.contains("at least 1"), "got: {err}");
    }
}

#[test]
fn playn_rejects_a_signal_count() {
    let err = play_err("fn kick(f) = sin(f)\nplayn([220], kick, sin(2))\n");
    assert!(err.contains("compile-time number"), "got: {err}");
}

/// The count is an argument like any other, so the arity message has to say
/// four rather than `play`'s three.
#[test]
fn playn_reports_its_own_arity() {
    let err = play_err("fn kick(f) = sin(f)\nplayn([220], kick, 2, 1, 9)\n");
    assert!(err.contains("playn expects at most 4 arguments, got 5"), "got: {err}");
}

/// Errors name the function that was actually called.
#[test]
fn a_bounded_play_reports_under_its_own_name() {
    let err = play_err("fn kick(f) = sin(f)\nplay_once([220], kick, 0)\n");
    assert!(err.starts_with("play_once:"), "got: {err}");
}

// ---- lanes ----

const BASS: &str = "fn bass(n, cut = 800, amp = 1) = saw(n) * amp\n";

#[test]
fn a_lane_is_bound_as_a_pattern() {
    let bs = bindings_of(&format!("{BASS}play([220, 330], bass, cut: [400, 2000])\n"));

    assert_eq!(bs[0].lanes.len(), 1);
    assert_eq!(bs[0].lanes[0].name, "cut");
    assert_eq!(
        bs[0].lanes[0].pattern,
        Pattern::seq(vec![Step::Value(400.0), Step::Value(2000.0)])
    );
}

/// A scalar lane is just a one-step pattern, so `amp: 0.8` needs no special
/// case anywhere downstream.
#[test]
fn a_scalar_lane_is_a_one_step_pattern() {
    let bs = bindings_of(&format!("{BASS}play([220], bass, amp: 0.8)\n"));
    assert_eq!(bs[0].lanes[0].pattern, Pattern::seq(vec![Step::Value(0.8)]));
}

#[test]
fn lanes_survive_the_pipe_form() {
    let bs = bindings_of(&format!("{BASS}[220, 330] >> play(bass, cut: [400, 2000])\n"));
    assert_eq!(bs[0].instrument, "bass");
    assert_eq!(bs[0].lanes.len(), 1);
}

/// `rate` speeds the pattern up and leaves the lanes alone. A lane is read by
/// position — the nth note takes the nth value — so it follows the notes at
/// whatever speed they go, and compressing it here would compress it twice.
#[test]
fn rate_speeds_the_pattern_and_not_the_lanes() {
    let bs = bindings_of(&format!("{BASS}play([220, 330], bass, 2, cut: [400, 2000])\n"));

    assert_eq!(bs[0].pattern, Pattern::Fast(2.0, Box::new(
        Pattern::seq(vec![Step::Value(220.0), Step::Value(330.0)]))));
    assert_eq!(bs[0].lanes[0].pattern,
        Pattern::seq(vec![Step::Value(400.0), Step::Value(2000.0)]));
}

/// End to end, from source to scheduled events: a lane longer than the pattern
/// walks its whole list rather than being squeezed into the pattern's cycle.
/// Twenty cutoffs under two notes take ten cycles, and every one is heard.
#[test]
fn a_long_lane_is_played_through_from_source() {
    use crate::pattern::pattern::Span;
    use crate::pattern::patterns::Patterns;

    let bs = bindings_of(&format!("{BASS}play([220, 330], bass, cut: 1..=20)\n"));
    let pats = Patterns { bindings: bs, origin: 0.0 };

    let cuts: Vec<f64> = (0..10)
        .flat_map(|c| pats.query(Span::new(c as f64, c as f64 + 1.0)))
        .map(|e| e.args.iter().find(|(n, _)| n == "cut").expect("cut lane").1)
        .collect();

    assert_eq!(cuts, (1..=20).map(|i| i as f64).collect::<Vec<_>>());

    // And then around again, in phase with the pattern.
    let next: Vec<f64> = pats.query(Span::new(10.0, 11.0))
        .iter()
        .map(|e| e.args.iter().find(|(n, _)| n == "cut").expect("cut lane").1)
        .collect();
    assert_eq!(next, vec![1.0, 2.0]);
}

#[test]
fn legato_is_accepted_without_being_a_parameter() {
    let bs = bindings_of(&format!("{BASS}play([220], bass, legato: 0.4)\n"));
    assert_eq!(bs[0].lanes[0].name, "legato");
}

#[test]
fn pan_is_accepted_without_being_a_parameter() {
    let bs = bindings_of(&format!("{BASS}play([220], bass, pan: -1)\n"));
    assert_eq!(bs[0].lanes[0].name, "pan");
}

/// A pan lane is a pattern like any other, so a voice can be placed somewhere
/// different on every note.
#[test]
fn pan_may_be_a_pattern() {
    let bs = bindings_of(&format!("{BASS}play([220, 330], bass, pan: [-1, 1])\n"));
    assert_eq!(bs[0].lanes[0].pattern.values(), vec![Some(-1.0), Some(1.0)]);
}

/// A lane shorter than the pattern is read by position and wraps, so a
/// two-value pan alternates note by note however many notes there are — the
/// spelling the reference gives for ping-pong percussion.
#[test]
fn a_short_pan_lane_alternates_across_the_pattern() {
    let bs = bindings_of(
        "fn hat() = noise()\nplay([\\, `, \\, `, \\, `, \\, `], hat, pan: [-0.8, 0.8])\n");
    assert_eq!(bs[0].lanes[0].pattern.values(), vec![Some(-0.8), Some(0.8)]);
}

/// And a zero-parameter instrument takes one, which is most of a drum kit.
#[test]
fn pan_works_on_an_instrument_that_takes_nothing() {
    let bs = bindings_of("fn kick() = sin(50)\nplay([\\], kick, pan: 0)\n");
    assert_eq!(bs[0].lanes[0].name, "pan");
}

// ---- lane errors ----

#[test]
fn play_rejects_a_lane_the_instrument_has_no_parameter_for() {
    let err = play_err(&format!("{BASS}play([220], bass, cutt: 400)\n"));
    assert!(err.contains("no parameter named 'cutt'"), "got: {err}");
}

/// The pattern fills the first parameter, so naming it as a lane is a
/// contradiction rather than an override.
#[test]
fn play_rejects_a_lane_naming_the_first_parameter() {
    let err = play_err(&format!("{BASS}play([220], bass, n: 400)\n"));
    assert!(err.contains("first parameter"), "got: {err}");
}

#[test]
fn play_rejects_a_repeated_lane() {
    let err = play_err(&format!("{BASS}play([220], bass, cut: 400, cut: 900)\n"));
    assert!(err.contains("given twice"), "got: {err}");
}

#[test]
fn play_rejects_an_instrument_whose_parameter_nothing_fills() {
    let err = play_err("fn bass(n, cut) = saw(n)\nplay([220], bass)\n");
    assert!(err.contains("needs 'cut'"), "got: {err}");
}

/// Filling it by name is exactly what that error asks for.
#[test]
fn a_lane_satisfies_a_parameter_with_no_default() {
    let bs = bindings_of("fn bass(n, cut) = saw(n) * cut\nplay([220], bass, cut: 400)\n");
    assert_eq!(bs[0].lanes[0].name, "cut");
}

/// `legato` changes the note's length, so it can never reach a parameter —
/// which has to be said at bind time, not discovered by ear.
#[test]
fn play_rejects_an_instrument_with_a_legato_parameter() {
    let err = play_err("fn bass(n, legato = 1) = saw(n)\nplay([220], bass, legato: 0.5)\n");
    assert!(err.contains("sets the note's length"), "got: {err}");
}

/// The same for `pan`, which is spent on the finished voice.
#[test]
fn play_rejects_an_instrument_with_a_pan_parameter() {
    let err = play_err("fn bass(n, pan = 0) = saw(n)\nplay([220], bass, pan: 1)\n");
    assert!(err.contains("stereo field"), "got: {err}");
}

/// The reserved names are checked one lane at a time, so a pan alongside
/// ordinary lanes is refused on its own account.
#[test]
fn play_rejects_a_pan_parameter_among_other_lanes() {
    let err = play_err("fn bass(n, cut = 1, pan = 0) = saw(n) * cut\n\
                        play([220], bass, cut: 2, pan: 0)\n");
    assert!(err.contains("stereo field"), "got: {err}");
}

#[test]
fn play_rejects_a_signal_in_a_lane() {
    let err = play_err(&format!("{BASS}play([220], bass, cut: [sin(2)])\n"));
    assert!(err.contains("signal"), "got: {err}");
}

#[test]
fn play_rejects_a_positional_argument_after_a_lane() {
    let err = play_err(&format!("{BASS}play([220], cut: 400, bass)\n"));
    assert!(err.contains("must come before named"), "got: {err}");
}

// ---- list builtins ----

/// Lower `expr` and read back the constant it folded to.
fn num(src: &str) -> f64 {
    let g = lower_src(&format!("sin({src})\n")).expect("lower failed");
    match g.nodes[0].inputs[0] {
        Const(v) => v,
        _ => panic!("expected a folded constant"),
    }
}

/// Read a whole list back as numbers.
///
/// Binds the list once and reads every element out of that single binding —
/// re-evaluating the source per index would re-roll the random builtins.
fn nums(src: &str) -> Vec<f64> {
    let n = num(&format!("len({src})")) as usize;
    let mut prog = format!("let __l = {src}\n");
    for i in 0..n {
        prog.push_str(&format!("sin(__l[{i}])\n"));
    }
    let g = lower_src(&prog).expect("lower failed");
    g.nodes
        .iter()
        .filter(|nd| nd.kind == NodeKind::Sin)
        .map(|nd| match nd.inputs[0] {
            Const(v) => v,
            _ => panic!("expected a folded constant"),
        })
        .collect()
}

fn list_err(src: &str) -> String {
    lower_src(&format!("sin({src})\n")).unwrap_err()
}

#[test]
fn rev_reverses() {
    assert_eq!(nums("rev([1, 2, 3])"), vec![3.0, 2.0, 1.0]);
    assert_eq!(nums("rev([])"), Vec::<f64>::new());
}

#[test]
fn palindrome_mirrors() {
    assert_eq!(nums("palindrome([1, 2, 3])"), vec![1.0, 2.0, 3.0, 3.0, 2.0, 1.0]);
}

#[test]
fn rotate_left_and_right() {
    assert_eq!(nums("rotl([1, 2, 3, 4])"), vec![2.0, 3.0, 4.0, 1.0]);
    assert_eq!(nums("rotr([1, 2, 3, 4])"), vec![4.0, 1.0, 2.0, 3.0]);
    assert_eq!(nums("rotl([1, 2, 3, 4], 2)"), vec![3.0, 4.0, 1.0, 2.0]);
    assert_eq!(nums("rotr([1, 2, 3, 4], 2)"), vec![3.0, 4.0, 1.0, 2.0]);
}

/// Rotating by the length is the identity, and negatives go the other way.
#[test]
fn rotation_wraps() {
    assert_eq!(nums("rotl([1, 2, 3], 3)"), vec![1.0, 2.0, 3.0]);
    assert_eq!(nums("rotl([1, 2, 3], 4)"), nums("rotl([1, 2, 3], 1)"));
    assert_eq!(nums("rotl([1, 2, 3], -1)"), nums("rotr([1, 2, 3], 1)"));
    assert_eq!(nums("rotl([], 3)"), Vec::<f64>::new());
}

#[test]
fn push_and_pop() {
    assert_eq!(nums("push([1, 2], 3)"), vec![1.0, 2.0, 3.0]);
    assert_eq!(nums("pop([1, 2, 3])"), vec![1.0, 2.0]);
    assert!(list_err("pop([])").contains("empty"));
}

/// Lists are immutable: push returns a new one.
#[test]
fn push_does_not_mutate() {
    let g = lower_src("let a = [1, 2]\nlet b = push(a, 3)\nsin(len(a) * 10 + len(b))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(23.0)])]);
}

#[test]
fn sort_ascending() {
    assert_eq!(nums("sort([3, 1, 2])"), vec![1.0, 2.0, 3.0]);
    assert_eq!(nums("sort([-1.5, 2, 0])"), vec![-1.5, 0.0, 2.0]);
    assert!(list_err("sort([1, [2]])").contains("number"));
}

#[test]
fn sum_folds_numbers() {
    assert_eq!(num("sum([1, 2, 3])"), 6.0);
    assert_eq!(num("sum([])"), 0.0);
}

/// A list of signals sums into the graph, like `for` does.
#[test]
fn sum_of_signals_emits_add_nodes() {
    let g = lower_src("sum([sin(110), sin(220), sin(330)])\n").unwrap();
    assert_eq!(g.nodes, vec![
        node(NodeKind::Sin, vec![Const(110.0)]),
        node(NodeKind::Sin, vec![Const(220.0)]),
        node(NodeKind::Sin, vec![Const(330.0)]),
        node(NodeKind::Add, vec![Node(NodeId(0)), Node(NodeId(1))]),
        node(NodeKind::Add, vec![Node(NodeId(3)), Node(NodeId(2))]),
    ]);
    assert_eq!(g.output, Some(NodeId(4)));
}

#[test]
fn split_chunks() {
    assert_eq!(num("len(split([1, 2, 3, 4], 2))"), 2.0);
    assert_eq!(nums("split([1, 2, 3, 4], 2)[0]"), vec![1.0, 2.0]);
    assert_eq!(nums("split([1, 2, 3, 4], 2)[1]"), vec![3.0, 4.0]);
    // A short final chunk is kept.
    assert_eq!(num("len(split([1, 2, 3], 2))"), 2.0);
    assert_eq!(nums("split([1, 2, 3], 2)[1]"), vec![3.0]);
    assert!(list_err("split([1, 2], 0)").contains("at least 1"));
}

#[test]
fn map_applies_the_transform_to_every_element() {
    let src = "fn up(n) = n + 12\n";
    let g = lower_src(&format!("{src}sin(len(map([60, 63], up)))\n")).unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(2.0)])]);

    let g = lower_src(&format!("{src}sin(map([60, 63], up)[0])\n")).unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(72.0)])]);

    let g = lower_src(&format!("{src}sin(map([60, 63], up)[1])\n")).unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(75.0)])]);
}

#[test]
fn map_reads_through_the_dot() {
    let g = lower_src("fn up(n) = n + 12\nsin([60, 63].map(up)[0])\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(72.0)])]);
}

#[test]
fn mapping_an_empty_list_is_empty() {
    let g = lower_src("fn up(n) = n + 12\nsin(len(map([], up)))\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(0.0)])]);
}

/// The reason `map` earns its place beside `for`: a `for` whose body is audio
/// sums, so the voices are gone by the time it answers. `map` keeps them apart.
#[test]
fn map_can_build_a_list_of_signals() {
    let g = lower_src("fn voice(n) = sin(n)\nmap([110, 220], voice)[1]\n").unwrap();
    let sines: Vec<_> = g.nodes.iter().filter(|n| n.kind == NodeKind::Sin).collect();
    assert_eq!(sines.len(), 2, "both voices should still be in the graph");
    assert_eq!(sines[1].inputs, vec![Const(220.0)]);
    // Indexing picked one out, so nothing summed them.
    assert!(!g.nodes.iter().any(|n| n.kind == NodeKind::Add));
}

#[test]
fn map_rejects_a_non_function() {
    let err = lower_src("sin(len(map([1, 2], 3)))\n").unwrap_err();
    assert!(err.contains("function"), "got: {err}");
}

#[test]
fn filter_keeps_matching_elements() {
    let src = "fn big(x) = x > 2\n";
    let g = lower_src(&format!("{src}sin(len(filter([1, 2, 3, 4], big)))\n")).unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(2.0)])]);

    let g = lower_src(&format!("{src}sin(filter([1, 2, 3, 4], big)[0])\n")).unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(3.0)])]);
}

#[test]
fn filter_rejects_a_non_function() {
    let err = lower_src("sin(len(filter([1, 2], 3)))\n").unwrap_err();
    assert!(err.contains("function"), "got: {err}");
}

#[test]
fn filter_rejects_a_signal_predicate_result() {
    let err = lower_src("fn p(x) = sin(x)\nsin(len(filter([1, 2], p)))\n").unwrap_err();
    assert!(err.contains("compile-time number"), "got: {err}");
}

// Random builtins: assert properties, since the seed changes per eval.

#[test]
fn choice_returns_a_member() {
    for _ in 0..20 {
        let v = num("choice([1, 2, 3])");
        assert!([1.0, 2.0, 3.0].contains(&v), "got {v}");
    }
    assert!(list_err("choice([])").contains("empty"));
}

#[test]
fn scramble_is_a_permutation() {
    for _ in 0..20 {
        let mut got = nums("scramble([1, 2, 3, 4, 5])");
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(got, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }
}

/// A zero weight is never selected.
#[test]
fn weighted_choice_respects_zero_weights() {
    for _ in 0..30 {
        let v = num("wchoice([1, 2, 3], [0, 1, 0])");
        assert_eq!(v, 2.0, "only the weighted element should be chosen");
    }
}

#[test]
fn weighted_choice_validates_its_arguments() {
    assert!(list_err("wchoice([1, 2], [1])").contains("weights"));
    assert!(list_err("wchoice([1, 2], [0, 0])").contains("zero"));
    assert!(list_err("wchoice([1, 2], [-1, 1])").contains(">= 0"));
}

/// Random choices advance the RNG, so repeated calls are independent.
#[test]
fn repeated_choices_are_not_all_identical() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..40 {
        seen.insert(num("choice([1, 2, 3, 4, 5, 6, 7, 8])").to_bits());
    }
    assert!(seen.len() > 1, "choice should vary across evals");
}

/// The list functions compose with patterns, which is the point of having them.
#[test]
fn list_builtins_feed_patterns() {
    let bs = bindings_of(
        "fn kick(f) = sin(f)\nplay(rotl(rev([110, 220, `, 330])), kick)\n");
    assert_eq!(bs.len(), 1);
    // rev -> [330, `, 220, 110]; rotl 1 -> [`, 220, 110, 330]
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Rest, Step::Value(220.0), Step::Value(110.0), Step::Value(330.0),
    ]));
}

/// A generated list is a pattern like any other — the reason `randis` and
/// friends answer with a list rather than with something of their own.
#[test]
fn random_lists_feed_patterns() {
    let bs = bindings_of("fn lead(n) = sin(n.m2h)\nplay(randis(4, 60, 72), lead)\n");
    assert_eq!(bs.len(), 1);
    let Pattern::Steps(slots) = &bs[0].pattern else { panic!("expected a sequence") };
    assert_eq!(slots.len(), 4);
    for slot in slots {
        let Step::Value(n) = slot.step else { panic!("expected a plain value") };
        assert!(n.fract() == 0.0 && (60.0..72.0).contains(&n), "{n} is not a note in range");
    }
}

/// The draw happens once, while the program is lowered — so the riff a pattern
/// is bound to is settled, and plays the same until the next eval. Something
/// re-rolled per cycle would be a different feature, and not this one.
#[test]
fn a_random_pattern_is_settled_at_eval() {
    let bs = bindings_of("fn lead(n) = sin(n)\nplay(rands(6, 100, 900), lead)\n");
    let Pattern::Steps(slots) = &bs[0].pattern else { panic!("expected a sequence") };
    // Every step is already a number, not a thunk to be evaluated later.
    assert!(slots.iter().all(|s| matches!(s.step, Step::Value(_))));
    assert_eq!(slots.len(), 6);
}

/// Seeding reaches the scheduler too: the same program binds the same riff.
#[test]
fn a_seeded_pattern_binds_the_same_notes_twice() {
    let src = "fn lead(n) = sin(n)\nseed(1234)\nplay(randis(8, 48, 72), lead)\n";
    assert_eq!(bindings_of(src)[0].pattern, bindings_of(src)[0].pattern);
    let other = src.replace("seed(1234)", "seed(1235)");
    assert_ne!(bindings_of(src)[0].pattern, bindings_of(&other)[0].pattern);
}

// ---- triggers and zero-parameter instruments ----

/// `\` is a sounding step that carries no data.
#[test]
fn trigger_is_a_sounding_step() {
    let bs = bindings_of("fn kick() = sin(50)\nplay([\\, `, \\, \\], kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(1.0), Step::Rest, Step::Value(1.0), Step::Value(1.0),
    ]));
}

/// Triggers subdivide like any other step.
#[test]
fn triggers_nest() {
    let bs = bindings_of("fn kick() = sin(50)\nplay([\\, [\\, \\]], kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(1.0),
        Step::Group(Box::new(Pattern::seq(vec![
            Step::Value(1.0), Step::Value(1.0),
        ]))),
    ]));
}

/// Triggers and numbers mix freely in one pattern.
#[test]
fn triggers_and_numbers_mix() {
    let bs = bindings_of("fn k(f) = sin(f)\nplay([220, \\, `, 330], k)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(220.0), Step::Value(1.0), Step::Rest, Step::Value(330.0),
    ]));
}

/// A trigger is pattern data, not audio.
#[test]
fn trigger_used_as_a_signal_is_an_error() {
    let err = lower_src("sin(\\)\n").unwrap_err();
    assert!(err.contains("trigger"), "got: {err}");
}

/// List builtins are value-agnostic, so they work on triggers too.
#[test]
fn triggers_survive_list_builtins() {
    let bs = bindings_of("fn kick() = sin(50)\nplay(rotl([\\, `, `, `]), kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Rest, Step::Rest, Step::Rest, Step::Value(1.0),
    ]));
}

/// A trigger must not confuse the newline-to-terminator pass.
#[test]
fn trigger_across_a_newline() {
    let bs = bindings_of("fn kick() = sin(50)\nplay([\n  \\,\n  `\n], kick)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![Step::Value(1.0), Step::Rest]));
}

// ---- note names ----

/// The anchors: middle C, concert A, and the value from the original request.
#[test]
fn note_names_use_the_standard_numbering() {
    assert_eq!(num("c4"), 60.0);
    assert_eq!(num("a4"), 69.0);
    assert_eq!(num("a1"), 33.0);
    assert_eq!(num("g3"), 55.0);
}

/// `a4` really is 440 Hz, so note names and `m2h` agree.
#[test]
fn note_names_agree_with_m2h() {
    assert!((num("a4.m2h") - 440.0).abs() < 1e-9);
    assert!((num("c4.m2h") - 261.625_565).abs() < 1e-4);
}

#[test]
fn sharps_and_flats_shift_one_semitone() {
    assert_eq!(num("a1"), 33.0);
    assert_eq!(num("as1"), 34.0);
    assert_eq!(num("af1"), 32.0);
    assert_eq!(num("cs4"), 61.0);
    assert_eq!(num("df4"), 61.0); // the same pitch, spelled two ways
}

/// Enharmonics across the octave boundary resolve correctly.
#[test]
fn enharmonics_cross_octaves() {
    assert_eq!(num("bs3"), num("c4"));
    assert_eq!(num("cf4"), num("b3"));
    assert_eq!(num("es4"), num("f4"));
    assert_eq!(num("ff4"), num("e4"));
}

#[test]
fn every_natural_note_in_an_octave() {
    let expected = [("c4", 60.0), ("d4", 62.0), ("e4", 64.0),
                    ("f4", 65.0), ("g4", 67.0), ("a4", 69.0), ("b4", 71.0)];
    for (name, midi) in expected {
        assert_eq!(num(name), midi, "{name}");
    }
}

/// Octaves span the MIDI range: c0 is 12, g9 is 127.
#[test]
fn octave_range_covers_midi() {
    assert_eq!(num("c0"), 12.0);
    assert_eq!(num("g9"), 127.0);
}

/// The octave is required, which is what keeps short names usable.
#[test]
fn a_bare_letter_is_not_a_note() {
    assert!(lower_src("sin(f)\n").unwrap_err().contains("unbound name: f"));
    assert!(lower_src("sin(as)\n").unwrap_err().contains("unbound name: as"));
    let g = lower_src("fn voice(f) = sin(f)\nvoice(220)\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
}

/// Bindings shadow note names rather than colliding with them.
#[test]
fn bindings_shadow_note_names() {
    assert_eq!(num("c4"), 60.0);
    let g = lower_src("let c4 = 100\nsin(c4)\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(100.0)])]);

    let g = lower_src("fn f(a4) = sin(a4)\nf(7)\n").unwrap();
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(7.0)])]);
}

/// Builtin names beginning with a note letter must not be read as notes.
#[test]
fn builtin_names_are_not_notes() {
    assert!(lower_src("sin(abs)\n").unwrap_err().contains("unbound name: abs"));
    assert_eq!(num("(-3).abs"), 3.0);
    assert_eq!(num("2.pow(3)"), 8.0);
}

/// An out-of-range octave says so, rather than reporting an unbound name.
#[test]
fn an_impossible_octave_is_reported() {
    let e = lower_src("sin(c12)\n").unwrap_err();
    assert!(e.contains("octave 12"), "got: {e}");
    assert!(e.contains("0..=9"), "got: {e}");
}

/// Note names are numbers, so everything numeric works on them.
#[test]
fn note_names_compose_with_methods_and_patterns() {
    assert_eq!(num("c4.oct(1)"), 72.0);
    assert_eq!(num("c4.semi(7)"), num("g4"));
    assert_eq!(num("[c4, e4, g4][1]"), 64.0);

    let bs = bindings_of("fn lead(n) = sin(n.m2h)\nplay([c4, ef4, `, g4], lead)\n");
    assert_eq!(bs[0].pattern, Pattern::seq(vec![
        Step::Value(60.0), Step::Value(63.0), Step::Rest, Step::Value(67.0),
    ]));
}

/// Every file in `examples/` must parse, lower, and realize.
///
/// A shipped example that does not compile is worse than no example, and these
/// are the first thing anyone reads. Realizing too, not just lowering, because
/// port/param mistakes only surface there.
#[test]
fn every_example_compiles_and_realizes() {
    let dir = std::path::Path::new("../examples");
    let mut checked = 0;

    for entry in std::fs::read_dir(dir).expect("examples/ should exist") {
        let path = entry.expect("a readable entry").path();
        if path.extension().map(|e| e != "scree").unwrap_or(true) {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&path).expect("a readable file");

        let items = parse(src.clone()).unwrap_or_else(|e| panic!("{name} failed to parse: {e}"));

        // Through the same pass the app runs, so an example that imports is
        // checked as the thing it will actually be: one file with the modules
        // it names folded into it. Its own path is what `use` resolves from.
        let workspace =
            crate::imports::Workspace::new(Some(path.display().to_string()), None);
        let items = crate::imports::expand(items, &src, &workspace)
            .unwrap_or_else(|e| panic!("{name} failed to resolve its imports: {e}"));

        // A file may name a pattern drawn in the side panel, which lives in a
        // project's `patterns.scree` — and `examples/` is not that project.
        // That is the one legitimate reason an example refers to a name it
        // never defines, so it is skipped rather than failed. Every other
        // kind of mistake — a
        // misspelled UGen, a bad arity, a wrong argument — reports differently
        // and still fails here.
        let lowered = match crate::lowerer::lower::lower(&items) {
            Ok(lowered) => lowered,
            Err(e) if e.starts_with("unbound name:") => {
                println!("{name}: skipped, needs a drawn pattern ({e})");
                continue;
            }
            Err(e) => panic!("{name} failed to lower: {e}"),
        };
        if crate::scree_graph::realizer::realize(&lowered.graph).is_err() {
            panic!("{name} failed to realize");
        }

        // An example that makes no sound is a broken example.
        assert!(
            !lowered.bindings.is_empty() || lowered.graph.output.is_some(),
            "{name} produces neither patterns nor a graph output"
        );

        // Every instrument a pattern names must exist and build.
        let instruments = crate::scheduler::voice::Instruments::from_program(&items);
        for binding in &lowered.bindings {
            // Lanes go in as the scheduler would send them: the value the first
            // note takes, so an example's named arguments are exercised too.
            let lanes: Vec<(String, f64)> = binding
                .lanes
                .iter()
                .filter(|l| l.name != crate::pattern::patterns::LEGATO)
                .filter_map(|l| {
                    l.pattern.values().first().copied().flatten().map(|v| (l.name.clone(), v))
                })
                .collect();
            let voice = crate::scheduler::voice::build_voice(
                &instruments, &binding.instrument, 60.0, &lanes, 0.5);
            if voice.is_err() {
                panic!("{name}: instrument `{}` failed to build", binding.instrument);
            }
        }
        checked += 1;
    }

    assert!(checked > 0, "no examples were checked");
}

// ---- .then ----

/// The instruments every sequencing test below plays.
const SECTIONS: &str = "\
fn lead(n) = sin(n)
fn bass(n) = saw(n)
fn hat(n) = noise() * n
fn chorus() = play([c4, e4], lead)
fn tail() = play_once([c2], bass)
";

/// What follows starts where the previous one stopped.
#[test]
fn then_offsets_what_follows() {
    let bs = bindings_of(&format!(
        "{SECTIONS}playn([c3], bass, 4).then(chorus)\n"));
    assert_eq!(bs.len(), 2);

    assert_eq!(bs[0].instrument, "bass");
    assert_eq!(bs[0].start, 0.0);
    assert_eq!(bs[0].cycles, Some(4.0));

    // The chorus opens exactly where the four cycles ran out.
    assert_eq!(bs[1].instrument, "lead");
    assert_eq!(bs[1].start, 4.0);
    assert_eq!(bs[1].cycles, None);
}

/// `play_once` is one cycle, so what follows starts at cycle 1.
#[test]
fn play_once_hands_over_after_one_cycle() {
    let bs = bindings_of(&format!("{SECTIONS}play_once([c3], bass).then(chorus)\n"));
    assert_eq!(bs[1].start, 1.0);
}

/// Chaining: each link starts where the previous stopped, and the offsets add.
#[test]
fn then_chains() {
    let bs = bindings_of(&format!(
        "{SECTIONS}playn([c3], bass, 2).then(tail).then(chorus)\n"));
    assert_eq!(bs.len(), 3);
    assert_eq!(bs[0].start, 0.0);       // playn, 2 cycles
    assert_eq!(bs[1].start, 2.0);       // tail: play_once, 1 cycle
    assert_eq!(bs[2].start, 3.0);       // chorus, after both
}

/// A `.then` nested inside the section is relative to that section's start,
/// so the offsets compose rather than fighting.
#[test]
fn nested_then_offsets_compose() {
    let src = "\
fn lead(n) = sin(n)
fn bass(n) = saw(n)
fn inner() = play_once([c4], lead)
fn outer() = play_once([c3], bass).then(inner)
playn([c2], bass, 2).then(outer)
";
    let bs = bindings_of(src);
    assert_eq!(bs.len(), 3);
    assert_eq!(bs[0].start, 0.0);   // the playn
    assert_eq!(bs[1].start, 2.0);   // outer's own play_once
    assert_eq!(bs[2].start, 3.0);   // inner, one cycle after that
}

/// `rate` packs repeats into fewer cycles, and the handover follows.
#[test]
fn rate_shortens_the_wait() {
    let bs = bindings_of(&format!("{SECTIONS}playn([c3], bass, 4, 2).then(chorus)\n"));
    // Four passes at double rate is two cycles.
    assert_eq!(bs[0].cycles, Some(2.0));
    assert_eq!(bs[1].start, 2.0);
}

/// A section that starts several things hands over after the longest.
#[test]
fn a_section_with_several_plays_ends_with_the_last() {
    let src = "\
fn lead(n) = sin(n)
fn bass(n) = saw(n)
fn both() = {
  playn([c4], lead, 2)
  playn([c3], bass, 5)
}
fn after() = play_once([c2], bass)
play_once([c1], bass).then(both).then(after)
";
    let bs = bindings_of(src);
    // both's two plays start at cycle 1; the longer runs 5, so `after` is at 6.
    assert_eq!(bs[1].start, 1.0);
    assert_eq!(bs[2].start, 1.0);
    assert_eq!(bs[3].start, 6.0);
}

/// A loop of plays finishes when its last one does.
#[test]
fn a_for_loop_of_plays_hands_over_after_all_of_them() {
    let src = "\
fn lead(n) = sin(n)
fn after() = play_once([c2], lead)
for i in 1..=3 { playn([c4], lead, i) }.then(after)
";
    let bs = bindings_of(src);
    assert_eq!(bs.len(), 4);
    // The longest of the three runs 3 cycles.
    assert_eq!(bs[3].start, 3.0);
}

// ---- .then errors ----

#[test]
fn then_refuses_an_endless_play() {
    let e = play_err(&format!("{SECTIONS}play([c3], bass).then(chorus)\n"));
    assert!(e.contains("never finishes"), "got: {e}");
}

#[test]
fn then_refuses_a_non_play_receiver() {
    let e = play_err(&format!("{SECTIONS}(4).then(chorus)\n"));
    assert!(e.contains("left side must be a play"), "got: {e}");
}

#[test]
fn then_refuses_a_non_function() {
    let e = play_err(&format!("{SECTIONS}play_once([c3], bass).then(4)\n"));
    assert!(e.contains("expects a function"), "got: {e}");
}

#[test]
fn then_refuses_a_function_taking_parameters() {
    let e = play_err(&format!(
        "{SECTIONS}fn takes(x) = play([x], lead)\nplay_once([c3], bass).then(takes)\n"));
    assert!(e.contains("no parameters"), "got: {e}");
}

/// A section that never stops cannot be followed by anything.
#[test]
fn then_after_an_endless_section_is_refused() {
    let src = "\
fn lead(n) = sin(n)
fn endless() = play([c4], lead)
fn after() = play_once([c2], lead)
play_once([c3], lead).then(endless).then(after)
";
    let e = play_err(src);
    assert!(e.contains("never finishes"), "got: {e}");
}

// ---- play_all ----

/// Nothing is sequenced: the parts keep the start they already had.
#[test]
fn play_all_leaves_its_parts_where_they_were() {
    let bs = bindings_of(&format!(
        "{SECTIONS}play_all(playn([c3], bass, 4), play_once([c4], lead))\n"));
    assert_eq!(bs.len(), 2);
    assert_eq!((bs[0].start, bs[0].cycles), (0.0, Some(4.0)));
    assert_eq!((bs[1].start, bs[1].cycles), (0.0, Some(1.0)));
}

/// The point of the grouping: `.then` follows the longest part, not the last
/// one written.
#[test]
fn play_all_hands_over_after_its_longest_part() {
    let bs = bindings_of(&format!(
        "{SECTIONS}play_all(playn([c3], bass, 4), play_once([c4], lead)).then(chorus)\n"));
    assert_eq!(bs.len(), 3);
    assert_eq!(bs[2].instrument, "lead");   // chorus
    assert_eq!(bs[2].start, 4.0);
}

/// A group inside a section is offset with it, and the chain goes on past it.
#[test]
fn play_all_composes_with_then_in_both_directions() {
    let src = "\
fn lead(n) = sin(n)
fn bass(n) = saw(n)
fn middle() = play_all(playn([c4], lead, 2), play_once([c3], bass))
fn after() = play_once([c2], bass)
play_once([c1], bass).then(middle).then(after)
";
    let bs = bindings_of(src);
    assert_eq!(bs.len(), 4);
    assert_eq!(bs[0].start, 0.0);   // the leading play_once
    assert_eq!(bs[1].start, 1.0);   // middle's two, together
    assert_eq!(bs[2].start, 1.0);
    assert_eq!(bs[3].start, 3.0);   // after the longer of the two
}

/// Groups nest, and the outer one still ends with the last of everything.
#[test]
fn play_all_nests() {
    let bs = bindings_of(&format!(
        "{SECTIONS}play_all(play_all(play_once([c3], bass), playn([c4], lead, 3)), \
         play_once([c2], hat)).then(chorus)\n"));
    assert_eq!(bs.len(), 4);
    assert!(bs[..3].iter().all(|b| b.start == 0.0));
    assert_eq!(bs[3].start, 3.0);
}

/// One part that never stops makes the group never stop.
#[test]
fn play_all_with_an_endless_part_cannot_be_followed() {
    let e = play_err(&format!(
        "{SECTIONS}play_all(play([c3], bass), play_once([c4], lead)).then(chorus)\n"));
    assert!(e.contains("never finishes"), "got: {e}");
}

#[test]
fn play_all_refuses_a_non_play_argument() {
    let e = play_err(&format!("{SECTIONS}play_all(play_once([c3], bass), 4)\n"));
    assert!(e.contains("argument 2 is not a play"), "got: {e}");
}

#[test]
fn play_all_needs_something_to_group() {
    let e = play_err(&format!("{SECTIONS}play_all()\n"));
    assert!(e.contains("at least one play"), "got: {e}");
}

/// A play handle is not audio and not pattern data.
#[test]
fn a_play_handle_is_not_a_value_to_compute_with() {
    let e = play_err(&format!("{SECTIONS}sin(play_once([c3], bass))\n"));
    assert!(e.contains("not audio"), "got: {e}");
}







// ---- samples ----

/// The buffer every test below writes, under the path they all name.
///
/// A ramp from -1 to 1, so what comes out of a `sample` node says where in the
/// buffer it was read from. Synthesized rather than decoded: what a file
/// *contains* is `samples`'s business, and this is about what the language
/// does with one once it has it.
fn sampled(src: &str) -> Result<crate::lowerer::lower::Lowered, String> {
    use std::sync::Arc;
    let mut wave = fundsp::wave::Wave::new(1, 1000.0);
    for i in 0..2000 {
        wave.push(i as f32 / 1000.0 - 1.0);
    }
    let samples = crate::samples::Samples::from_pairs(
        [("break.wav".to_string(), Arc::new(wave))]);

    let items = parse(src.to_string()).expect("parse failed");
    crate::lowerer::lower::lower_with_samples(&items, samples)
}

fn sample_err(src: &str) -> String {
    match sampled(src) {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn a_loaded_buffer_can_be_read_at_a_position() {
    let g = sampled("sample(load(\"break.wav\"), ramp(0.5))\n").expect("should lower").graph;

    assert_eq!(g.samples.len(), 1, "the buffer should be in the graph");
    let read = g.nodes.iter().find(|n| n.kind == NodeKind::Sample).expect("a Sample node");
    // The position is wired; the buffer index and channel are baked in.
    assert!(matches!(read.inputs[0], Node(_)), "the position should be a wired input");
    assert_eq!(read.inputs[1], Const(0.0), "buffer 0");
    assert_eq!(read.inputs[2], Const(0.0), "channel 0 by default");
}

/// `secs` is known while lowering, which is what lets it divide into a `ramp`
/// frequency rather than having to be measured at audio rate.
#[test]
fn a_buffers_length_is_a_compile_time_number() {
    // 2000 frames at 1000 Hz is two seconds, so the phasor is 0.5 Hz — and
    // folds to a constant rather than becoming a Div node.
    let g = sampled("ramp(1 / load(\"break.wav\").secs)\n").expect("should lower").graph;
    assert_eq!(g.nodes, vec![node(NodeKind::Ramp, vec![Const(0.5)])]);
}

#[test]
fn a_buffers_channel_count_is_a_number() {
    let g = sampled("sin(load(\"break.wav\").channels * 100)\n").expect("should lower").graph;
    assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(100.0)])]);
}

/// One file read four ways is one buffer. A break chopped sixteen times should
/// not be sixteen copies of the audio.
#[test]
fn one_file_read_many_times_is_stored_once() {
    let src = "\
let b = load(\"break.wav\")
sample(b, ramp(1)) + sample(b, ramp(2)) + sample(load(\"break.wav\"), ramp(4))
";
    let g = sampled(src).expect("should lower").graph;
    assert_eq!(g.samples.len(), 1, "one file, one buffer");
    assert_eq!(g.nodes.iter().filter(|n| n.kind == NodeKind::Sample).count(), 3);
}

/// The chosen shape: direction and speed are arithmetic on the position, so
/// reversing needs nothing from `sample` at all.
#[test]
fn reversing_is_arithmetic_on_the_position() {
    let g = sampled("sample(load(\"break.wav\"), 1 - ramp(0.5))\n").expect("should lower").graph;
    assert!(g.nodes.iter().any(|n| n.kind == NodeKind::Sub), "1 - ramp is a Sub node");
    assert!(g.nodes.iter().any(|n| n.kind == NodeKind::Sample));
}

#[test]
fn a_second_channel_can_be_asked_for() {
    let g = sampled("sample(load(\"break.wav\"), ramp(1), 1)\n").expect("should lower").graph;
    let read = g.nodes.iter().find(|n| n.kind == NodeKind::Sample).unwrap();
    assert_eq!(read.inputs[2], Const(1.0));
}

// ---- what a buffer is not ----

#[test]
fn a_buffer_is_not_a_signal() {
    let e = sample_err("sin(load(\"break.wav\"))\n");
    assert!(e.contains("sample(buffer, position)"), "got: {e}");
}

#[test]
fn a_buffer_is_not_a_pattern() {
    let e = sample_err("fn v(n) = sin(n)\nplay([load(\"break.wav\")], v)\n");
    assert!(e.contains("cannot contain a buffer"), "got: {e}");
}

#[test]
fn reading_something_that_is_not_a_buffer_says_so() {
    let e = sample_err("sample(220, ramp(1))\n");
    assert!(e.contains("must be a buffer"), "got: {e}");
}

/// The channel picks which reader is built, so it cannot be modulated.
#[test]
fn a_channel_must_be_a_compile_time_number() {
    let e = sample_err("sample(load(\"break.wav\"), ramp(1), sin(2))\n");
    assert!(e.contains("compile-time number"), "got: {e}");
}

// ---- load ----

/// The path has to be findable by the walk that loads it, which means written
/// out rather than computed.
#[test]
fn a_path_must_be_written_out() {
    let e = sample_err("let p = 1\nsample(load(p), ramp(1))\n");
    assert!(e.contains("written out as a string"), "got: {e}");
}

#[test]
fn a_string_anywhere_else_is_refused() {
    let e = sample_err("sin(\"break.wav\")\n");
    assert!(e.contains("only meaningful as the path"), "got: {e}");
}

/// A program naming a file nothing loaded is a bug in the walk rather than
/// anything a program did — but it must still be an error, not a panic.
#[test]
fn a_buffer_that_was_never_loaded_is_an_error() {
    let e = sample_err("sample(load(\"missing.wav\"), ramp(1))\n");
    assert!(e.contains("was not loaded"), "got: {e}");
}

#[test]
fn a_path_is_not_something_to_chain_into() {
    let e = sample_err("1 >> load(\"break.wav\")\n");
    assert!(e.contains("not something to chain into"), "got: {e}");
}

/// The sampling examples in the README, compiled.
///
/// Documentation that does not run is worse than none, and these are the whole
/// explanation of the feature. Kept as the source text the reference shows, so
/// a change to one has to be a change to both.
#[test]
fn the_readme_sampling_examples_compile() {
    // `breaks/amen.wav` and `pad.wav` as the reference writes them.
    fn readme_samples() -> crate::samples::Samples {
        use std::sync::Arc;
        let mut wave = fundsp::wave::Wave::new(2, 1000.0);
        for i in 0..2000 {
            wave.push((i as f32 / 1000.0 - 1.0, i as f32 / 1000.0 - 1.0));
        }
        let wave = Arc::new(wave);
        crate::samples::Samples::from_pairs([
            ("breaks/amen.wav".to_string(), wave.clone()),
            ("pad.wav".to_string(), wave),
        ])
    }

    let examples: &[&str] = &[
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, ramp(1 / amen.secs))\n",
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, 1 - ramp(1 / amen.secs))\n",
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, ramp(2 / amen.secs))\n",
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, ramp(0.5 / amen.secs))\n",
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, ramp(4 / amen.secs) * 0.25)\n",
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, 0.5 + ramp(4 / amen.secs) * 0.25)\n",
        "let amen = load(\"breaks/amen.wav\")\nsample(amen, ramp(1 / amen.secs) >> hold(16, 0))\n",
        // The chopping example, entire.
        "fn chop(n, at = 0) =\n  sample(load(\"breaks/amen.wav\"), at + ramp(n) * 0.0625) * perc(0.001, 0.2)\n\
         play([\\, \\, \\, \\, \\, \\, \\, \\], chop, 1,\n     at: [0, 0.25, 0.5, 0.0625, 0.75, 0.5, 0.125, 0.875])\n",
        // The stereo one.
        "let stereo = load(\"pad.wav\")\nlet pos = ramp(1 / stereo.secs)\n\
         (sample(stereo, pos, 0) + sample(stereo, pos, 1)) * 0.5\n",
    ];

    for src in examples {
        let items = parse(src.to_string())
            .unwrap_or_else(|e| panic!("README example failed to parse: {e}\n{src}"));
        let lowered = crate::lowerer::lower::lower_with_samples(&items, readme_samples())
            .unwrap_or_else(|e| panic!("README example failed to lower: {e}\n{src}"));
        crate::scree_graph::realizer::realize(&lowered.graph)
            .unwrap_or_else(|e| panic!("README example failed to realize: {e}\n{src}"));
    }
}

/// And the chopping example really builds a voice, which is the half that
/// lowering the program does not reach.
#[test]
fn the_readme_chop_instrument_builds_a_voice() {
    use std::sync::Arc;
    let mut wave = fundsp::wave::Wave::new(1, 1000.0);
    for i in 0..1000 {
        wave.push(i as f32 / 500.0 - 1.0);
    }
    let samples = crate::samples::Samples::from_pairs(
        [("breaks/amen.wav".to_string(), Arc::new(wave))]);

    let src = "fn chop(n, at = 0) =\n  \
               sample(load(\"breaks/amen.wav\"), at + ramp(n) * 0.0625) * perc(0.001, 0.2)\n";
    let items = parse(src.to_string()).expect("should parse");
    let ins = crate::scheduler::voice::Instruments::from_program(&items).with_samples(samples);

    let lanes = vec![("at".to_string(), 0.75)];
    assert!(crate::scheduler::voice::build_voice(&ins, "chop", 1.0, &lanes, 0.25).is_ok());
}

/// `;` end to end: what someone types, as the rhythm it turns into.
///
/// The timing arithmetic is `pattern.rs`'s to test; these are about the trip
/// from source text to a bound pattern — that the token parses where a step
/// belongs, that the number reaches the slot, and that the two positions a list
/// can be read in give it the two meanings they should.
#[cfg(test)]
mod length_tests {
    use super::*;
    use crate::pattern::pattern::{Slot, Span, UNIT};

    const TONE: &str = "fn tone(n, cut = 800) = saw(n)\n";

    fn pattern_of(src: &str) -> Pattern {
        bindings_of(src).into_iter().next().expect("a binding").pattern
    }

    fn slots(p: &Pattern) -> Vec<Slot> {
        match p {
            Pattern::Steps(slots) => slots.clone(),
            other => panic!("expected a sequence, got {other:?}"),
        }
    }

    /// The token parses, and the number lands on the slot it followed.
    #[test]
    fn a_length_reaches_its_slot() {
        let got = slots(&pattern_of(&format!("{TONE}play([220;2, 330, 440], tone)\n")));
        assert_eq!(
            got,
            vec![
                Slot::sized(Step::Value(220.0), 2.0),
                Slot::new(Step::Value(330.0)),
                Slot::new(Step::Value(440.0)),
            ],
        );
    }

    /// A pattern with no `;` in it is untouched — the whole compatibility
    /// promise, checked at the level someone actually writes.
    #[test]
    fn a_pattern_without_lengths_is_unchanged() {
        let got = slots(&pattern_of(&format!("{TONE}play([220, 330], tone)\n")));
        assert!(got.iter().all(|s| s.length == UNIT), "got {got:?}");
    }

    /// Rests and triggers take lengths too, which is what lets a bar end in
    /// silence without padding it out with single cells.
    #[test]
    fn rests_and_triggers_take_lengths() {
        let got = slots(&pattern_of(&format!(
            "fn hit() = sin(50)\nplay([\\;3, `;5], hit)\n")));
        assert_eq!(got[0].length, 3.0);
        assert_eq!(got[1].length, 5.0);
        assert_eq!(got[1].step, Step::Rest);
    }

    /// The motivating rhythm, written the way it would be: a quarter, two
    /// eighths, and a half of silence, in one bar.
    #[test]
    fn a_quarter_and_two_eighths_from_source() {
        let p = pattern_of(&format!("{TONE}play([220;2, 330, 440, `;4], tone)\n"));
        let evs = p.query(Span::new(0.0, 1.0));
        let onsets: Vec<f64> = evs.iter().map(|e| e.begin).collect();
        assert_eq!(onsets, vec![0.0, 0.25, 0.375]);
        assert_eq!(evs[0].duration(), 0.25, "the quarter lasts a beat");
    }

    /// A length may be any expression that folds to a number, like `rate`.
    #[test]
    fn a_length_may_be_a_bound_name() {
        let src = format!("{TONE}let beats = 3\nplay([220;beats, 330], tone)\n");
        assert_eq!(slots(&pattern_of(&src))[0].length, 3.0);
    }

    /// Nesting and `;` compose: a group takes its share and divides it.
    #[test]
    fn a_group_takes_a_length() {
        let got = slots(&pattern_of(&format!("{TONE}play([220;3, [330, 440]], tone)\n")));
        assert_eq!(got[0].length, 3.0);
        assert!(matches!(got[1].step, Step::Group(_)), "got {:?}", got[1].step);
    }

    /// A lane is indexed by note, so a length there is a count of notes.
    #[test]
    fn a_lane_length_holds_a_value_across_notes() {
        let bs = bindings_of(&format!(
            "{TONE}play([220, 330, 440], tone, cut: [400;2, 2000])\n"));
        assert_eq!(bs[0].lanes[0].pattern.values(),
                   vec![Some(400.0), Some(400.0), Some(2000.0)]);
    }

    /// And so a fraction there is not a place. Rejected rather than rounded,
    /// because the mistake is a misunderstanding of what the lane is.
    #[test]
    fn a_fractional_lane_length_is_refused() {
        let err = play_err(&format!("{TONE}play([220], tone, cut: [400;1.5, 2000])\n"));
        assert!(err.contains("whole number"), "got: {err}");
        assert!(err.contains("cut"), "the message should name the lane: {err}");
    }

    /// A fraction in the pattern itself is fine — that is a dotted note.
    #[test]
    fn a_fractional_pattern_length_is_allowed() {
        let got = slots(&pattern_of(&format!("{TONE}play([220;1.5, 330;0.5], tone)\n")));
        assert_eq!(got[0].length, 1.5);
    }

    /// A list being used as data has no extent for a length to be a share of,
    /// so `len` counts what was written. The alternative — expanding `[1;3, 2]`
    /// to four elements — would give one literal two different lengths
    /// depending on which side of a `play` it was read from.
    #[test]
    fn plain_list_operations_ignore_lengths() {
        let g = lower_src("sin(len([1;3, 2]))\n").unwrap();
        assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(2.0)])]);
    }

    /// Indexing gives the element, not the element with its length stuck to it.
    #[test]
    fn indexing_yields_the_bare_value() {
        let g = lower_src("sin([220;4, 330][0])\n").unwrap();
        assert_eq!(g.nodes, vec![node(NodeKind::Sin, vec![Const(220.0)])]);
    }

    /// Rearranging a list keeps each element's length with it — `rev` moves
    /// notes around, it does not restripe the rhythm.
    #[test]
    fn reversing_carries_lengths_along() {
        let got = slots(&pattern_of(&format!("{TONE}play(rev([220;3, 330]), tone)\n")));
        assert_eq!(got[0].length, UNIT, "330 was unweighted and still is");
        assert_eq!(got[1].length, 3.0, "220 kept its length across the reverse");
    }

    /// Zero and negative lengths are not rhythms, and are caught where the
    /// number is folded rather than left to divide something by nothing.
    #[test]
    fn a_length_must_be_positive() {
        for bad in ["0", "-2"] {
            let err = play_err(&format!("{TONE}play([220;{bad}, 330], tone)\n"));
            assert!(err.contains("positive"), "length {bad} gave: {err}");
        }
    }
}

/// `stack` — patterns that sound at once.
///
/// The piano roll's primitive: overlapping notes are decomposed into
/// monophonic voices and layered, and a chord is the case where the voices
/// happen to line up.
#[cfg(test)]
mod stack_tests {
    use super::*;
    use crate::pattern::pattern::Span;

    const TONE: &str = "fn tone(n, cut = 800) = saw(n)\n";

    fn pattern_of(src: &str) -> Pattern {
        bindings_of(src).into_iter().next().expect("a binding").pattern
    }

    fn onsets(p: &Pattern, span: Span) -> Vec<f64> {
        let mut got: Vec<f64> = p.query(span).iter().map(|e| e.begin).collect();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        got
    }

    /// A chord: three notes struck together, all lasting the whole cycle.
    #[test]
    fn a_chord_sounds_all_its_notes_at_once() {
        let p = pattern_of(&format!("{TONE}play(stack([c4], [e4], [g4]), tone)\n"));
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(evs.len(), 3);
        assert!(evs.iter().all(|e| e.begin == 0.0), "all together: {evs:?}");

        let mut values: Vec<f64> = evs.iter().map(|e| e.value).collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(values, vec![60.0, 64.0, 67.0]);
    }

    /// Layers keep their own division, which is the whole reason this is not
    /// just a longer list: three against four needs both to be true at once.
    #[test]
    fn layers_divide_independently() {
        let p = pattern_of(&format!(
            "{TONE}play(stack([220, 330, 440], [55, 110]), tone)\n"));
        // Thirds from one layer, halves from the other.
        assert_eq!(
            onsets(&p, Span::new(0.0, 1.0)),
            vec![0.0, 0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0],
        );
    }

    /// Lengths and layers compose: this is what a drawn part turns into — one
    /// voice holding while another moves under it.
    #[test]
    fn a_held_note_under_a_moving_line() {
        let p = pattern_of(&format!(
            "{TONE}play(stack([c4;2, e4;2], [g4;4]), tone)\n"));
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(evs.len(), 3);

        let held = evs.iter().find(|e| e.value == 67.0).expect("the g");
        assert_eq!(held.duration(), 1.0, "the held note spans the cycle");
        let moving: Vec<f64> = evs.iter().filter(|e| e.value != 67.0).map(|e| e.begin).collect();
        assert_eq!(moving, vec![0.0, 0.5]);
    }

    /// A stack inside a step is a chord at one point of a longer line, rather
    /// than a layer over the whole of it.
    #[test]
    fn a_stack_may_fill_one_step() {
        let p = pattern_of(&format!(
            "{TONE}play([c4, stack([e4], [g4])], tone)\n"));
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(onsets(&p, Span::new(0.0, 1.0)), vec![0.0, 0.5, 0.5]);
        assert_eq!(evs.iter().filter(|e| e.begin == 0.5).count(), 2);
    }

    /// Stacks nest, so a voice may itself be layered.
    #[test]
    fn stacks_nest() {
        let p = pattern_of(&format!(
            "{TONE}play(stack([c4], stack([e4], [g4])), tone)\n"));
        assert_eq!(p.query(Span::new(0.0, 1.0)).len(), 3);
    }

    /// A lane reads a stack as all its values, layer by layer — the same
    /// positional flattening a nested list gets.
    #[test]
    fn a_lane_reads_a_stack_in_order() {
        let bs = bindings_of(&format!(
            "{TONE}play([220, 330], tone, cut: stack([400], [2000]))\n"));
        assert_eq!(bs[0].lanes[0].pattern.values(), vec![Some(400.0), Some(2000.0)]);
    }

    /// An empty stack is a mistake worth naming rather than silence.
    #[test]
    fn an_empty_stack_is_an_error() {
        let err = play_err(&format!("{TONE}play(stack(), tone)\n"));
        assert!(err.contains("at least one"), "got: {err}");
    }

    /// A stack is layered patterns, not audio, and saying so beats a type name.
    #[test]
    fn a_stack_is_not_a_signal() {
        // `Lowered` has no Debug, so `expect_err` is unavailable here.
        let err = play_err("sin(stack([220], [330]))\n");
        assert!(err.contains("not audio"), "got: {err}");
    }}
