//! `play(pattern, instrument, rate = 1, <lane>: pattern, ...)` — binding a
//! pattern to an instrument.
//!
//! `play` is intercepted before argument evaluation because the instrument
//! must be named syntactically: `Binding` stores a name, and a `Value::Function`
//! has already lost it. Having the definition in hand is also what lets a lane
//! name be checked against the instrument's parameters here, at eval time,
//! rather than failing once per event on the scheduler thread.
//!
//! `play_once` and `playn` are the same binding with an end to it: they differ
//! only in the `cycles` the binding carries, which is where the whole of "stop
//! after n passes" lives.

use crate::scree_graph::environment::Value;
use crate::lowerer::lower::Lowerer;
use crate::parser::parser::{Arg, Expr, Ident};
use crate::pattern::pattern::{Pattern, Step};
use crate::pattern::patterns::{Binding, Lane, LEGATO};

/// The one-shot: `play`, stopping after a single pass of the pattern.
pub const PLAY_ONCE: &str = "play_once";
/// The counted one: `playn(pattern, instrument, times, rate = 1)`.
pub const PLAY_N: &str = "playn";

impl Lowerer {
    /// True when this call should be handled as a `play`.
    pub fn is_play(name: &str) -> bool {
        matches!(name, "play" | PLAY_ONCE | PLAY_N)
    }

    /// `play(pattern, instrument)` or `play(pattern, instrument, rate)`, plus
    /// any number of trailing named lanes: `play(bass, cut: [400, 2000])`.
    /// When piped (`pat >> play(inst)`) the pattern arrives as `piped` and the
    /// positional arguments shift left by one.
    ///
    /// `name` picks the variant: `play_once` plays one pass, and `playn` takes
    /// its count in the position `rate` would otherwise sit in.
    pub fn play(&mut self, name: &str, args: &[Arg], piped: Option<Value>)
        -> Result<Value, String>
    {
        // Same rule as an ordinary call: interleaving would make it a puzzle
        // which of `play(pat, cut: 400, kick)`'s arguments is the instrument.
        if let Some(first_named) = args.iter().position(|a| a.name.is_some()) {
            if args[first_named..].iter().any(|a| a.name.is_none()) {
                return Err(format!(
                    "{name}: positional arguments must come before named ones"));
            }
        }

        let (positional, named): (Vec<&Arg>, Vec<&Arg>) =
            args.iter().partition(|a| a.name.is_none());
        let positional: Vec<&Expr> = positional.into_iter().map(|a| &a.value).collect();

        let (pattern_value, rest) = match piped {
            Some(p) => (p, positional.as_slice()),
            None => {
                let Some((first, rest)) = positional.split_first() else {
                    return Err(format!("{name} expects a pattern and an instrument"));
                };
                (self.expr(first)?, rest)
            }
        };

        let Some((instrument_expr, tail)) = rest.split_first() else {
            return Err(format!("{name} expects an instrument name"));
        };

        let Expr::Var(Ident(instrument)) = instrument_expr else {
            return Err(format!("{name}: the instrument must be a plain function name"));
        };

        let Some(Value::Function(def)) = self.env.lookup(instrument) else {
            return Err(format!("{name}: {instrument} is not a function"));
        };

        // `playn` takes its count where the others take their rate, so it comes
        // off the front of the tail and everything after it lines up again.
        let (repeats, tail) = match name {
            PLAY_N => {
                let Some((count, tail)) = tail.split_first() else {
                    return Err(format!(
                        "{PLAY_N} expects a number of repeats: {PLAY_N}(pat, {instrument}, 4)"));
                };
                let Value::Number(n) = self.expr(count)? else {
                    return Err(format!("{PLAY_N}: repeats must be a compile-time number"));
                };
                if !(n >= 1.0) || !n.is_finite() {
                    return Err(format!("{PLAY_N}: repeats must be at least 1, got {n}"));
                }
                (Some(n), tail)
            }
            PLAY_ONCE => (Some(1.0), tail),
            _ => (None, tail),
        };

        let rate = match tail.first() {
            None => 1.0,
            Some(e) => match self.expr(e)? {
                Value::Number(n) => n,
                _ => return Err(format!("{name}: rate must be a compile-time number")),
            },
        };
        if !(rate > 0.0) || !rate.is_finite() {
            return Err(format!("{name}: rate must be positive and finite, got {rate}"));
        }
        if tail.len() > 1 {
            // The count `playn` took above is still one of the caller's
            // arguments, so both numbers have to account for it.
            let taken = if name == PLAY_N { 3 } else { 2 };
            return Err(format!(
                "{name} expects at most {} arguments, got {}", taken + 1, tail.len() + taken));
        }

        // The pattern fills the first parameter, so a lane cannot also name it.
        // Every other name must be one the instrument actually declares — the
        // arity check that was missing when a voice took exactly one argument.
        let mut lanes = Vec::with_capacity(named.len());
        for arg in named {
            let lane = arg.name.as_ref().expect("partitioned on name").0.clone();
            if lane != LEGATO {
                match def.params.iter().position(|p| p.name.0 == lane) {
                    None => return Err(format!(
                        "{name}: {instrument} has no parameter named '{lane}'")),
                    Some(0) => return Err(format!(
                        "{name}: '{lane}' is {instrument}'s first parameter, which the \
                         pattern itself fills")),
                    Some(_) => {}
                }
            } else if def.params.iter().any(|p| p.name.0 == LEGATO) {
                return Err(format!(
                    "{name}: '{LEGATO}' sets the note's length, so {instrument} cannot \
                     take a parameter of that name"));
            }
            if lanes.iter().any(|l: &Lane| l.name == lane) {
                return Err(format!("{name}: lane '{lane}' given twice"));
            }

            let value = self.expr(&arg.value)?;
            lanes.push(Lane { name: lane, pattern: to_pattern(&value)? });
        }

        // Every parameter the pattern and lanes leave unfilled has to have a
        // default, or the instrument cannot be called at all.
        for param in def.params.iter().skip(1) {
            let filled = lanes.iter().any(|l| l.name == param.name.0);
            if !filled && param.default.is_none() {
                return Err(format!(
                    "{name}: {instrument} needs '{}' — give it a default or pass \
                     `{}: ...` here", param.name.0, param.name.0));
            }
        }

        let mut pattern = to_pattern(&pattern_value)?;
        if rate != 1.0 {
            // Lanes are compressed with the pattern, so `rate` stays a property
            // of the whole binding: a lane written step-for-step against the
            // pattern keeps lining up with it at any rate.
            pattern = Pattern::Fast(rate, Box::new(pattern));
            for lane in &mut lanes {
                lane.pattern = Pattern::Fast(rate, Box::new(lane.pattern.clone()));
            }
        }

        // Repeats are passes of the pattern, but the binding is bounded in
        // cycles — and `rate` has just packed `rate` passes into each one.
        let cycles = repeats.map(|n| n / rate);

        self.bindings.push(
            Binding { instrument: instrument.clone(), pattern, lanes, cycles });

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
        Value::Trigger => Ok(Pattern::Steps(vec![Step::Value(1.0)])),
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
        // A trigger sounds but carries nothing; instruments that take an
        // argument see 1.
        Value::Trigger => Ok(Step::Value(1.0)),
        Value::List(_) => Ok(Step::Group(Box::new(to_pattern(v)?))),
        Value::Signal(_) => {
            Err("a pattern cannot contain a signal (patterns are events, not audio)".to_string())
        }
        Value::Function(_) => Err("a pattern cannot contain a function".to_string()),
    }
}
