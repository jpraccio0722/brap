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
use crate::pattern::pattern::{Pattern, Slot, Step, UNIT};
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
            let pattern = to_pattern(&value)?;
            whole_lengths(&pattern).map_err(|len| format!(
                "{name}: lane '{lane}' has a length of {len}. A lane is read by note, \
                 not by time — a `;` there is how many notes the value covers, so it \
                 has to be a whole number"))?;
            lanes.push(Lane { name: lane, pattern });
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
        let first = self.bindings.len();
        self.bindings.push(Binding {
            instrument: instrument.clone(),
            pattern,
            lanes,
            start,
            cycles,
            // Written once and never chosen between: only `wthen` and `maybe`
            // set these, and they set them on bindings this call already made.
            repeat: None,
            choice: None,
        });

        // The handle is what `.then` chains from. It contributes nothing to the
        // output sum, like a bare number.
        Ok(Value::Play {
            starts_at: start,
            ends_at: cycles.map(|c| start + c),
            first,
            last: self.bindings.len(),
            // Nothing precedes a `play`, so this is where the chain begins.
            chain_first: first,
            // A single play, so `.then_fill` has exactly one instrument to
            // inherit — the only place a template is ever set.
            template: Some(first),
        })
    }
}

/// Interpret a lowered value rhythmically. A list is a sequence; a nested list
/// subdivides its slot; a rest is silence; a bare number is a one-step pattern.
pub fn to_pattern(v: &Value) -> Result<Pattern, String> {
    match v {
        Value::List(items) => {
            // A `;` length here is time: the slot takes that share of the
            // cycle, as one sustained event. Absent, it is `UNIT` and the
            // sequence divides evenly, exactly as it did before `;` existed.
            let steps = items
                .iter()
                .map(|item| {
                    Ok(Slot::sized(to_step(&item.value)?, item.length.unwrap_or(UNIT)))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Pattern::Steps(steps))
        }
        // Layers, each dividing the cycle in its own right. Nothing is done to
        // reconcile their lengths — that they need not agree is the point.
        Value::Stack(layers) => Ok(Pattern::Stack(
            layers.iter().map(to_pattern).collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Number(n) => Ok(Pattern::seq([Step::Value(*n)])),
        Value::Rest => Ok(Pattern::Silence),
        Value::Trigger => Ok(Pattern::seq([Step::Value(1.0)])),
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

/// Refuse a fractional `;` anywhere in a pattern being used as a lane.
///
/// Lengths mean two different things in the two positions, and only one of them
/// admits a fraction. A pattern divides time, where `;1.5` is a dotted note; a
/// lane is indexed by which note is asking, where one and a half notes is not a
/// place. Rejecting it is better than rounding, because the rounding is silent
/// and the mistake is a misunderstanding worth naming.
///
/// Returns the offending length, so the caller can say which number it meant.
fn whole_lengths(p: &Pattern) -> Result<(), f64> {
    match p {
        Pattern::Silence => Ok(()),
        Pattern::Steps(slots) => slots.iter().try_for_each(|slot| {
            if slot.length.fract() != 0.0 {
                return Err(slot.length);
            }
            match &slot.step {
                Step::Group(inner) => whole_lengths(inner),
                _ => Ok(()),
            }
        }),
        Pattern::Stack(ps) => ps.iter().try_for_each(whole_lengths),
        Pattern::Fast(_, inner) => whole_lengths(inner),
    }
}

fn to_step(v: &Value) -> Result<Step, String> {
    match v {
        Value::Number(n) => Ok(Step::Value(*n)),
        Value::Rest => Ok(Step::Rest),
        // A trigger sounds but carries nothing; instruments that take an
        // argument see 1.
        Value::Trigger => Ok(Step::Value(1.0)),
        // Both fill the slot with a whole pattern: a list divides it in turn, a
        // stack layers over the whole of it. That is how a chord is written at
        // one step of a longer line.
        Value::List(_) | Value::Stack(_) => Ok(Step::Group(Box::new(to_pattern(v)?))),
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
        let mut first = usize::MAX;
        let mut last = 0usize;
        for (i, arg) in args.iter().enumerate() {
            let Value::Play { ends_at: end, first: f, last: l, .. } = arg else {
                return Err(format!(
                    "{PLAY_ALL}: argument {} is not a play — every argument must be a \
                     `play`, `play_once`, `playn`, or another `{PLAY_ALL}`", i + 1));
            };
            ends_at = later_end(ends_at, *end);
            // Contiguous, because the arguments were lowered left to right and
            // each one pushed its bindings as it went.
            first = first.min(*f);
            last = last.max(*l);
        }

        Ok(Value::Play {
            starts_at: self.play_start,
            ends_at,
            first: first.min(last),
            last,
            // A group is its own beginning: the plays it gathers were written
            // as its arguments, so there is no chain in front of it.
            chain_first: first.min(last),
            // Gathering several plays is exactly what makes "the instrument"
            // ambiguous, so a group is never a template for a fill — unless it
            // gathered only one, where nothing was made ambiguous at all.
            template: (last == first + 1).then_some(first),
        })
    }
}
