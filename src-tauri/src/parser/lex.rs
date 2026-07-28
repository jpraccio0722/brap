use logos::Logos;
use std::fmt;

/// Where a token came from, as a byte range into the source text.
///
/// Carried alongside the token stream rather than inside `Token`, which is
/// compared by value all through the parser — a span would make every one of
/// those comparisons position-sensitive.
pub type Span = std::ops::Range<usize>;

#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(skip r"[ \t]+")]

#[logos(skip(r"//[^\n]*", allow_greedy = true))]
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

    /// Method sugar: `60.m2h`. One character, so `..=` wins by longest match
    /// and `.5` still lexes as a number.
    #[token(".")]
    Dot,

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

    /// A sounding step carrying no data, written as a single backslash.
    #[token("\\")]
    Trigger,

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

/// A token as it is written, so parse errors can quote the source rather than
/// the enum: "expected `,` or `]`" instead of "expected Comma or BracketClose".
///
/// chumsky's `Rich` errors are only printable through this impl — without it
/// the parser's own message is the `Debug` spelling of a whole error vector.
impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Token::Ad => "+",
            Token::Assign => "=",
            Token::BraceOpen => "{",
            Token::BraceClose => "}",
            Token::BracketOpen => "[",
            Token::BracketClose => "]",
            Token::Colon => ":",
            Token::Comma => ",",
            Token::Div => "/",
            Token::Dot => ".",
            Token::DotDotEq => "..=",
            Token::Else => "else",
            Token::EqEq => "==",
            Token::For => "for",
            Token::Function => "fn",
            Token::Ge => ">=",
            Token::Gt => ">",
            Token::Ident(name) => return write!(f, "{name}"),
            Token::If => "if",
            Token::In => "in",
            Token::Le => "<=",
            Token::Let => "let",
            Token::Lt => "<",
            Token::Mul => "*",
            Token::Ne => "!=",
            Token::NewLine => "a line break",
            Token::Null => "null",
            Token::Num(n) => return write!(f, "{n}"),
            Token::ParensOpen => "(",
            Token::ParensClose => ")",
            Token::Rest => "`",
            Token::Trigger => "\\",
            Token::Percent => "%",
            Token::Semi => ";",
            Token::ShiftRight => ">>",
            Token::Sub => "-",
            // Never written down: the lexer inserts it at the end of a line
            // that stands on its own, so name it the way a reader would.
            Token::Term => "the end of the line",
        };
        write!(f, "{s}")
    }
}

/// Insert statement terminators at the line breaks that end a statement, and
/// report where every surviving token came from.
///
/// The spans are what lets a parse error — which chumsky reports as an index
/// into the returned token stream — be turned back into a place in the source.
/// They are returned as a parallel vector rather than paired with the tokens
/// because the parser takes a `&[Token]` slice.
pub fn insert_terminators(raw: Vec<(Token, Span)>) -> (Vec<Token>, Vec<Span>) {

    fn can_end(t: &Token) -> bool {
        matches!(t,
            Token::Ident(_) | Token::Num(_) | Token::Rest | Token::Trigger |
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
            Token::Comma | Token::Div | Token::Dot | Token::DotDotEq |
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
    let mut spans: Vec<Span> = Vec::with_capacity(raw.len());
    let mut prev : Option<Token> = None;

    let mut depth: i32 = 0;
    let mut i = 0;

    while i < raw.len() {
        let (tok, span) = &raw[i];
        match tok {
            Token::ParensOpen | Token::BracketOpen => depth += 1,
            Token::ParensClose | Token::BracketClose => depth -= 1,
            _ => {}
        }

        if *tok == Token::NewLine {
            // The inserted terminator stands where the line break was, so an
            // error against it points at the end of the line it terminates.
            let span = span.clone();
            i += 1;
            if depth > 0 { continue; }

            let ends = prev.as_ref().map_or(false, can_end);
            if !ends { continue; }

            let mut j = i;
            while j < raw.len() && raw[j].0 == Token::NewLine { j += 1; }

            if let Some((nxt, _)) = raw.get(j) {
                if cont_next(nxt) { continue; }
            }

            out.push(Token::Term);
            spans.push(span);
            prev = Some(Token::NewLine);
            continue;
        }

        out.push(convert(tok.clone()));
        spans.push(span.clone());
        prev = Some(tok.clone());
        i += 1;
    }

    (out, spans)
}