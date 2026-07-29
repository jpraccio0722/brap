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
    /// `use drums::kick` — another file's definitions, named here.
    ///
    /// Nothing downstream ever sees one: `imports::expand` replaces every
    /// `use` with the definitions it names before the program is lowered.
    Use(Use),
}

/// One name brought in by a `use`, and what it is called here: `kick`, or
/// `kick as thump`.
#[derive(Clone, Debug, PartialEq)]
pub struct UseName { pub name: Ident, pub alias: Option<Ident> }

/// What a `use` takes from the path it names.
#[derive(Clone, Debug, PartialEq)]
pub enum UseTree {
    /// `use lib::drums` — whichever of the module or the single item the path
    /// turns out to name; only the filesystem can say which. The `Ident` is an
    /// `as` alias, if one was written.
    Plain(Option<Ident>),
    /// `use lib::drums::{kick, snare as clap}`.
    Names(Vec<UseName>),
    /// `use lib::drums::*` — everything the module defines.
    Glob,
}

/// One `use` item.
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    /// The path as written, split on `::`.
    pub path: Vec<Ident>,
    pub tree: UseTree,
    /// Byte offset of the `use` keyword, so a module that cannot be found is
    /// reported on the line that asked for it.
    pub at: usize,
}

impl Use {
    /// The path as it was written, for messages.
    pub fn written(&self) -> String {
        self.path.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join("::")
    }
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
    /// A double-quoted string. Only ever the path in `load("break.wav")` —
    /// there is nothing else in the language that takes one, and the lowerer
    /// says so if one turns up anywhere else.
    Str(String),
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

/// The `as` of `use drums as d`.
///
/// A name rather than a token of its own, so it is only a keyword where a
/// keyword is what it looks like: `as` on its own is still an ordinary name,
/// which some file somewhere is bound to be using.
fn kw_as<'a, I>() -> impl Parser<'a, I, (), extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    select! { Token::Ident(name) if name == "as" => () }.labelled("`as`")
}

/// A name, qualified by the module it came from if it needs to be:
/// `kick`, `drums::kick`.
///
/// The qualifier is kept in the `Ident` rather than beside it, because a
/// qualified name is still just a name to everything downstream — `use`
/// resolution rewrites it to the one the definition was filed under, and the
/// lowerer looks it up like any other.
fn name<'a, I>() -> impl Parser<'a, I, Ident, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    ident()
        .then(
            just(Token::PathSep)
                .ignore_then(ident())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(head, rest)| {
            if rest.is_empty() {
                return head;
            }
            let mut joined = head.0;
            for segment in rest {
                joined.push_str("::");
                joined.push_str(&segment.0);
            }
            Ident(joined)
        })
}

/// `{ let a = f * 2\n sin(a) }` — statements, then the expression the block is
/// worth.
///
/// Takes the expression parser rather than calling `expr` itself so the copy
/// inside `expr` can recurse through the recursive handle, while a function
/// body — which is not inside an expression yet — can build its own.
fn block<'a, I, E>(expr: E) -> impl Parser<'a, I, Expr, extra::Err<Rich<'a, Token>>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan>,
    E: Parser<'a, I, Expr, extra::Err<Rich<'a, Token>>> + Clone,
{
    let sep = just(Token::Term).repeated().at_least(1);

    let stmt = choice((
        expr.clone().map(Statement::Expr),
        just(Token::Let)
            .ignore_then(ident())
            .then_ignore(just(Token::Assign))
            .then(expr)
            .map(|(name, value)| Statement::Let { name, value: Box::new(value) }),
    ));

    stmt.separated_by(sep)
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
        .try_map(|mut stmts, span| match stmts.pop() {
            Some(Statement::Expr(tail)) => Ok(Expr::Block { stmts, tail: Box::new(tail) }),
            _ => Err(Rich::custom(span, "a block must end in an expression")),
        })
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

    result.map(|mut items| {
        // A `use` carried the index of its own token out of the parser, which
        // is only a place in the file while the spans are still in scope.
        for item in &mut items {
            if let ScreeItem::Use(u) = item {
                u.at = spans.get(u.at).map_or(0, |s| s.start);
            }
        }
        items
    })
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
                | Token::In | Token::Let | Token::Use => 2,
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

/// One `::`-separated piece of a `use` path: another name, or the braced list
/// that can only come last.
enum Segment {
    Name(Ident),
    Names(Vec<UseName>),
}

/// Assemble a `use` from its pieces, refusing the combinations that have no
/// meaning — a braced list mid-path, or a `*` and an `as` on the same item.
///
/// `at` is a token index here; `parse` maps it back to a byte offset once the
/// whole file has been read, which is the only place the spans still exist.
fn build_use(
    head: Ident,
    segments: Vec<Segment>,
    glob: bool,
    alias: Option<Ident>,
    at: usize,
) -> Result<Use, String> {
    let mut path = vec![head];
    let mut names: Option<Vec<UseName>> = None;

    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.into_iter().enumerate() {
        match segment {
            Segment::Name(name) => path.push(name),
            Segment::Names(_) if i != last => {
                return Err("a `{…}` list has to be the last part of a `use`".to_string())
            }
            Segment::Names(list) => names = Some(list),
        }
    }

    let tree = match (names, glob, alias) {
        (Some(_), true, _) | (Some(_), _, Some(_)) => {
            return Err("a `{…}` list already names what it takes".to_string())
        }
        (None, true, Some(_)) => {
            return Err("`use …::*` takes every name as it is written, so there \
                        is nothing for `as` to rename".to_string())
        }
        (Some(list), false, None) => UseTree::Names(list),
        (None, true, None) => UseTree::Glob,
        (None, false, alias) => UseTree::Plain(alias),
    };

    Ok(Use { path, tree, at })
}

fn expr<'a, I>() -> impl Parser<'a, I, Expr, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    recursive(|expr| {
        let int = select! { Token::Num(n) => Expr::Num(n) }.labelled("a number");
        let string = select! { Token::Str(s) => Expr::Str(s) }.labelled("a string");
        let var = name().map(Expr::Var);
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

        let call = name()
            .then(args.clone().delimited_by(just(Token::ParensOpen), just(Token::ParensClose)))
            .map(|(func, args)| Expr::Call { func, args });

        let block = block(expr.clone());

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
            int, string, rest, trigger, list, for_expr, if_expr, let_expr, block, call, var,
            paren,
        ));

        // Indexing and method calls share one fold so they interleave freely:
        // `xs[0].m2h`, `x.scale(s)[1]`.
        //
        // `a.f(b)` is desugared here into the same `Chain` node `a >> f(b)`
        // produces — the two operators mean exactly the same thing and differ
        // only in binding. Nothing downstream needs to know `.` exists.
        let method = just(Token::Dot)
            // Qualified, so an imported function chains like any other:
            // `n.m2h.drums::bass()`.
            .ignore_then(name())
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

    // Two spellings of one thing: `fn kick(f) = sin(f)` for a body that fits on
    // the line, and `fn kick(f) { ... }` for one with statements in it. The
    // braced form is the block expression the `=` form would have been handed,
    // written without the `=` that would introduce it.
    let body = choice((
        just(Token::Assign).ignore_then(expr()),
        block(expr()),
    ));

    let function = just(Token::Function)
        .ignore_then(ident())
        .then(params)
        .then(body)
        .map(|((name, params), body)| ScreeItem::Function { name, params, body });

    let let_item = just(Token::Let)
        .ignore_then(ident())
        .then_ignore(just(Token::Assign))
        .then(expr())
        .map(|(name, value)| ScreeItem::Let { name, value });

    // `use lib::drums`, `use lib::drums as d`, `use lib::drums::kick`,
    // `use lib::drums::{kick, snare as clap}`, `use lib::drums::*`.
    //
    // The path is read as one repetition rather than as "path, then optional
    // suffix" so nothing ever has to be un-consumed: after each `::` the next
    // thing is either another segment or the braced list that ends the item.
    let use_name = ident()
        .then(kw_as().ignore_then(ident()).or_not())
        .map(|(name, alias)| UseName { name, alias });

    let use_names = use_name
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose));

    let segment = just(Token::PathSep).ignore_then(choice((
        ident().map(Segment::Name),
        use_names.map(Segment::Names),
    )));

    let use_item = just(Token::Use)
        .ignore_then(ident())
        .then(segment.repeated().collect::<Vec<_>>())
        .then(just(Token::PathGlob).or_not())
        .then(kw_as().ignore_then(ident()).or_not())
        .try_map_with(|(((head, segments), glob), alias), e| {
            let span: SimpleSpan = e.span();
            build_use(head, segments, glob.is_some(), alias, span.start)
                .map(ScreeItem::Use)
                .map_err(|message| Rich::custom(span, message))
        });

    let item = choice((
        use_item,
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

    /// The same function, braced. The body is the block the `=` form would
    /// have been given explicitly, so it holds one expression and no statements.
    #[test]
    fn parses_a_braced_function_body() {
        let ast = parse("fn add(a, b) { a + b }\n".to_string()).expect("should parse");

        let expected = vec![ScreeItem::Function {
            name: Ident("add".to_string()),
            params: vec![
                Param { name: Ident("a".to_string()), default: None },
                Param { name: Ident("b".to_string()), default: None },
            ],
            body: Expr::Block {
                stmts: Vec::new(),
                tail: Box::new(Expr::Add { lhs: var("a"), rhs: var("b") }),
            },
        }];

        assert_eq!(ast, expected);
    }

    /// The point of the braced form: statements ahead of the value, over lines.
    #[test]
    fn a_braced_body_holds_statements() {
        let ast = parse(
            "fn kick(f) {\n  let a = f * 2\n  sin(a)\n}\n".to_string()
        ).expect("should parse");

        let Some(ScreeItem::Function { body: Expr::Block { stmts, tail }, .. }) = ast.first()
        else {
            panic!("expected a function with a block body, got {ast:?}");
        };
        assert_eq!(stmts.len(), 1, "the `let` should be a statement of the block");
        assert_eq!(**tail, Expr::Call {
            func: Ident("sin".to_string()),
            args: vec![Arg::positional(Expr::Var(Ident("a".to_string())))],
        });
    }

    /// Defaults and the braced body are independent — one must not cost the
    /// other, since both hang off the same `=`.
    #[test]
    fn a_braced_body_still_takes_parameter_defaults() {
        let ast = parse("fn bass(n, cut = 800) { saw(n) }\n".to_string()).expect("should parse");

        let Some(ScreeItem::Function { params, body, .. }) = ast.first() else {
            panic!("expected a function, got {ast:?}");
        };
        assert_eq!(params[1].default, Some(Expr::Num(800.0)));
        assert!(matches!(body, Expr::Block { .. }));
    }

    /// A block is worth its last expression, so there has to be one.
    #[test]
    fn an_empty_braced_body_is_an_error() {
        assert!(parse("fn kick(f) { }\n".to_string()).is_err());
        assert!(parse("fn kick(f) { let a = 1 }\n".to_string()).is_err());
    }

    /// Both forms mean the same thing to everything downstream — which is what
    /// lets the body be a block without anything else knowing.
    #[test]
    fn both_function_forms_reach_the_same_value() {
        let one_line = parse("fn f(a) = a * 2\nsin(f(4))\n".to_string()).expect("should parse");
        let braced = parse("fn f(a) { a * 2 }\nsin(f(4))\n".to_string()).expect("should parse");

        let graph_of = |ast| crate::lowerer::lower::lower_graph(&ast).expect("lower failed");
        assert_eq!(graph_of(one_line).nodes, graph_of(braced).nodes);
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

    // ---- use ----

    fn uses(src: &str) -> Vec<Use> {
        parse(src.to_string())
            .unwrap_or_else(|e| panic!("{src} should parse: {e}"))
            .into_iter()
            .filter_map(|item| match item {
                ScreeItem::Use(u) => Some(u),
                _ => None,
            })
            .collect()
    }

    fn id(name: &str) -> Ident {
        Ident(name.to_string())
    }

    /// The four shapes, and what each of them takes.
    #[test]
    fn parses_every_use_shape() {
        let u = &uses("use drums\n")[0];
        assert_eq!(u.path, vec![id("drums")]);
        assert_eq!(u.tree, UseTree::Plain(None));

        let u = &uses("use lib::drums as d\n")[0];
        assert_eq!(u.path, vec![id("lib"), id("drums")]);
        assert_eq!(u.tree, UseTree::Plain(Some(id("d"))));

        let u = &uses("use lib::drums::{kick, snare as clap}\n")[0];
        assert_eq!(u.path, vec![id("lib"), id("drums")]);
        assert_eq!(
            u.tree,
            UseTree::Names(vec![
                UseName { name: id("kick"), alias: None },
                UseName { name: id("snare"), alias: Some(id("clap")) },
            ])
        );

        let u = &uses("use lib::drums::*\n")[0];
        assert_eq!(u.path, vec![id("lib"), id("drums")]);
        assert_eq!(u.tree, UseTree::Glob);
    }

    /// `::*` is one token so the line it ends is over. Without that, the next
    /// line reads as the right-hand side of a multiplication.
    #[test]
    fn a_glob_ends_its_line() {
        let items = parse("use drums::*\nsin(220)\n".to_string()).expect("should parse");
        assert_eq!(items.len(), 2);
        assert!(matches!(items[1], ScreeItem::Expr(Expr::Call { .. })));
    }

    /// A `use` says where it is, so a module that cannot be found is reported
    /// on the line that asked for it rather than at the top of the file.
    #[test]
    fn a_use_remembers_where_it_was_written() {
        let src = "sin(220)\nuse drums\n";
        let u = &uses(src)[0];
        assert_eq!(&src[u.at..u.at + 3], "use");
    }

    /// Combinations with no meaning are refused where they are written, in the
    /// parser's own words.
    #[test]
    fn nonsense_use_shapes_are_refused() {
        let err = parse("use a::{b}::c\n".to_string()).expect_err("list mid-path");
        assert!(err.message.contains("last part"), "got: {}", err.message);

        let err = parse("use a::* as b\n".to_string()).expect_err("glob and alias");
        assert!(err.message.contains("nothing for `as`"), "got: {}", err.message);
    }

    /// `as` is only a keyword in a `use`. Everywhere else it is a name like any
    /// other, and some file somewhere is already using it as one.
    #[test]
    fn as_is_still_an_ordinary_name() {
        let items = parse("let as = 1\nsin(as)\n".to_string()).expect("should parse");
        assert_eq!(items.len(), 2);
    }

    /// A qualified name is one name, not two: everything downstream looks names
    /// up by string, and `drums::kick` is the string it looks up.
    #[test]
    fn a_qualified_name_is_one_name() {
        let items = parse("play([50], drums::kick)\n".to_string()).expect("should parse");
        let Some(ScreeItem::Expr(Expr::Call { args, .. })) = items.first() else {
            panic!("expected a call, got {items:?}");
        };
        assert_eq!(args[1].value, Expr::Var(id("drums::kick")));
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
