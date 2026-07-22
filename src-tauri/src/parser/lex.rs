use logos::Logos;
use chumsky::prelude::*;


#[derive(Logos, Debug, PartialEq)]
#[logos(skip r"[ \t]+")]
#[regex(r"//[^\n]*", logos::skip)]
pub enum Token {
    #[token("+")]
    Ad,

    #[token("=")]
    Assign,

    #[token("{")]
    BraceOpen,

    #[token("}")]
    BraceClose,

    #[token("[")]
    BracketOpen,

    #[token("]")]
    BracketClose,

    #[token(":")]
    Colon,

    #[token(",")]
    Comma,

    #[token("/")]
    Div,

    #[token("..=")]
    DotDotEq,

    #[token("fn")]
    Function,

    #[regex("[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident(String),

    #[regex("[0-9]+", |lex| lex.slice().parse::<i64>())]
    Int(i64),

    #[token("let")]
    Let,

    #[token("*")]
    Mul,

    #[token("\n")]
    NewLine,

    #[token("null")]
    Null,

    #[token("(")]
    ParensOpen,

    #[token(")")]
    ParensClose,

    #[token(";")]
    Semi,

    #[token("-")]
    Sub,

    Term,
}

pub fn insert_terminators(raw: Vec<Token>) -> Vec<Token> {

    fn can_end(t: Token) -> bool {
        matches!(t,
            Token::Ident(_) | Token::Int(_) |
            Token::BraceClose | Token::ParensClose
        )
    }

    fn cont_next(t: &Token) -> bool {
        matches!(t,
            Token::Ad | Token::Assign | Token::BraceOpen |
            Token::BracketOpen | Token::Colon |
            Token::Comma | Token::Div | Token::DotDotEq |
            Token::Mul | Token::Sub
        )
    }

    fn convert(t: Token) -> Token {
        match t {
            Token::NewLine => unreachable!("new line handled separately")
            _ => t
        }
    }

    let mut out = Vec::with_capacity(raw.len());
    let mut prev : Option<Token> = None;

    let mut depth: i32 = 0;
    let mut i = 0;

    while i < *raw.len() {
        let tok = &raw[i];
        match tok {
            Token::ParensOpen => depth += 1,
            Token::ParensClose => depth -= 1,
            _ => {}
        }

        if *tok == Token::NewLine {
            i += 1;
            if *depth > 0 { continue; }

            let ends = prev.as_ref().map_or(false, can_end);
            if !ends { continue; }

            let mut j = i;
            while j < *raw.len() && raw[j] == Token::NewLine { j += 1; }

            if let Some(nxt) = raw.get(j) {
                if cont_next(nxt) { continue; }
            }

            out.push(Token::Term);
            prev = Some(Token::NewLine);
            continue;
        }

        out.push(convert(tok.clone()));
        prev = Some(tok.clone());
        i += 1;
    }

    out
}