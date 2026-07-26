//! `play(pattern, instrument, rate = 1)` — binding a pattern to an instrument.
//!
//! `play` is intercepted before argument evaluation because the instrument
//! must be named syntactically: `Binding` stores a name, and a `Value::Function`
//! has already lost it.

use crate::brap_graph::environment::Value;
use crate::lowerer::lower::Lowerer;
use crate::parser::parser::{Expr, Ident};
use crate::pattern::pattern::{Pattern, Step};
use crate::pattern::patterns::Binding;

impl Lowerer {
    /// True when this call should be handled as a `play`.
    pub fn is_play(name: &str) -> bool {
        name == "play"
    }

    /// `play(pattern, instrument)` or `play(pattern, instrument, rate)`.
    /// When piped (`pat >> play(inst)`) the pattern arrives as `piped` and the
    /// written arguments shift left by one.
    pub fn play(&mut self, args: &[Expr], piped: Option<Value>) -> Result<Value, String> {
        let (pattern_value, rest) = match piped {
            Some(p) => (p, args),
            None => {
                let Some((first, rest)) = args.split_first() else {
                    return Err("play expects a pattern and an instrument".to_string());
                };
                (self.expr(first)?, rest)
            }
        };

        let Some((instrument_expr, tail)) = rest.split_first() else {
            return Err("play expects an instrument name".to_string());
        };

        let Expr::Var(Ident(instrument)) = instrument_expr else {
            return Err("play: the instrument must be a plain function name".to_string());
        };

        if !matches!(self.env.lookup(instrument), Some(Value::Function(_))) {
            return Err(format!("play: {instrument} is not a function"));
        }

        let rate = match tail.first() {
            None => 1.0,
            Some(e) => match self.expr(e)? {
                Value::Number(n) => n,
                _ => return Err("play: rate must be a compile-time number".to_string()),
            },
        };
        if !(rate > 0.0) || !rate.is_finite() {
            return Err(format!("play: rate must be positive and finite, got {rate}"));
        }
        if tail.len() > 1 {
            return Err(format!("play expects at most 3 arguments, got {}", tail.len() + 2));
        }

        let mut pattern = to_pattern(&pattern_value)?;
        if rate != 1.0 {
            pattern = Pattern::Fast(rate, Box::new(pattern));
        }

        self.bindings.push(Binding { instrument: instrument.clone(), pattern });

        // Contributes nothing to the output sum, like a bare number.
        Ok(Value::Number(0.0))
    }
}

/// Interpret a lowered value rhythmically. A list is a sequence; a nested list
/// subdivides its slot; a rest is silence; a bare number is a one-step pattern.
pub fn to_pattern(v: &Value) -> Result<Pattern, String> {
    match v {
        Value::List(items) => {
            let steps = items.iter().map(to_step).collect::<Result<Vec<_>, _>>()?;
            Ok(Pattern::Steps(steps))
        }
        Value::Number(n) => Ok(Pattern::Steps(vec![Step::Value(*n)])),
        Value::Rest => Ok(Pattern::Silence),
        Value::Signal(_) => {
            Err("a pattern cannot contain a signal (patterns are events, not audio)".to_string())
        }
        Value::Function(_) => Err("a pattern cannot contain a function".to_string()),
    }
}

fn to_step(v: &Value) -> Result<Step, String> {
    match v {
        Value::Number(n) => Ok(Step::Value(*n)),
        Value::Rest => Ok(Step::Rest),
        Value::List(_) => Ok(Step::Group(Box::new(to_pattern(v)?))),
        Value::Signal(_) => {
            Err("a pattern cannot contain a signal (patterns are events, not audio)".to_string())
        }
        Value::Function(_) => Err("a pattern cannot contain a function".to_string()),
    }
}
