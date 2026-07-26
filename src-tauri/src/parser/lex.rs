use logos::Logos;


#[derive(Logos, Debug, PartialEq, Clone)]
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

    #[token("else")]
    Else,

    #[token("==")]
    EqEq,
    
    #[token("for")]
    For,

    #[token("fn")]
    Function,
    
    #[token(">=")]
    Ge,

    #[token(">")]
    Gt,
    
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    #[token("if")]
    If,

    #[token("in")]
    In,

    #[token("<=")]
    Le,
    
    #[token("let")]
    Let,

    #[token("<")]
    Lt,

    #[token("*")]
    Mul,

    #[token("!=")]
    Ne,

    #[token("\n")]
    NewLine,

    #[token("null")]
    Null,

    #[regex(r"(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?", |lex| lex.slice().parse::<f64>().ok())]
    Num(f64),

    #[token("(")]
    ParensOpen,

    #[token(")")]
    ParensClose,

    /// A rest inside a pattern: `[220, `, 330, `]`.
    #[token("`")]
    Rest,

    #[token("%")]
    Percent,

    #[token(";")]
    Semi,

    #[token(">>")]
    ShiftRight,

    #[token("-")]
    Sub,

    Term,
}

pub fn insert_terminators(raw: Vec<Token>) -> Vec<Token> {

    fn can_end(t: &Token) -> bool {
        matches!(t,
            Token::Ident(_) | Token::Num(_) | Token::Rest |
            Token::BraceClose | Token::BracketClose | Token::ParensClose
        )
    }

    fn cont_next(t: &Token) -> bool {
        matches!(t,
            // BracketOpen is deliberately absent: a line starting with `[`
            // begins a new statement (a pattern), it does not index the
            // previous line's value.
            Token::Ad | Token::Assign | Token::BraceOpen |
            Token::Colon |
            Token::Comma | Token::Div | Token::DotDotEq |
            Token::Else | Token::EqEq | Token::Ge | Token::Gt |
            Token::Le | Token::Lt | Token::Mul | Token::Ne |
            Token::Percent | Token::ShiftRight | Token::Sub
        )
    }

    fn convert(t: Token) -> Token {
        match t {
            Token::NewLine => unreachable!("new line handled separately"),
            _ => t,
        }
    }

    let mut out = Vec::with_capacity(raw.len());
    let mut prev : Option<Token> = None;

    let mut depth: i32 = 0;
    let mut i = 0;

    while i < raw.len() {
        let tok = &raw[i];
        match tok {
            Token::ParensOpen | Token::BracketOpen => depth += 1,
            Token::ParensClose | Token::BracketClose => depth -= 1,
            _ => {}
        }

        if *tok == Token::NewLine {
            i += 1;
            if depth > 0 { continue; }

            let ends = prev.as_ref().map_or(false, can_end);
            if !ends { continue; }

            let mut j = i;
            while j < raw.len() && raw[j] == Token::NewLine { j += 1; }

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