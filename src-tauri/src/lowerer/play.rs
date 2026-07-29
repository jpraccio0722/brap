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
//!
//! `play_all` writes no binding of its own. It is the parallel counterpart to
//! `.then`: several plays that all open together, gathered into the one handle
//! that `.then` can chain from.

use crate::scree_graph::environment::Value;
use crate::lowerer::lower::Lowerer;
use crate::parser::parser::{Arg, Expr, Ident};
use crate::pattern::pattern::{Pattern, Step};
use crate::pattern::patterns::{Binding, Lane, RESERVED};

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
            match RESERVED.iter().find(|(reserved, _)| *reserved == lane) {
                // A reserved lane is never passed on, so the instrument must
                // not be expecting it under that name.
                Some((_, effect)) => {
                    if def.params.iter().any(|p| p.name.0 == lane) {
                        return Err(format!(
                            "{name}: '{lane}' {effect}, so {instrument} cannot take a \
                             parameter of that name"));
                    }
                }
                None => match def.params.iter().position(|p| p.name.0 == lane) {
                    None => return Err(format!(
                        "{name}: {instrument} has no parameter named '{lane}'")),
                    Some(0) => return Err(format!(
                        "{name}: '{lane}' is {instrument}'s first parameter, which the \
                         pattern itself fills")),
                    Some(_) => {}
                },
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
            // Only the pattern. A lane is read by position — the nth note takes
            // the nth value — so it advances with the notes whatever speed they
            // go at, and compressing it too would be compressing it twice.
            pattern = Pattern::Fast(rate, Box::new(pattern));
        }

        // Repeats are passes of the pattern, but the binding is bounded in
        // cycles — and `rate` has just packed `rate` passes into each one.
        let cycles = repeats.map(|n| n / rate);

        // Anything already sequenced by a `.then` above this call has moved the
        // start; a bare `play` writes at the origin.
        let start = self.play_start;
        self.bindings.push(
            Binding { instrument: instrument.clone(), pattern, lanes, start, cycles });

        // The handle is what `.then` chains from. It contributes nothing to the
        // output sum, like a bare number.
        Ok(Value::Play { ends_at: cycles.map(|c| start + c) })
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
        Value::Buffer(_) => Err(
            "a pattern cannot contain a buffer (a pattern says when, an instrument \
             says what)".to_string()),
        Value::Function(_) => Err("a pattern cannot contain a function".to_string()),
        Value::Play { .. } => Err("a pattern cannot contain a play".to_string()),
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
        Value::Buffer(_) => Err(
            "a pattern cannot contain a buffer (a pattern says when, an instrument \
             says what)".to_string()),
        Value::Function(_) => Err("a pattern cannot contain a function".to_string()),
        Value::Play { .. } => Err("a pattern cannot contain a play".to_string()),
    }
}

/// When two things running at once are both over. One that never stops makes
/// the pair never stop, so nothing may be scheduled after it.
pub fn later_end(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        _ => None,
    }
}

/// `play_all(a, b, ...)` — several plays as one section.
pub const PLAY_ALL: &str = "play_all";

impl Lowerer {
    pub fn is_play_all(name: &str) -> bool {
        name == PLAY_ALL
    }

    /// Gather plays that run together into a single handle.
    ///
    /// There is nothing to schedule here. Each argument was lowered before this
    /// was reached, and every one of them wrote its binding at the current
    /// `play_start` — they are already concurrent. What is missing is a name for
    /// the group, and that is what this returns: one `Play` that ends when the
    /// last of them does, so `.then` can follow the whole section rather than
    /// having to pick one of its parts to chain from.
    pub fn play_all(&mut self, args: &[Value]) -> Result<Value, String> {
        if args.is_empty() {
            return Err(format!(
                "{PLAY_ALL} expects at least one play: \
                 {PLAY_ALL}(playn(verse, lead, 4), play_once(hits, bass))"));
        }

        // The floor is where the section itself opened: a group is never over
        // before it began, whatever its parts turn out to be.
        let mut ends_at = Some(self.play_start);
        for (i, arg) in args.iter().enumerate() {
            let Value::Play { ends_at: end } = arg else {
                return Err(format!(
                    "{PLAY_ALL}: argument {} is not a play — every argument must be a \
                     `play`, `play_once`, `playn`, or another `{PLAY_ALL}`", i + 1));
            };
            ends_at = later_end(ends_at, *end);
        }

        Ok(Value::Play { ends_at })
    }
}

/// `.then(f)` — run `f`'s bindings once this one has finished.
pub const THEN: &str = "then";

impl Lowerer {
    pub fn is_then(name: &str) -> bool {
        name == THEN
    }

    /// Sequence one section after another.
    ///
    /// There is no runtime interpreter here — the program *is* the graph — so
    /// this cannot be a callback fired by the audio thread. What it does
    /// instead is the useful half: `f` is inlined now, at eval time, and every
    /// binding it writes is offset to start where the receiver stops. The
    /// scheduler needs no notion of "afterwards" at all; it just sees bindings
    /// that open later.
    ///
    /// `f` takes no parameters. Closures do not exist — a function is inlined
    /// and captures nothing — so it has to be a named `fn`.
    pub fn then(&mut self, args: &[Value]) -> Result<Value, String> {
        let (Some(receiver), Some(body), 2) = (args.first(), args.get(1), args.len()) else {
            return Err(format!(
                "{THEN} expects a function to run afterwards: \
                 playn(pat, inst, 4).{THEN}(next)"));
        };

        let Value::Play { ends_at } = receiver else {
            return Err(format!(
                "{THEN}: the left side must be a play — `playn(...).{THEN}(f)`"));
        };
        let Some(offset) = *ends_at else {
            return Err(format!(
                "{THEN}: plain `play` never finishes, so nothing could follow it. \
                 Use `play_once` or `playn`."));
        };

        let Value::Function(def) = body else {
            return Err(format!("{THEN}: expects a function, and not a call of one"));
        };
        if !def.params.is_empty() {
            return Err(format!(
                "{THEN}: the function must take no parameters, but it declares {}",
                def.params.len()));
        }

        // Everything `f` writes starts where the receiver stopped. Nested
        // `.then`s inside it are relative to *its* start, so the offsets add up
        // exactly as the nesting reads.
        let outer = self.play_start;
        let first_new = self.bindings.len();
        self.play_start = offset;
        let result = self.apply(THEN, def.clone(), Vec::new());
        self.play_start = outer;
        result?;

        // Where the whole chain now ends. A binding that never stops makes the
        // chain never stop, so a further `.then` is refused rather than being
        // scheduled at a time that will not arrive.
        let mut ends_at = Some(offset);
        for binding in &self.bindings[first_new..] {
            ends_at = later_end(ends_at, binding.cycles.map(|c| binding.start + c));
        }

        Ok(Value::Play { ends_at })
    }
}
