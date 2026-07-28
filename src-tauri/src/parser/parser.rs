use chumsky::error::{RichPattern, RichReason};
use chumsky::prelude::*;
use chumsky::input::ValueInput;
use logos::Logos;
use crate::diagnostic::{Diagnostic, Stage};
use crate::parser::lex::{insert_terminators, Span, Token};

#[derive(Clone, Debug, PartialEq)]
pub struct Ident(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct Param { pub name: Ident, pub default: Option<Expr> }

/// One argument at a call site. `name` is `Some` for `cut: 400`, which binds to
/// the parameter of that name rather than by position.
#[derive(Clone, Debug, PartialEq)]
pub struct Arg { pub name: Option<Ident>, pub value: Expr }

impl Arg {
    /// A positional argument, for the many places that synthesize calls.
    pub fn positional(value: Expr) -> Arg {
        Arg { name: None, value }
    }

    /// A named argument: `cut: 400`.
    pub fn named(name: &str, value: Expr) -> Arg {
        Arg { name: Some(Ident(name.to_string())), value }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScreeItem {
    Function { name: Ident, params: Vec<Param>, body: Expr },
    Let { name: Ident, value: Expr },
    Call { func: Ident, args: Vec<Arg> },
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Add   { lhs: Box<Expr>, rhs: Box<Expr> },
    Block { stmts: Vec<Statement>, tail: Box<Expr> },
    Call  { func: Ident, args: Vec<Arg> },
    Cmp { op: CmpOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Chain { lhs: Box<Expr>, rhs: Box<Expr> },
    Div   { lhs: Box<Expr>, rhs: Box<Expr> },
    For { var: Ident, iter: Box<Expr>, body: Box<Expr> },
    If  { cond: Box<Expr>, then: Box<Expr>, otherwise: Option<Box<Expr>> },
    Index { base: Box<Expr>, index: Box<Expr> },
    Let { name: Ident, value: Box<Expr>, body: Box<Expr> },
    List(Vec<Expr>),
    Mul   { lhs: Box<Expr>, rhs: Box<Expr> },
    Neg { expr: Box<Expr> },
    Num(f64),
    Range { lo: Box<Expr>, hi: Box<Expr> },
    Rem { lhs: Box<Expr>, rhs: Box<Expr> },
    /// A silent step in a pattern, written `` ` ``.
    Rest,
    /// A sounding step with no value, written as a single backslash.
    Trigger,
    Sub   { lhs: Box<Expr>, rhs: Box<Expr> },
    Var(Ident),
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CmpOp { Lt, Le, Gt, Ge, Eq, Ne }


#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Let { name: Ident, value: Box<Expr> },
    Expr(Expr),
}

#[allow(dead_code)]
pub enum Pattern {
    Ident(Ident)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Range { Const(i64, i64) }

// One enum per precedence tier, so each fold below can match exhaustively.
// A single shared BinOp would force a catch-all arm in both, which is how
// `Rem` once slipped through the product fold into `unreachable!()`.
#[derive(Clone, Debug)]
pub enum ProductOp {
    Mul, Div, Rem,
}

#[derive(Clone, Debug)]
pub enum SumOp {
    Add, Sub,
}

/// One postfix step. Both bind tighter than any infix operator, and they share
/// a fold so `xs[0].m2h` and `x.scale(s)[1]` both parse.
enum Postfix {
    Index(Expr),
    Method(Ident, Vec<Arg>),
}

/// Labelled, because `select!` has nothing to say about what it wanted: an
/// error where a name belongs would otherwise read "expected something else".
fn ident<'a, I>() -> impl Parser<'a, I, Ident, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    select! { Token::Ident(s) => Ident(s) }.labelled("a name")
}

/// Read a program, or say what stopped it.
///
/// Both failures carry a place in the source: the lexer's is the character it
/// could not read, and the parser's is recovered by mapping the token index it
/// reports back through the spans the lexer kept.
pub fn parse(code: String) -> Result<Vec<ScreeItem>, Diagnostic> {
    let mut raw_tokens: Vec<(Token, Span)> = Vec::new();
    for (token, span) in Token::lexer(&code).spanned() {
        match token {
            Ok(token) => raw_tokens.push((token, span)),
            Err(_) => {
                let text = code.get(span.clone()).unwrap_or_default();
                return Err(Diagnostic::at(
                    Stage::Lex,
                    format!("`{text}` is not part of the language"),
                    &code,
                    span.start,
                ));
            }
        }
    }

    let (tokens, spans) = insert_terminators(raw_tokens);

    // Bound rather than returned directly: the parser borrows `tokens`, and as
    // a tail expression it would outlive them.
    let result = parser()
        .parse(&tokens[..])
        .into_result()
        .map_err(|errs| parse_diagnostic(&errs, &spans, &code));

    result
}

/// The first parse error, as something worth reading.
///
/// Only the first: chumsky reports every branch that failed, and after the
/// earliest one they are describing a parse that had already gone wrong.
fn parse_diagnostic(
    errs: &[Rich<'_, Token>],
    spans: &[Span],
    code: &str,
) -> Diagnostic {
    let Some(err) = errs.first() else {
        // `into_result` never gives an empty error vector, but a parse that
        // failed must not report itself as having no reason.
        return Diagnostic::message(Stage::Parse, "could not parse this program");
    };

    // The error's span indexes the token stream, not the source. A span past
    // the end is an error against end-of-input, which belongs after the last
    // token rather than nowhere.
    let offset = spans
        .get(err.span().start)
        .map(|s| s.start)
        .or_else(|| spans.last().map(|s| s.end))
        .unwrap_or(0);

    Diagnostic::at(Stage::Parse, parse_message(err), code, offset)
}

fn parse_message(err: &Rich<'_, Token>) -> String {
    match err.reason() {
        RichReason::Custom(message) => message.clone(),
        RichReason::ExpectedFound { expected, .. } => {
            let found = match err.found() {
                Some(token) => format!("unexpected `{token}`"),
                None => "the file ends here".to_string(),
            };

            // Ranked before sorting so the useful candidates survive the cap:
            // a call missing its `)` can have the parser expecting every infix
            // operator in the language alongside it, and the `)` is the answer.
            let mut ranked: Vec<(u8, String)> = expected.iter().map(pattern_name).collect();
            ranked.sort();
            ranked.dedup();
            let names: Vec<String> = ranked.into_iter().map(|(_, name)| name).collect();

            match expected_list(&names) {
                Some(list) => format!("{found}, expected {list}"),
                None => found,
            }
        }
    }
}

/// One thing the parser would have accepted, as it is written, paired with how
/// much it is worth saying: the thing that closes or separates what you are in
/// the middle of writing tells you more than the operators that could follow it.
fn pattern_name(pattern: &RichPattern<'_, Token>) -> (u8, String) {
    match pattern {
        // The two tokens whose `Display` is a phrase rather than a spelling:
        // backticks around them would be quoting something nobody can type.
        RichPattern::Token(token) if matches!(&**token, Token::Term | Token::NewLine) => {
            (1, token.to_string())
        }
        RichPattern::Token(token) => {
            let rank = match &**token {
                Token::ParensClose | Token::BracketClose | Token::BraceClose => 0,
                Token::Comma | Token::Colon | Token::Assign | Token::Term => 1,
                Token::ParensOpen | Token::BracketOpen | Token::BraceOpen => 2,
                Token::Else | Token::For | Token::Function | Token::If
                | Token::In | Token::Let => 2,
                _ => 3,
            };
            (rank, format!("`{}`", &**token))
        }
        RichPattern::EndOfInput => (1, "the end of the file".to_string()),
        other => (2, other.to_string()),
    }
}

/// "`,`", "`,` or `]`", "`,`, `]` or 3 more".
///
/// Capped because a failure early in an expression can have the parser
/// expecting every operator and every literal at once, and a list that long
/// says less than the first few of it do.
fn expected_list(names: &[String]) -> Option<String> {
    const SHOWN: usize = 4;

    match names.len() {
        0 => None,
        1 => Some(names[0].clone()),
        n if n <= SHOWN => Some(format!(
            "{} or {}",
            names[..n - 1].join(", "),
            names[n - 1],
        )),
        n => Some(format!(
            "{} or {} more",
            names[..SHOWN].join(", "),
            n - SHOWN,
        )),
    }
}

fn expr<'a, I>() -> impl Parser<'a, I, Expr, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    recursive(|expr| {
        let sep = just(Token::Term).repeated().at_least(1);

        let int = select! { Token::Num(n) => Expr::Num(n) }.labelled("a number");
        let var = ident().map(Expr::Var);
        let paren = expr.clone()
            .delimited_by(just(Token::ParensOpen), just(Token::ParensClose));

        // `cut: 400` is tried before a bare expression. Both start with an
        // Ident, so this relies on `choice` rewinding when the Colon is absent.
        let arg = choice((
            ident()
                .then_ignore(just(Token::Colon))
                .then(expr.clone())
                .map(|(name, value)| Arg { name: Some(name), value }),
            expr.clone().map(Arg::positional),
        ));

        let args = arg
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>();

        let list = expr.clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::BracketOpen), just(Token::BracketClose))
            .map(Expr::List);

        let call = ident()
            .then(args.clone().delimited_by(just(Token::ParensOpen), just(Token::ParensClose)))
            .map(|(func, args)| Expr::Call { func, args });

        let stmt = choice((
            expr.clone().map(Statement::Expr),
            just(Token::Let)
                .ignore_then(ident())
                .then_ignore(just(Token::Assign))
                .then(expr.clone())
                .map(|(name, value)| Statement::Let { name, value: Box::new(value) }),
        ));

        let block = stmt.clone()
            .separated_by(sep.clone())
            .allow_leading()
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
            .try_map(|mut stmts, span| match stmts.pop() {
                Some(Statement::Expr(tail)) =>
                    Ok(Expr::Block { stmts, tail: Box::new(tail) }),
                _ => Err(Rich::custom(span, "a block must end in an expression")),
            });

        let for_expr = just(Token::For)
            .ignore_then(ident())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(block.clone())
            .map(|((var, iter), body)| Expr::For {
                var, iter: Box::new(iter), body: Box::new(body)
            });
        
        let if_expr = recursive(|if_expr| {
            just(Token::If)
                .ignore_then(expr.clone())
                .then(block.clone())
                .then(
                    just(Token::Else)
                        .ignore_then(choice((if_expr.clone(), block.clone())))
                        .or_not()
                )
                .map(|((cond, then), otherwise)| Expr::If {
                    cond: Box::new(cond),
                    then: Box::new(then),
                    otherwise: otherwise.map(Box::new),
                })
        });

        let let_expr = just(Token::Let)
            .ignore_then(ident())
            .then_ignore(just(Token::Assign))
            .then(expr.clone())
            .then_ignore(just(Token::In)) 
            .then(expr.clone())
            .map(|((name, value), body)| Expr::Let {
                name, value: Box::new(value), body: Box::new(body),
            });

        // `call` must be tried before `var`: both start with an Ident, and
        // choice commits to the first success — var-first would leave `(...)`
        // unconsumed and split `sin(440)` into two items.
        let rest = just(Token::Rest).to(Expr::Rest);
        let trigger = just(Token::Trigger).to(Expr::Trigger);

        let atom = choice((
            int, rest, trigger, list, for_expr, if_expr, let_expr, block, call, var, paren,
        ));

        // Indexing and method calls share one fold so they interleave freely:
        // `xs[0].m2h`, `x.scale(s)[1]`.
        //
        // `a.f(b)` is desugared here into the same `Chain` node `a >> f(b)`
        // produces — the two operators mean exactly the same thing and differ
        // only in binding. Nothing downstream needs to know `.` exists.
        let method = just(Token::Dot)
            .ignore_then(ident())
            .then(
                args.clone()
                    .delimited_by(just(Token::ParensOpen), just(Token::ParensClose))
                    .or_not(),
            )
            .map(|(func, args)| Postfix::Method(func, args.unwrap_or_default()));

        let index = expr
            .clone()
            .delimited_by(just(Token::BracketOpen), just(Token::BracketClose))
            .map(Postfix::Index);

        let postfix = atom.foldl(
            choice((method, index)).repeated(),
            |base, step| match step {
                Postfix::Index(index) => Expr::Index {
                    base: Box::new(base),
                    index: Box::new(index),
                },
                Postfix::Method(func, args) => Expr::Chain {
                    lhs: Box::new(base),
                    rhs: Box::new(Expr::Call { func, args }),
                },
            },
        );

        let unary = just(Token::Sub)
            .repeated()
            .foldr(postfix, |_, rhs| Expr::Neg { expr: Box::new(rhs) });

        let product = unary.clone().foldl(
            choice((
                just(Token::Mul).to(ProductOp::Mul),
                just(Token::Div).to(ProductOp::Div),
                just(Token::Percent).to(ProductOp::Rem),
            ))
            .then(unary)
            .repeated(),
            |lhs, (op, rhs)| match op {
                ProductOp::Mul => Expr::Mul { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                ProductOp::Div => Expr::Div { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                ProductOp::Rem => Expr::Rem { lhs: Box::new(lhs), rhs: Box::new(rhs) },
            },
        );

        let sum = product.clone().foldl(
            choice((
                just(Token::Ad).to(SumOp::Add),
                just(Token::Sub).to(SumOp::Sub),
            ))
            .then(product)
            .repeated(),
            |lhs, (op, rhs)| match op {
                SumOp::Add => Expr::Add { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                SumOp::Sub => Expr::Sub { lhs: Box::new(lhs), rhs: Box::new(rhs) },
            },
        );

        let compare = sum.clone().foldl(
                choice((
                    just(Token::Le).to(CmpOp::Le),
                    just(Token::Lt).to(CmpOp::Lt),
                    just(Token::Ge).to(CmpOp::Ge),
                    just(Token::Gt).to(CmpOp::Gt),
                    just(Token::EqEq).to(CmpOp::Eq),
                    just(Token::Ne).to(CmpOp::Ne),
                ))
                .then(sum.clone())
                .repeated(),
                |lhs, (op, rhs)| Expr::Cmp { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
        );

        let range = compare.clone()
            .then(just(Token::DotDotEq).ignore_then(compare.clone()).or_not())
            .map(|(lo, hi)| match hi {
                Some(hi) => Expr::Range { lo: Box::new(lo), hi: Box::new(hi) },
                None => lo,
        });

        let chain = range.clone().foldl(
            just(Token::ShiftRight).ignore_then(compare.clone()).repeated(),
            |lhs, rhs| Expr::Chain { lhs: Box::new(lhs), rhs: Box::new(rhs) }
        );

        chain
    })
}

fn parser<'a, I>() -> impl Parser<'a, I, Vec<ScreeItem>, extra::Err<Rich<'a, Token>>>
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {

    let param = ident()
        .then(
            just(Token::Assign)
                .ignore_then(expr())
                .or_not()
        )
        .map(|(name, default)| Param { name, default });

    let params = param
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParensOpen), just(Token::ParensClose));

    let function = just(Token::Function)
        .ignore_then(ident())
        .then(params)
        .then_ignore(just(Token::Assign))
        .then(expr())
        .map(|((name, params), body)| ScreeItem::Function { name, params, body });

    let let_item = just(Token::Let)
        .ignore_then(ident())
        .then_ignore(just(Token::Assign))
        .then(expr())
        .map(|(name, value)| ScreeItem::Let { name, value });

    let item = choice((
        function,
        expr().map(ScreeItem::Expr),
        let_item,
    ));

    just(Token::Term)
        .repeated()
        .ignore_then(item)
        .then_ignore(just(Token::Term).repeated())
        .repeated()
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str) -> Box<Expr> {
        Box::new(Expr::Var(Ident(name.to_string())))
    }

    /// `fn add(a, b) = a + b` — a function whose body adds its two parameters.
    #[test]
    fn parses_function_adding_two_vars() {
        let ast = parse("fn add(a, b) = a + b\n".to_string()).expect("should parse");

        let expected = vec![ScreeItem::Function {
            name: Ident("add".to_string()),
            params: vec![
                Param { name: Ident("a".to_string()), default: None },
                Param { name: Ident("b".to_string()), default: None },
            ],
            body: Expr::Add { lhs: var("a"), rhs: var("b") },
        }];

        assert_eq!(ast, expected);
    }

    /// The one argument list, parsed both ways at once.
    #[test]
    fn parses_named_and_positional_arguments() {
        let ast = parse("play(kick, cut: 400)\n".to_string()).expect("should parse");

        let expected = vec![ScreeItem::Expr(Expr::Call {
            func: Ident("play".to_string()),
            args: vec![
                Arg::positional(Expr::Var(Ident("kick".to_string()))),
                Arg::named("cut", Expr::Num(400.0)),
            ],
        })];

        assert_eq!(ast, expected);
    }

    /// A named argument's value is a whole expression, not just a literal —
    /// which is what makes a lane able to be a pattern.
    #[test]
    fn a_named_argument_takes_any_expression() {
        let ast = parse("play(bass, cut: [400, 2000])\n".to_string()).expect("should parse");

        let Some(ScreeItem::Expr(Expr::Call { args, .. })) = ast.first() else {
            panic!("expected a call, got {ast:?}");
        };
        assert_eq!(args[1].name, Some(Ident("cut".to_string())));
        assert_eq!(args[1].value, Expr::List(vec![Expr::Num(400.0), Expr::Num(2000.0)]));
    }

    /// `choice` has to rewind after the Ident when no Colon follows, or a bare
    /// variable argument stops parsing.
    #[test]
    fn a_bare_identifier_argument_still_parses() {
        let ast = parse("sin(freq)\n".to_string()).expect("should parse");

        let Some(ScreeItem::Expr(Expr::Call { args, .. })) = ast.first() else {
            panic!("expected a call, got {ast:?}");
        };
        assert_eq!(args, &vec![Arg::positional(Expr::Var(Ident("freq".to_string())))]);
    }

    /// Method sugar shares the argument parser, so names work there too.
    #[test]
    fn method_calls_take_named_arguments() {
        let ast = parse("[220] >> play(bass, cut: 400)\n".to_string()).expect("should parse");

        let Some(ScreeItem::Expr(Expr::Chain { rhs, .. })) = ast.first() else {
            panic!("expected a chain, got {ast:?}");
        };
        let Expr::Call { args, .. } = rhs.as_ref() else {
            panic!("expected a call on the right of the chain");
        };
        assert_eq!(args[1], Arg::named("cut", Expr::Num(400.0)));
    }

    /// The whole point of the diagnostic: the error has to say where it is,
    /// and say it in the source's own spelling rather than the token enum's.
    #[test]
    fn a_parse_error_points_at_the_line_it_is_on() {
        let err = parse("let a = 1\nfn foo(a b) = a\nsin(220)\n".to_string())
            .expect_err("a missing comma should not parse");

        assert_eq!(err.stage, Stage::Parse);
        assert_eq!(err.line, Some(2));
        assert_eq!(err.column, Some(10));
        assert_eq!(err.snippet.as_deref(), Some("fn foo(a b) = a"));
        assert_eq!(err.message, "unexpected `b`, expected `)`, `,` or `=`");
    }

    /// An unclosed bracket holds the statement open, so the error lands on the
    /// line that finally could not continue it — not on the line that opened.
    /// Reported honestly rather than guessed at, since the parser has no idea
    /// which of the two the writer meant.
    #[test]
    fn an_unclosed_call_reports_where_it_gave_up() {
        let err = parse("let a = 1\nsin(440\nlet b = 2\n".to_string())
            .expect_err("an unclosed call should not parse");

        assert_eq!(err.line, Some(3));
        assert!(
            err.message.starts_with("unexpected `let`, expected `)`"),
            "the closing paren should lead the list: {}",
            err.message,
        );
    }

    /// A character the language has no token for stops at that character,
    /// rather than at the start of the file.
    #[test]
    fn an_unknown_character_reports_itself() {
        let err = parse("sin(440)\nlet x = 1 @ 2\n".to_string())
            .expect_err("`@` should not lex");

        assert_eq!(err.stage, Stage::Lex);
        assert_eq!(err.line, Some(2));
        assert_eq!(err.column, Some(11));
        assert!(err.message.contains('@'), "unhelpful message: {}", err.message);
    }

    /// Trailing input must not parse quietly: before this, half a program
    /// could reach the engine with the rest of it dropped on the floor.
    #[test]
    fn a_stray_bracket_is_an_error_rather_than_a_short_program() {
        parse("sin(440)\n]\n".to_string()).expect_err("a stray `]` should not parse");
    }

    /// A block that ends in a `let` fails with the parser's own words, which
    /// arrive by a different route than the expected/found errors.
    #[test]
    fn a_custom_parser_error_keeps_its_message() {
        let err = parse("{ let a = 1 }\n".to_string())
            .expect_err("a block ending in a let should not parse");

        assert!(
            err.message.contains("must end in an expression"),
            "lost the parser's message: {}",
            err.message,
        );
    }
}
