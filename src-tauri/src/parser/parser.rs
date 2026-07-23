use chumsky::prelude::*;
use chumsky::input::ValueInput;
use logos::Logos;
use crate::parser::lex::{insert_terminators, Token};

#[derive(Clone, Debug, PartialEq)]
pub struct Ident(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct Param { pub name: Ident, pub default: Option<Expr> }

#[derive(Clone, Debug, PartialEq)]
pub enum BrapItem {
    Function { name: Ident, params: Vec<Param>, body: Expr },
    Let { name: Ident, value: Expr },
    Call { func: Ident, args: Vec<Expr> },
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Ad   { lhs: Box<Expr>, rhs: Box<Expr> },
    Block { stmts: Vec<Statement>, tail: Box<Expr> },
    Call  { func: Ident, args: Vec<Expr> },
    Chain { lhs: Box<Expr>, rhs: Box<Expr> },
    Div   { lhs: Box<Expr>, rhs: Box<Expr> },
    Num(f64),
    Let { name: Ident, value: Box<Expr>, body: Box<Expr> },
    Mul   { lhs: Box<Expr>, rhs: Box<Expr> },
    Neg { expr: Box<Expr> },
    Sub   { lhs: Box<Expr>, rhs: Box<Expr> },
    Var(Ident),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Let { name: Ident, value: Box<Expr> },
    Expr(Expr),
}

#[allow(dead_code)]
enum Pattern {
    Ident(Ident)
}

#[allow(dead_code)]
enum Range { Const(i64, i64) }

#[derive(Clone, Debug)]
enum BinOp {
    Mul, Div, Add, Sub
}

fn ident<'a, I>() -> impl Parser<'a, I, Ident, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    select! { Token::Ident(s) => Ident(s) }
}

pub fn parse(code: String) -> Result<Vec<BrapItem>, String> {
    let raw_tokens: Vec<Token> = Token::lexer(&code)
        .collect::<Result<_, _>>()
        .map_err(|_| "lexing error".to_string())?;

    let tokens: Vec<Token> = insert_terminators(raw_tokens);

    let result = parser()
        .parse(&tokens[..])
        .into_result()
        .map_err(|errs| format!("{:?}", errs));

    result
}

fn expr<'a, I>() -> impl Parser<'a, I, Expr, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    recursive(|expr| {
        let sep = just(Token::Term).repeated().at_least(1);

        let int = select! { Token::Num(n) => Expr::Num(n) };
        let var = ident().map(Expr::Var);
        let paren = expr.clone()
            .delimited_by(just(Token::ParensOpen), just(Token::ParensClose));

        let args = expr.clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>();

        let call = ident()
            .then(args.delimited_by(just(Token::ParensOpen), just(Token::ParensClose)))
            .map(|(func, args)| Expr::Call { func, args });

        let stmt = choice((
            just(Token::Let)
                .ignore_then(ident())
                .then_ignore(just(Token::Assign))
                .then(expr.clone())
                .map(|(name, value)| Statement::Let { name, value: Box::new(value) }),
            expr.clone().map(Statement::Expr),
        ));

        let block = stmt.clone()
            .then_ignore(sep.clone())
            .repeated()
            .collect::<Vec<_>>()
            .then(expr.clone())
            .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
            .map(|(stmts, tail)| Expr::Block { stmts, tail: Box::new(tail) });

        let atom = choice((
            int, block, var, paren, call
        ));

        let unary = just(Token::Sub)
            .repeated()
            .foldr(atom, |_, rhs| Expr::Neg { expr: Box::new(rhs) });

        let product = unary.clone().foldl(
            choice((
                just(Token::Mul).to(BinOp::Mul),
                just(Token::Div).to(BinOp::Div),
            ))
            .then(unary)
            .repeated(),
            |lhs, (op, rhs)| match op {
                BinOp::Mul => Expr::Mul { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                BinOp::Div => Expr::Div { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                _ => unreachable!(),
            },
        );

        let sum = product.clone().foldl(
            choice((
                just(Token::Ad).to(BinOp::Add),
                just(Token::Sub).to(BinOp::Sub),
            ))
            .then(product)
            .repeated(),
            |lhs, (op, rhs)| match op {
                BinOp::Add => Expr::Ad { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                BinOp::Sub => Expr::Sub { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                _ => unreachable!(),
            },
        );

        let chain = sum.clone().foldl(
            just(Token::ShiftRight).ignore_then(sum.clone()).repeated(),
            |lhs, rhs| Expr::Chain { lhs: Box::new(lhs), rhs: Box::new(rhs) }
        );

        chain
    })
}

fn parser<'a, I>() -> impl Parser<'a, I, Vec<BrapItem>, extra::Err<Rich<'a, Token>>>
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
        .map(|((name, params), body)| BrapItem::Function { name, params, body });

    let let_item = just(Token::Let)
        .ignore_then(ident())
        .then_ignore(just(Token::Assign))
        .then(expr())
        .map(|(name, value)| BrapItem::Let { name, value });

    let item = choice((
        function,
        let_item,
        expr().map(BrapItem::Expr),
    ));

    // A program is a sequence of items, each optionally surrounded by
    // terminators (statement breaks the lexer inserts for newlines).
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

        let expected = vec![BrapItem::Function {
            name: Ident("add".to_string()),
            params: vec![
                Param { name: Ident("a".to_string()), default: None },
                Param { name: Ident("b".to_string()), default: None },
            ],
            body: Expr::Ad { lhs: var("a"), rhs: var("b") },
        }];

        assert_eq!(ast, expected);
    }
}
