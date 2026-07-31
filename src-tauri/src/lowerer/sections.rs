//! Arrangement: the family of combinators that chain from a `play`.
//!
//! `.then` is the root of it, and the rest are variations on the one move it
//! makes — inline a section now, at eval time, with `play_start` moved to
//! wherever it should begin. Nothing here is a callback, and the audio thread
//! learns nothing new: it still sees bindings that happen to open later.
//!
//! Three of them break that rule, and it is worth knowing which.
//!
//! `.take` and `.stop` reach *backwards*, shortening bindings that are already
//! on the timeline. They are the only things in the language that edit a
//! binding after it was written, which is what lets a bounded pattern decide
//! when an unbounded one gives up.
//!
//! `wthen` (with `rthen` and `maybe`) is the one that genuinely needs the
//! scheduler's help. Everything else settles at eval time; a choice that
//! rerolls cannot, because there is nothing at eval time to reroll *against*.
//! So it writes every arm's bindings, marks them as belonging to one choice,
//! and lets `Patterns::windows` decide which arm sounds each time around. See
//! [`ChoiceGroup`] for why that decision is a hash and not an RNG.

use std::rc::Rc;

use crate::lowerer::lower::Lowerer;
use crate::lowerer::play::{later_end, to_pattern};
use crate::pattern::patterns::{Binding, ChoiceGroup, ChoiceRef};
use crate::scree_graph::environment::{FunctionDef, Value};

pub const THEN: &str = "then";
pub const THEN_AFTER: &str = "then_after";
pub const THEN_N: &str = "then_n";
pub const THEN_EACH: &str = "then_each";
pub const THEN_FILL: &str = "then_fill";
pub const OVERLAP: &str = "overlap";
pub const WITH: &str = "with";
pub const AT: &str = "at";
pub const SEQ: &str = "seq";
pub const QUANTIZE: &str = "quantize";
pub const TAKE: &str = "take";
pub const STOP: &str = "stop";
pub const WTHEN: &str = "wthen";
pub const RTHEN: &str = "rthen";
pub const SHUFFLE_THEN: &str = "shuffle_then";
pub const MAYBE: &str = "maybe";

/// Every name this module handles, in one place so `lang` and `call` agree.
pub const SECTION_BUILTINS: &[&str] = &[
    THEN, THEN_AFTER, THEN_N, THEN_EACH, THEN_FILL, OVERLAP, WITH, AT, SEQ,
    QUANTIZE, TAKE, STOP, WTHEN, RTHEN, SHUFFLE_THEN, MAYBE,
];

/// What a handle covers: where it sits, and which bindings are its own.
struct Section {
    starts_at: f64,
    ends_at: Option<f64>,
    first: usize,
    last: usize,
}

impl Section {
    /// The handle a combinator answers with.
    ///
    /// `template` is set only when the section turned out to be a single
    /// binding, because that is the only case where "the instrument this
    /// section is playing" is a question with one answer — see `then_fill`.
    fn value(&self) -> Value {
        Value::Play {
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            first: self.first,
            last: self.last,
            template: (self.last == self.first + 1).then_some(self.first),
        }
    }
}

impl Lowerer {
    pub fn is_section(name: &str) -> bool {
        SECTION_BUILTINS.contains(&name)
    }

    /// Dispatch, once `call` has evaluated the arguments.
    pub fn section_builtin(&mut self, name: &str, args: &[Value]) -> Result<Value, String> {
        match name {
            THEN => self.then(args),
            THEN_AFTER => self.then_after(args),
            THEN_N => self.then_n(args),
            THEN_EACH => self.then_each(args),
            THEN_FILL => self.then_fill(args),
            OVERLAP => self.overlap(args),
            WITH => self.with(args),
            AT => self.at(args),
            SEQ => self.seq(args),
            QUANTIZE => self.quantize(args),
            TAKE => self.take(args),
            STOP => self.stop(args),
            WTHEN => self.wthen(name, args),
            RTHEN => self.wthen(name, args),
            SHUFFLE_THEN => self.shuffle_then(args),
            MAYBE => self.maybe(args),
            _ => Err(format!("{name} is not a section combinator")),
        }
    }

    // ---- the shared moves ----

    /// Inline `def` with `play_start` set to `offset`, and report what it wrote.
    ///
    /// This is `.then`'s whole mechanism, factored out: every combinator here
    /// that runs a section is this call with a different `offset`, and the
    /// differences between them are arithmetic on that one number.
    ///
    /// Nested combinators inside `def` are relative to *its* start, so offsets
    /// add up exactly as the nesting reads.
    fn inline(
        &mut self,
        who: &str,
        def: Rc<FunctionDef>,
        args: Vec<Value>,
        offset: f64,
    ) -> Result<Section, String> {
        if !offset.is_finite() {
            return Err(format!("{who}: cannot start a section at cycle {offset}"));
        }
        let outer = self.play_start;
        let first = self.bindings.len();
        self.play_start = offset;
        let result = self.inline_body(who, def, args);
        self.play_start = outer;
        result?;

        // The floor is the offset itself: a section is never over before it
        // began, however little it turned out to write. A binding that never
        // stops makes the whole section never stop, so a further `.then` is
        // refused rather than scheduled at a time that will not arrive.
        let mut ends_at = Some(offset);
        for binding in &self.bindings[first..] {
            ends_at = later_end(ends_at, binding.cycles.map(|c| binding.start + c));
        }
        Ok(Section { starts_at: offset, ends_at, first, last: self.bindings.len() })
    }

    fn inline_body(
        &mut self,
        who: &str,
        def: Rc<FunctionDef>,
        args: Vec<Value>,
    ) -> Result<(), String> {
        let wanted = args.len();
        if def.params.len() != wanted {
            return Err(match wanted {
                0 => format!(
                    "{who}: the section must take no parameters, but it declares {}",
                    def.params.len()),
                _ => format!(
                    "{who}: the section must take {wanted} parameter(s), but it declares {}",
                    def.params.len()),
            });
        }
        self.apply(who, def, args).map(|_| ())
    }

    /// The receiver of a chained combinator, unpacked.
    fn receiver(&self, who: &str, args: &[Value]) -> Result<Section, String> {
        let Some(Value::Play { starts_at, ends_at, first, last, .. }) = args.first() else {
            return Err(format!(
                "{who}: the left side must be a play — `playn(...).{who}(...)`"));
        };
        Ok(Section {
            starts_at: *starts_at,
            ends_at: *ends_at,
            first: *first,
            last: *last,
        })
    }

    /// Where a chain has reached. Refuses the endless case with the reason.
    fn cursor(&self, who: &str, s: &Section) -> Result<f64, String> {
        s.ends_at.ok_or_else(|| format!(
            "{who}: this section never finishes, so nothing could follow it. Bound it \
             with `play_once`, `playn`, `.take(n)` or `.stop()` first."))
    }

    fn function<'a>(&self, who: &str, what: &str, v: Option<&'a Value>)
        -> Result<Rc<FunctionDef>, String>
    {
        match v {
            Some(Value::Function(def)) => Ok(def.clone()),
            _ => Err(format!(
                "{who} expects a function: {what} must be a `fn` written by name, \
                 not a call of one")),
        }
    }

    fn constant(&self, who: &str, what: &str, v: Option<&Value>) -> Result<f64, String> {
        match v {
            Some(Value::Number(n)) if n.is_finite() => Ok(*n),
            _ => Err(format!("{who}: {what} must be a compile-time number")),
        }
    }

    /// Shorten every binding of a section so none of it sounds past `cut`.
    ///
    /// The one place a binding is edited after it was written. An unbounded
    /// one is given an end; a bounded one that already stops sooner is left
    /// alone, because a cut is a ceiling and not a length.
    fn cut_at(&mut self, s: &Section, cut: f64) {
        for b in &mut self.bindings[s.first..s.last] {
            let ends = b.cycles.map(|c| b.start + c);
            if ends.is_none_or(|e| e > cut) {
                b.cycles = Some((cut - b.start).max(0.0));
            }
        }
    }

    // ---- placing a section in time ----

    /// `.then(f)` — run `f`'s bindings once this section has finished.
    pub fn then(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(THEN, args)?;
        if args.len() != 2 {
            return Err(format!(
                "{THEN} expects a section to run afterwards: \
                 playn(pat, inst, 4).{THEN}(next)"));
        }
        let def = self.function(THEN, "the section", args.get(1))?;
        let at = self.cursor(THEN, &s)?;
        Ok(self.inline(THEN, def, Vec::new(), at)?.value())
    }

    /// `.then_after(n, f)` — `n` cycles of silence, then `f`.
    pub fn then_after(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(THEN_AFTER, args)?;
        if args.len() != 3 {
            return Err(format!(
                "{THEN_AFTER} expects a gap and a section: \
                 playn(pat, inst, 4).{THEN_AFTER}(2, next)"));
        }
        let gap = self.constant(THEN_AFTER, "the gap", args.get(1))?;
        if gap < 0.0 {
            return Err(format!(
                "{THEN_AFTER}: a gap cannot be negative — use `.{OVERLAP}` to start early"));
        }
        let def = self.function(THEN_AFTER, "the section", args.get(2))?;
        let at = self.cursor(THEN_AFTER, &s)? + gap;
        Ok(self.inline(THEN_AFTER, def, Vec::new(), at)?.value())
    }

    /// `.overlap(n, f)` — start `f` `n` cycles *before* this section ends.
    ///
    /// The whole of a crossfade, and the reason it is a combinator rather than
    /// a negative gap: the two sections really do sound together for `n`
    /// cycles, and the chain carries on from whichever of them ends later.
    pub fn overlap(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(OVERLAP, args)?;
        if args.len() != 3 {
            return Err(format!(
                "{OVERLAP} expects an overlap and a section: \
                 playn(pat, inst, 4).{OVERLAP}(1, next)"));
        }
        let by = self.constant(OVERLAP, "the overlap", args.get(1))?;
        if by < 0.0 {
            return Err(format!(
                "{OVERLAP}: an overlap cannot be negative — use `.{THEN_AFTER}` for a gap"));
        }
        let def = self.function(OVERLAP, "the section", args.get(2))?;
        let end = self.cursor(OVERLAP, &s)?;
        // Never before the section began: overlapping by more than its whole
        // length would put the newcomer in front of it.
        let at = (end - by).max(s.starts_at);
        let next = self.inline(OVERLAP, def, Vec::new(), at)?;
        Ok(Section {
            starts_at: s.starts_at,
            ends_at: later_end(Some(end), next.ends_at),
            first: s.first,
            last: next.last,
        }
        .value())
    }

    /// `.with(f)` — run `f` alongside this section, from where it began.
    ///
    /// `play_all` gathers plays that are already concurrent; this *makes* one
    /// concurrent with a section that has already been placed, which is what
    /// lets `A.with(drums).then(B)` read in the order it happens.
    pub fn with(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(WITH, args)?;
        if args.len() != 2 {
            return Err(format!(
                "{WITH} expects a section to run alongside: \
                 playn(pat, inst, 4).{WITH}(drums)"));
        }
        let def = self.function(WITH, "the section", args.get(1))?;
        let alongside = self.inline(WITH, def, Vec::new(), s.starts_at)?;
        Ok(Section {
            starts_at: s.starts_at,
            ends_at: later_end(s.ends_at, alongside.ends_at),
            first: s.first,
            last: alongside.last,
        }
        .value())
    }

    /// `at(n, f)` — place a section at cycle `n`, counted from the origin.
    ///
    /// The escape hatch from chaining: an arrangement you already know the
    /// shape of is often clearer written down than derived one `.then` at a
    /// time.
    pub fn at(&mut self, args: &[Value]) -> Result<Value, String> {
        if args.len() != 2 {
            return Err(format!("{AT} expects a cycle and a section: {AT}(8, chorus)"));
        }
        let when = self.constant(AT, "the cycle", args.first())?;
        if when < 0.0 {
            return Err(format!("{AT}: a cycle cannot be negative, got {when}"));
        }
        let def = self.function(AT, "the section", args.get(1))?;
        Ok(self.inline(AT, def, Vec::new(), when)?.value())
    }

    /// `seq(a, b, c)` — sections one after another, without the nesting.
    pub fn seq(&mut self, args: &[Value]) -> Result<Value, String> {
        if args.is_empty() {
            return Err(format!("{SEQ} expects at least one section: {SEQ}(intro, verse)"));
        }
        let start = self.play_start;
        let first = self.bindings.len();
        let mut at = start;
        for (i, arg) in args.iter().enumerate() {
            let def = self.function(SEQ, &format!("section {}", i + 1), Some(arg))?;
            let s = self.inline(SEQ, def, Vec::new(), at)?;
            at = s.ends_at.ok_or_else(|| format!(
                "{SEQ}: section {} never finishes, so nothing could follow it", i + 1))?;
        }
        Ok(Section {
            starts_at: start,
            ends_at: Some(at),
            first,
            last: self.bindings.len(),
        }
        .value())
    }

    // ---- repeating a section ----

    /// `.then_n(f, n)` — run `f` `n` times, back to back.
    ///
    /// Inlined afresh each time round rather than written once and repeated,
    /// which is what makes a `rand` inside `f` a different number on each pass
    /// — the same rule a voice already follows.
    pub fn then_n(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(THEN_N, args)?;
        if args.len() != 3 {
            return Err(format!(
                "{THEN_N} expects a section and a count: \
                 playn(pat, inst, 4).{THEN_N}(verse, 3)"));
        }
        let def = self.function(THEN_N, "the section", args.get(1))?;
        let times = self.constant(THEN_N, "the count", args.get(2))?;
        if times.fract() != 0.0 || times < 1.0 {
            return Err(format!(
                "{THEN_N}: the count must be a whole number of at least 1, got {times}"));
        }
        if times > MAX_REPEATS {
            return Err(format!(
                "{THEN_N}: {times} repeats is more than {MAX_REPEATS} — a typo? Each one is \
                 inlined, so the count is a size and not just a duration."));
        }

        let start = self.cursor(THEN_N, &s)?;
        let first = self.bindings.len();
        let mut at = start;
        for i in 0..times as usize {
            let s = self.inline(THEN_N, def.clone(), Vec::new(), at)?;
            at = s.ends_at.ok_or_else(|| format!(
                "{THEN_N}: pass {} never finishes, so the next could not follow it", i + 1))?;
        }
        Ok(Section { starts_at: start, ends_at: Some(at), first, last: self.bindings.len() }
            .value())
    }

    /// `.then_each(list, f)` — `f(element)` per element, in sequence.
    ///
    /// Arrangement by list, which is the point: every list function in the
    /// language already builds the shape of a piece, and this is what spends
    /// one. `[1, 2, 4, 8].rev` is a ritardando if `f` reads its argument as a
    /// rate.
    pub fn then_each(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(THEN_EACH, args)?;
        if args.len() != 3 {
            return Err(format!(
                "{THEN_EACH} expects a list and a section: \
                 playn(pat, inst, 4).{THEN_EACH}([1, 2, 4], faster)"));
        }
        let Some(Value::List(items)) = args.get(1) else {
            return Err(format!("{THEN_EACH}: expects a list to walk"));
        };
        if items.is_empty() {
            return Err(format!("{THEN_EACH}: the list is empty, so there is nothing to play"));
        }
        let items = items.clone();
        let def = self.function(THEN_EACH, "the section", args.get(2))?;
        if def.params.len() != 1 {
            return Err(format!(
                "{THEN_EACH}: the section must take exactly one parameter — the element — \
                 but it declares {}", def.params.len()));
        }

        let start = self.cursor(THEN_EACH, &s)?;
        let first = self.bindings.len();
        let mut at = start;
        for (i, item) in items.iter().enumerate() {
            let s = self.inline(
                THEN_EACH, def.clone(), vec![item.value.clone()], at)?;
            at = s.ends_at.ok_or_else(|| format!(
                "{THEN_EACH}: element {} never finishes, so the next could not follow it",
                i + 1))?;
        }
        Ok(Section { starts_at: start, ends_at: Some(at), first, last: self.bindings.len() }
            .value())
    }

    /// `.then_fill(pattern, rate?)` — one pass of `pattern` on this section's
    /// own instrument.
    ///
    /// The difference from `.then` is that there is no `fn` and no second
    /// `play`: a fill is played *by* whoever just played, so the instrument
    /// and every lane come from the binding this chains off. That is also why
    /// it needs a receiver that is one play — a group of them has no single
    /// instrument to be a fill for.
    pub fn then_fill(&mut self, args: &[Value]) -> Result<Value, String> {
        let Some(Value::Play { ends_at, template, .. }) = args.first() else {
            return Err(format!(
                "{THEN_FILL}: the left side must be a play — `playn(...).{THEN_FILL}(pat)`"));
        };
        if args.len() < 2 || args.len() > 3 {
            return Err(format!(
                "{THEN_FILL} expects a pattern and an optional rate: \
                 playn(groove, drums, 4).{THEN_FILL}([1, 1, 1, 1])"));
        }
        let Some(template) = *template else {
            return Err(format!(
                "{THEN_FILL}: a fill is played by the instrument it follows, so the left side \
                 has to be a single `play` — this one covers several, and there is no one \
                 instrument to fill for. Chain the fill onto the play itself."));
        };
        let at = ends_at.ok_or_else(|| format!(
            "{THEN_FILL}: this section never finishes, so a fill could not follow it. Use \
             `play_once` or `playn`."))?;

        let rate = match args.get(2) {
            None => 1.0,
            Some(v) => {
                let r = self.constant(THEN_FILL, "the rate", Some(v))?;
                if !(r > 0.0) {
                    return Err(format!(
                        "{THEN_FILL}: rate must be positive and finite, got {r}"));
                }
                r
            }
        };

        let mut pattern = to_pattern(args.get(1).expect("checked above"))?;
        if rate != 1.0 {
            pattern = crate::pattern::pattern::Pattern::Fast(rate, Box::new(pattern));
        }

        // Everything but the pattern is inherited: the fill is the same voice
        // and the same lanes, playing something else for one pass.
        let source = &self.bindings[template];
        let fill = Binding {
            instrument: source.instrument.clone(),
            pattern,
            lanes: source.lanes.clone(),
            start: at,
            cycles: Some(1.0 / rate),
            repeat: None,
            choice: None,
        };
        let first = self.bindings.len();
        self.bindings.push(fill);
        Ok(Section {
            starts_at: at,
            ends_at: Some(at + 1.0 / rate),
            first,
            last: self.bindings.len(),
        }
        .value())
    }

    // ---- bounding a section ----

    /// `.quantize(n?)` — round where the chain has reached up to a multiple of
    /// `n` cycles, without touching what is already playing.
    ///
    /// The cure for a section whose length is not a whole number. `rate`
    /// divides into the count — `playn(pat, inst, 3, 2)` is 1.5 cycles — and
    /// without this every `.then` after such a section is permanently off the
    /// downbeat, with nothing in the language able to recover.
    pub fn quantize(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(QUANTIZE, args)?;
        if args.len() > 2 {
            return Err(format!("{QUANTIZE} expects at most a grid: `.{QUANTIZE}(4)`"));
        }
        let grid = match args.get(1) {
            None => 1.0,
            Some(v) => self.constant(QUANTIZE, "the grid", Some(v))?,
        };
        if !(grid > 0.0) {
            return Err(format!("{QUANTIZE}: the grid must be positive, got {grid}"));
        }
        let end = self.cursor(QUANTIZE, &s)?;
        Ok(Section {
            starts_at: s.starts_at,
            // `ceil` of an exact multiple is itself, so a section already on
            // the grid is left where it is rather than pushed a bar out.
            ends_at: Some((end / grid).ceil() * grid),
            first: s.first,
            last: s.last,
        }
        .value())
    }

    /// `.take(n)` — this section, cut to `n` cycles.
    ///
    /// What gives a plain `play` an end. Until now an unbounded binding could
    /// never be chained from at all; `playn` only fixes that for a single
    /// play, not for a `play_all` group or a whole nested section.
    pub fn take(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(TAKE, args)?;
        if args.len() != 2 {
            return Err(format!("{TAKE} expects a length in cycles: `.{TAKE}(8)`"));
        }
        let n = self.constant(TAKE, "the length", args.get(1))?;
        if !(n > 0.0) {
            return Err(format!("{TAKE}: the length must be positive, got {n}"));
        }
        let cut = s.starts_at + n;
        self.cut_at(&s, cut);
        Ok(Section { starts_at: s.starts_at, ends_at: Some(cut), first: s.first, last: s.last }
            .value())
    }

    /// `.stop()` — cut everything still open in this section at the moment its
    /// last *bounded* part finishes.
    ///
    /// One pattern as the trigger to stop the rest: put a `playn` next to a
    /// pair of endless `play`s and the counted one decides when all three give
    /// up. Which is why the cut is the latest bounded end rather than the
    /// section's own `ends_at` — that is `None` here by definition, since an
    /// endless binding is exactly what there is to stop.
    pub fn stop(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(STOP, args)?;
        if args.len() != 1 {
            return Err(format!("{STOP} takes no arguments: `play_all(...).{STOP}()`"));
        }
        let cut = self.bindings[s.first..s.last]
            .iter()
            .filter_map(|b| b.cycles.map(|c| b.start + c))
            .fold(None::<f64>, |acc, e| Some(acc.map_or(e, |a| a.max(e))));
        let Some(cut) = cut else {
            return Err(format!(
                "{STOP}: nothing in this section ever finishes, so there is no moment to stop \
                 at. `{STOP}` cuts the endless parts where the counted ones run out — give it \
                 at least one `play_once` or `playn`, or say how long with `.{TAKE}(n)`."));
        };
        self.cut_at(&s, cut);
        Ok(Section { starts_at: s.starts_at, ends_at: Some(cut), first: s.first, last: s.last }
            .value())
    }

    // ---- choosing between sections ----

    /// `.wthen([a, b], [0.7, 0.3])` and `.rthen([a, b, c])`.
    ///
    /// Every arm is written to the timeline, all of them starting where this
    /// section ends and all of them marked as arms of one choice. Which one
    /// actually sounds is settled per repetition by the scheduler, so the
    /// answer is different each time around — that is the whole point, and the
    /// reason this cannot be an eval-time draw like `choice` is.
    ///
    /// The block repeats forever, because a choice with nowhere to come back
    /// to would be drawn once and never again. `ends_at` is `None` for the
    /// same reason; bound it with `.take(n)` if something should follow.
    pub fn wthen(&mut self, who: &str, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(who, args)?;
        let uniform = who == RTHEN;
        let wanted = if uniform { 2 } else { 3 };
        if args.len() != wanted && !(uniform && args.len() == 2) {
            return Err(match uniform {
                true => format!(
                    "{RTHEN} expects a list of sections: `.{RTHEN}([verse, chorus, bridge])`"),
                false => format!(
                    "{WTHEN} expects sections and their weights: \
                     `.{WTHEN}([verse, chorus], [0.7, 0.3])`"),
            });
        }

        let Some(Value::List(arms)) = args.get(1) else {
            return Err(format!("{who}: the sections must be given as a list"));
        };
        if arms.is_empty() {
            return Err(format!("{who}: needs at least one section to choose between"));
        }
        let arms = arms.clone();

        let weights = match uniform {
            true => vec![1.0; arms.len()],
            false => {
                let Some(Value::List(ws)) = args.get(2) else {
                    return Err(format!("{WTHEN}: the weights must be given as a list"));
                };
                if ws.len() != arms.len() {
                    return Err(format!(
                        "{WTHEN}: {} sections but {} weights — every section needs one",
                        arms.len(), ws.len()));
                }
                ws.iter()
                    .map(|i| match i.value {
                        Value::Number(n) if n.is_finite() && n >= 0.0 => Ok(n),
                        Value::Number(n) => Err(format!(
                            "{WTHEN}: a weight cannot be negative, got {n}")),
                        _ => Err(format!("{WTHEN}: weights must be compile-time numbers")),
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        if weights.iter().sum::<f64>() <= 0.0 {
            return Err(format!(
                "{who}: every weight is zero, so no section could ever be chosen"));
        }

        let at = self.cursor(who, &s)?;
        let group = self.choices.len();
        // Reserved before the arms are lowered so a nested choice inside one
        // of them takes a later index rather than this one.
        let seed = self.choice_seed();
        self.choices.push(ChoiceGroup { weights: weights.clone(), seed });

        let first = self.bindings.len();
        let mut period: f64 = 0.0;
        let mut spans = Vec::with_capacity(arms.len());
        for (i, arm) in arms.iter().enumerate() {
            let def = self.function(who, &format!("section {}", i + 1), Some(&arm.value))?;
            let s = self.inline(who, def, Vec::new(), at)?;
            let end = s.ends_at.ok_or_else(|| format!(
                "{who}: section {} never finishes. Every arm of a choice has to have a \
                 length, because the choice is made again each time round and they all \
                 have to come back to the same place.", i + 1))?;
            period = period.max(end - at);
            spans.push((s.first, s.last, i));
        }
        if !(period > 0.0) {
            return Err(format!("{who}: every section is empty, so there is nothing to choose"));
        }

        // Marked after the fact rather than during, so a nested `wthen` inside
        // an arm keeps its own group and only picks up this one as well.
        for (arm_first, arm_last, arm) in spans {
            for b in &mut self.bindings[arm_first..arm_last] {
                b.repeat = Some(period);
                if b.choice.is_none() {
                    b.choice = Some(ChoiceRef { group, arm });
                }
            }
        }

        Ok(Section { starts_at: at, ends_at: None, first, last: self.bindings.len() }.value())
    }

    /// `.maybe(p, f)` — `f`, with probability `p`, each time round.
    ///
    /// A `wthen` whose second arm is silence. It repeats and rerolls for the
    /// same reason `wthen` does: a coin flipped once is just an `if`.
    pub fn maybe(&mut self, args: &[Value]) -> Result<Value, String> {
        let s = self.receiver(MAYBE, args)?;
        if args.len() != 3 {
            return Err(format!(
                "{MAYBE} expects a chance and a section: `.{MAYBE}(0.25, fill)`"));
        }
        let p = self.constant(MAYBE, "the chance", args.get(1))?;
        if !(0.0..=1.0).contains(&p) {
            return Err(format!("{MAYBE}: the chance must be between 0 and 1, got {p}"));
        }
        let def = self.function(MAYBE, "the section", args.get(2))?;

        let at = self.cursor(MAYBE, &s)?;
        let group = self.choices.len();
        // Arm 1 is the silent one: it owns no bindings, so drawing it simply
        // means nothing sounds that time round.
        let seed = self.choice_seed();
        self.choices.push(ChoiceGroup { weights: vec![p, 1.0 - p], seed });

        let first = self.bindings.len();
        let played = self.inline(MAYBE, def, Vec::new(), at)?;
        let end = played.ends_at.ok_or_else(|| format!(
            "{MAYBE}: the section never finishes, so there would be nothing to come back \
             from and no second chance to take"))?;
        let period = end - at;
        if !(period > 0.0) {
            return Err(format!("{MAYBE}: the section is empty, so there is nothing to chance"));
        }
        for b in &mut self.bindings[played.first..played.last] {
            b.repeat = Some(period);
            if b.choice.is_none() {
                b.choice = Some(ChoiceRef { group, arm: 0 });
            }
        }
        Ok(Section { starts_at: at, ends_at: None, first, last: self.bindings.len() }.value())
    }

    /// `.shuffle_then([a, b, c])` — all of them, once each, in an order drawn
    /// now.
    ///
    /// The counterpart to `wthen` rather than a variant of it: a weighted
    /// choice may pass a section over for a long time, and this one cannot.
    /// It settles at eval time like `scramble`, so it has a length, and a
    /// `.then` may follow it.
    pub fn shuffle_then(&mut self, args: &[Value]) -> Result<Value, String> {
        use rand::seq::SliceRandom;

        let s = self.receiver(SHUFFLE_THEN, args)?;
        if args.len() != 2 {
            return Err(format!(
                "{SHUFFLE_THEN} expects a list of sections: \
                 `.{SHUFFLE_THEN}([verse, chorus, bridge])`"));
        }
        let Some(Value::List(items)) = args.get(1) else {
            return Err(format!("{SHUFFLE_THEN}: the sections must be given as a list"));
        };
        if items.is_empty() {
            return Err(format!("{SHUFFLE_THEN}: needs at least one section"));
        }

        let mut defs = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            defs.push(self.function(
                SHUFFLE_THEN, &format!("section {}", i + 1), Some(&item.value))?);
        }
        defs.shuffle(&mut self.rng);

        let start = self.cursor(SHUFFLE_THEN, &s)?;
        let first = self.bindings.len();
        let mut at = start;
        for (i, def) in defs.into_iter().enumerate() {
            let s = self.inline(SHUFFLE_THEN, def, Vec::new(), at)?;
            at = s.ends_at.ok_or_else(|| format!(
                "{SHUFFLE_THEN}: section {} never finishes, so the next could not follow it",
                i + 1))?;
        }
        Ok(Section { starts_at: start, ends_at: Some(at), first, last: self.bindings.len() }
            .value())
    }

    /// A seed for one choice, drawn from the eval's own RNG — so re-evaluating
    /// deals a new hand, and `seed` pins the whole arrangement along with
    /// everything else it pins.
    fn choice_seed(&mut self) -> u64 {
        use rand::RngExt;
        self.rng.random::<u64>()
    }
}

/// The most passes `then_n` will inline. Each one is real bindings rather than
/// a loop counter, so an absurd count is a memory problem and not just a long
/// piece of music.
const MAX_REPEATS: f64 = 1024.0;
