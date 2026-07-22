use chumsky::prelude::*;
use chumsky::input::ValueInput;
use crate::parser::lex::{insert_terminators, Token};
use std::clone::Clone;
use crate::parser::lex::Token::{BraceClose, ParensClose};
use crate::parser::parser::Expr::Var;

#[derive(Clone, Debug)]
struct Ident(String);

#[derive(Clone, Debug)]
pub struct Param { pub name: Ident, pub default: Option<Expr> }

pub enum BrapItem {
    Function { name: Ident, params: Vec(Param), body: Expr },
    Let { name: Ident, value: Expr },
    Call { func: Ident, args: Vec<Expr> },
    Expr(Expr)
}

#[derive(Clone, Debug)]
pub enum Expr {
    Ad   { lhs: Box<Expr>, rhs: Box<Expr> },
    Block { stmts: Vec<Statement>, tail: Box<Expr> },
    Call  { func: Ident, args: Vec<Expr> },
    Div   { lhs: Box<Expr>, rhs: Box<Expr> },
    Int(i64),
    Let { name: Ident, value: Box<Expr>, body: Box<Expr> },
    Mul   { lhs: Box<Expr>, rhs: Box<Expr> },
    Neg { expr: Box<Expr> },
    Sub   { lhs: Box<Expr>, rhs: Box<Expr> },
    Var(Ident),
}

#[derive(Clone, Debug)]
pub enum Statement {
    Let { name: Ident, value: Box<Expr> },
    Expr(Expr),
}

enum Pattern {
    Ident(Ident)
}

enum Range { Const(i64, i64) }
#[derive(Clone, Debug)]
enum BinOp {
    Mul, Div, Add, Sub
}

fn ident<'a, I>() -> impl Parser<'a, I, Ident, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {
    select! { Token::Ident(s) => Ident(s) }
}


pub fn parse(code: String) {
    let raw_tokens: Vec<Token> = Token::lexer(code).collect::<Result<_,_>>()?;
    let tokens: Vec<Token> = insert_terminators(raw_tokens);
    let ast = parser().parse(&tokens)?;
}

fn expr<'a, I>() -> impl Parser<'a, I, Expr, extra::Err<Rich<'a, Token>>> + Clone
where I: ValueInput<'a, Token = Token, Span = SimpleSpan> {

    let sep = just(Token::Term).repeated().at_least(1);

    let int = select! { Token::Int(n) => Expr::Int(n) };
    let args = expr.clone()
        .seperated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>();

    let var = ident().map(Expr::Var);
    let paren = expr.clone()
        .delimeted_by(just(Token::ParensOpen), just(Token::ParensClose));

    let stmt = choice((
        just(Token::Let)
            .ignore_then(ident())
            .then_ignore(just(Token::Assign))
            .then(expr.clone())
            .map(|(name, value)| Statement::Let {name, value }),
            expr.clone().map(Statement::Expr),
    ));

    let block = stmt.clone()
        .then_ignore(sep.clone())
        .repeated()
        .collect::<Vec<_>>()
        .then(expr.clone())
        .delimited_by(just(Token::BraceOpen), just(Token::BraceClose))
        .map(|(stmts, tail)| Expr::Block {stmts, tail: Box::new(tail)});

    let atom = choice((
        int, block.clone(), var, paren
    ));

    let unary = just(Token::Sub)
        .repeated()
        .foldr(atom.clone(), |_, rhs| Expr::Neg { expr: Box::new(rhs) });

    let product = unary.clone().foldl(
        choice((
            just(Token::Mul).to(BinOp::Mul),
            just(Token::Div).to(BinOp::Div),
        ))
        .then(unary.clone())
        .repeated(),
        |lhs, (op, rhs) | match op {
            BinOp::Mul => Expr::Mul { lhs: Box::new(lhs), rhs: Box::new(rhs) },
            BinOp::Div => Expr::Div { lhs: Box::new(lhs), rhs: Box::new(rhs) },
        }
    );

    let sum = product.clone().foldl(
        choice((
            just(Token::Ad).to(BinOp::Add),
            just(Token::Sub).to(BinOp::Sub),
        ))
            .then(product.clone())
            .repeated(),
        |lhs, (op, rhs)| match op {
            BinOp::Add => Expr::Ad { lhs: Box::new(lhs), rhs: Box::new(rhs) },
            BinOp::Sub => Expr::Sub { lhs: Box::new(lhs), rhs: Box::new(rhs) },
        },
    );

    sum
}

fn parser<'a>() -> impl Parser<'a, &'a [Token], Vec<BrapItem>, extra::Err<Rich<'a, Token>>> {

    let param = ident()
        .then(
            just(Token::Assign)
                .ignore_then(expr())
                .or_not()
        ).map(|(name, default) | Param { name, default});

    let params = param
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParensOpen), just(ParensClose));

    let function = just(Token::Function)
        .ignore_then(ident())
        .then(params)
        .then_ignore(just(Token::Assign))
        .then(expr())
        .map(|(name, params), body| BrapItem::Function {name, params, body});

}