//! Patterns: pure descriptions of what happens when.
//!
//! Time is measured in *cycles* (one full repetition). A pattern has no cursor
//! and no state — you ask what falls inside a span and it tells you. That is
//! what makes swapping one mid-performance trivial.

/// Slack for comparing an onset against a time that should equal it. Onsets are
/// built by arithmetic on step durations, so the same instant reached two ways
/// can differ in the last bits; anything this small is nowhere near the gap
/// between two steps of a pattern anyone can hear.
const ONSET_EPSILON: f64 = 1e-9;

/// A half-open span of cycle-time: `[begin, end)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span { pub begin: f64, pub end: f64 }

impl Span {
    pub fn new(begin: f64, end: f64) -> Self {
        Span { begin, end }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub begin: f64,
    pub end: f64,
    pub value: f64,   // the argument handed to the instrument
}

impl Event {
    pub fn duration(&self) -> f64 {
        self.end - self.begin
    }
}

/// What happens in a slot.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Silence for the slot's duration.
    Rest,
    /// Sound, carrying the argument handed to the instrument.
    Value(f64),
    /// A whole pattern squeezed into this slot — one cycle of it, compressed.
    Group(Box<Pattern>),
}

/// The length every slot has unless a `;` gave it another one. A sequence of
/// these divides its cycle evenly, which is what patterns did before lengths
/// existed and what the great majority of them still want.
pub const UNIT: f64 = 1.0;

/// One slot in a sequence: what happens, and how much of the sequence it takes.
///
/// Lengths are *relative*, not absolute — the sequence always fills exactly one
/// cycle and the slots divide it in proportion. So `[a;2, b, c]` is a half and
/// two quarters, and the same shape written `[a;4, b;2, c;2]` is the same
/// rhythm. That is what makes an absent length mean "one": a sequence with no
/// lengths at all is every slot at [`UNIT`], which divides evenly.
///
/// A length is one slot, not several. `[c4;3, e4]` holds c4 for three quarters
/// of the cycle as a single sustained note — it does not strike it three times.
/// That distinction is the whole reason lengths live here rather than being
/// desugared into repeated steps on the way in.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub step: Step,
    pub length: f64,
}

impl Slot {
    /// A slot of the default length, which is what everything that predates
    /// `;` builds.
    pub fn new(step: Step) -> Slot {
        Slot { step, length: UNIT }
    }

    /// A slot with an explicit length. Nonsense lengths fall back to [`UNIT`]
    /// rather than poisoning the division: a single bad number would otherwise
    /// take the whole sequence silent, which is a hard thing to hear the cause
    /// of. `play` rejects them at bind time, where there is somewhere to say so.
    pub fn sized(step: Step, length: f64) -> Slot {
        let length = if length.is_finite() && length > 0.0 { length } else { UNIT };
        Slot { step, length }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Silence,
    /// Slots dividing one cycle in proportion to their lengths.
    Steps(Vec<Slot>),
    Stack(Vec<Pattern>),
    /// Compress into `1/rate` of the time, repeating to fill the cycle.
    Fast(f64, Box<Pattern>),
}


impl Pattern {
    /// Convenience for a flat sequence with no subdivision.
    pub fn steps(values: impl IntoIterator<Item = Option<f64>>) -> Pattern {
        Pattern::seq(values.into_iter().map(|v| match v {
            Some(x) => Step::Value(x),
            None => Step::Rest,
        }))
    }

    /// A sequence of steps at the default length — an even division, which is
    /// every pattern that does not use `;`.
    pub fn seq(steps: impl IntoIterator<Item = Step>) -> Pattern {
        Pattern::Steps(steps.into_iter().map(Slot::new).collect())
    }

    pub fn fast(rate: f64, p: Pattern) -> Pattern {
        Pattern::Fast(rate, Box::new(p))
    }

    pub fn slow(rate: f64, p: Pattern) -> Pattern {
        Pattern::Fast(1.0 / rate, Box::new(p))
    }

    /// Every event whose *onset* falls in `span`. Onsets match half-open, so
    /// adjacent spans never double-trigger or drop a note.
    pub fn query(&self, span: Span) -> Vec<Event> {
        match self {
            Pattern::Silence => Vec::new(),

            Pattern::Steps(steps) => {
                if steps.is_empty() || span.end <= span.begin { return Vec::new(); }
                // The lengths are relative, so the cycle is divided by their
                // total. `Slot::sized` has already refused anything that could
                // make this zero or non-finite.
                let total: f64 = steps.iter().map(|s| s.length).sum();
                if !(total > 0.0) || !total.is_finite() { return Vec::new(); }
                let mut out = Vec::new();
                for cycle in span.begin.floor() as i64 ..= span.end.ceil() as i64 {
                    // Walked rather than indexed: where a slot starts depends
                    // on the lengths of the ones before it, not on its position.
                    let mut offset = 0.0;
                    for Slot { step, length } in steps.iter() {
                        let step_dur = length / total;
                        let slot = cycle as f64 + offset / total;
                        offset += length;
                        match step {
                            Step::Rest => {}

                            Step::Value(value) => {
                                if slot >= span.begin && slot < span.end {
                                    out.push(Event {
                                        begin: slot,
                                        end: slot + step_dur,
                                        value: *value,
                                    });
                                }
                            }

                            // Exactly one cycle of the inner pattern fills the
                            // slot. Map the query span into slot-local time,
                            // clamped to that single cycle, then map back.
                            Step::Group(inner) => {
                                let local_begin =
                                    ((span.begin - slot) / step_dur).max(0.0);
                                let local_end =
                                    ((span.end - slot) / step_dur).min(1.0);
                                if local_end <= local_begin {
                                    continue;
                                }
                                for e in inner.query(Span::new(local_begin, local_end)) {
                                    out.push(Event {
                                        begin: slot + e.begin * step_dur,
                                        end: slot + e.end * step_dur,
                                        value: e.value,
                                    });
                                }
                            }
                        }
                    }
                }
                out
            }

            Pattern::Stack(ps) => ps.iter().flat_map(|p| p.query(span)).collect(),

            Pattern::Fast(rate, p) => {
                if *rate <= 0.0 || !rate.is_finite() { return Vec::new(); }
                let inner = Span { begin: span.begin * rate, end: span.end * rate };
                p.query(inner).into_iter().map(|e| Event {
                    begin: e.begin / rate,
                    end: e.end / rate,
                    value: e.value,
                }).collect()
            }
        }
    }

    /// How many onsets this pattern has strictly before `at`.
    ///
    /// This is what gives a lane its position: the nth note played takes the
    /// nth value of the lane, so a lane has to be told which note this is
    /// rather than what time it is. Counting rather than sampling is also what
    /// makes the two lengths independent — twenty cutoffs against two notes
    /// walks the whole list over ten cycles instead of reading the same two
    /// entries forever.
    ///
    /// Exact rather than `at * events_per_cycle`, because `Fast` may put a
    /// non-whole number of onsets in a cycle: each layer is counted in its own
    /// time, and only `Steps` — which is periodic in one cycle by construction —
    /// multiplies out.
    pub fn onsets_before(&self, at: f64) -> usize {
        if !at.is_finite() || at <= 0.0 {
            return 0;
        }
        match self {
            Pattern::Silence => 0,

            Pattern::Steps(steps) => {
                if steps.is_empty() {
                    return 0;
                }
                // One cycle's worth of onsets, then multiplied by whole cycles:
                // `Steps` repeats exactly, so cycle n holds the same set as
                // cycle 0 shifted by n.
                let in_cycle: Vec<f64> =
                    self.query(Span::new(0.0, 1.0)).into_iter().map(|e| e.begin).collect();
                if in_cycle.is_empty() {
                    return 0;
                }
                // Pulled back because `at` is itself an onset every time this is
                // called: the note being counted for must land outside its own
                // count, whichever side the arithmetic drifted to.
                let at = at - ONSET_EPSILON;
                let whole = at.floor();
                let within = at - whole;
                whole as usize * in_cycle.len()
                    + in_cycle.iter().filter(|s| **s < within).count()
            }

            Pattern::Stack(ps) => ps.iter().map(|p| p.onsets_before(at)).sum(),

            // `Fast` is a change of clock, so the count is the inner pattern's
            // at the inner time.
            Pattern::Fast(rate, p) => {
                if *rate <= 0.0 || !rate.is_finite() {
                    return 0;
                }
                p.onsets_before(at * rate)
            }
        }
    }

    /// Every value this pattern holds, in order, as one flat sequence.
    ///
    /// A lane is read positionally, so its subdivisions are not timing — a
    /// nested list is simply more values in the line. `None` is a rest, which
    /// leaves the instrument's own default in place for that note.
    ///
    /// A length here is how many *notes* the value covers rather than how much
    /// time, because that is the only thing position can measure: `[400;2,
    /// 2000]` is 400 for two notes and then 2000. Held and repeated are the
    /// same thing for a lane — there is no attack to re-strike — which is why
    /// this may flatten a length that a pattern would have had to sustain.
    pub fn values(&self) -> Vec<Option<f64>> {
        let mut out = Vec::new();
        self.collect_values(&mut out);
        out
    }

    fn collect_values(&self, out: &mut Vec<Option<f64>>) {
        match self {
            Pattern::Silence => {}
            Pattern::Steps(steps) => {
                for Slot { step, length } in steps {
                    // Lengths reaching a lane are whole numbers — `play` rejects
                    // fractions there, where half a note position means nothing
                    // — but round rather than trust it, since a pattern reused
                    // as a lane has been through no such check.
                    let held = (length.round() as usize).max(1);
                    match step {
                        Step::Rest => out.extend(std::iter::repeat(None).take(held)),
                        Step::Value(v) => {
                            out.extend(std::iter::repeat(Some(*v)).take(held))
                        }
                        // A group is already several values; stretching it would
                        // have to mean stretching each, which is not a thing
                        // anyone writes. Its own slots carry their own lengths.
                        Step::Group(inner) => inner.collect_values(out),
                    }
                }
            }
            Pattern::Stack(ps) => {
                for p in ps {
                    p.collect_values(out);
                }
            }
            Pattern::Fast(_, p) => p.collect_values(out),
        }
    }

    /// What this pattern is *holding* at one instant, or `None` over a rest.
    ///
    /// Not how lanes are read — see `onsets_before` for that. This answers the
    /// different question of what a pattern has at some time, which is what
    /// inspecting one at a moment needs.
    pub fn sample(&self, at: f64) -> Option<f64> {
        if !at.is_finite() { return None; }
        match self {
            Pattern::Silence => None,

            Pattern::Steps(steps) => {
                if steps.is_empty() { return None; }
                let total: f64 = steps.iter().map(|s| s.length).sum();
                if !(total > 0.0) || !total.is_finite() { return None; }
                // Where in the cycle, rescaled to the lengths' own units so it
                // can be walked against them.
                let pos = at.rem_euclid(1.0) * total;

                let mut offset = 0.0;
                for Slot { step, length } in steps {
                    // The last slot takes anything rem_euclid rounded up to the
                    // very end of the cycle, which no earlier slot will claim.
                    let last = offset + length >= total;
                    if pos < offset + length || last {
                        return match step {
                            Step::Rest => None,
                            Step::Value(v) => Some(*v),
                            // Slot-local time: one cycle of the inner pattern
                            // fills the slot, exactly as `query` treats a group.
                            Step::Group(inner) => inner.sample((pos - offset) / length),
                        };
                    }
                    offset += length;
                }
                None
            }

            // First lane that has something wins, so a stack reads as a layered
            // fallback rather than silently summing.
            Pattern::Stack(ps) => ps.iter().find_map(|p| p.sample(at)),

            Pattern::Fast(rate, p) => {
                if *rate <= 0.0 || !rate.is_finite() { return None; }
                p.sample(at * rate)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn onsets(events: &[Event]) -> Vec<f64> {
        events.iter().map(|e| e.begin).collect()
    }

    #[test]
    fn four_steps_fill_one_cycle() {
        let p = Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(onsets(&evs), vec![0.0, 0.25, 0.5, 0.75]);
        assert_eq!(evs[0].duration(), 0.25);
        assert_eq!(evs[3].value, 4.0);
    }

    #[test]
    fn rests_produce_nothing() {
        let p = Pattern::steps([Some(1.0), None, Some(3.0), None]);
        assert_eq!(onsets(&p.query(Span::new(0.0, 1.0))), vec![0.0, 0.5]);
    }

    #[test]
    fn pattern_repeats_every_cycle() {
        let p = Pattern::steps([Some(1.0), Some(2.0)]);
        let evs = p.query(Span::new(0.0, 3.0));
        assert_eq!(onsets(&evs), vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5]);
    }

    #[test]
    fn span_crossing_a_cycle_boundary() {
        let p = Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        assert_eq!(onsets(&p.query(Span::new(0.75, 1.25))), vec![0.75, 1.0]);
    }

    /// The property the scheduler depends on: consecutive spans partition the
    /// timeline exactly — every event once, none lost, none repeated.
    #[test]
    fn adjacent_spans_never_double_or_drop() {
        let p = Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let whole = onsets(&p.query(Span::new(0.0, 2.0)));

        let mut pieced = Vec::new();
        let mut t = 0.0;
        while t < 2.0 {
            pieced.extend(onsets(&p.query(Span::new(t, t + 0.17))));
            t += 0.17;
        }
        pieced.retain(|&x| x < 2.0);

        assert_eq!(whole, pieced, "querying in chunks must match one big query");
    }

    #[test]
    fn empty_and_degenerate_spans_are_safe() {
        let p = Pattern::steps([Some(1.0), Some(2.0)]);
        assert!(p.query(Span::new(1.0, 1.0)).is_empty());
        assert!(p.query(Span::new(2.0, 1.0)).is_empty());
        assert!(Pattern::Silence.query(Span::new(0.0, 10.0)).is_empty());
        assert!(Pattern::steps([]).query(Span::new(0.0, 1.0)).is_empty());
    }

    #[test]
    fn fast_doubles_density_and_shortens_events() {
        let p = Pattern::fast(2.0, Pattern::steps([Some(1.0), Some(2.0)]));
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(onsets(&evs), vec![0.0, 0.25, 0.5, 0.75]);
        assert!((evs[0].duration() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn slow_halves_density() {
        let p = Pattern::slow(2.0, Pattern::steps([Some(1.0), Some(2.0)]));
        assert_eq!(onsets(&p.query(Span::new(0.0, 2.0))), vec![0.0, 1.0]);
    }

    #[test]
    fn degenerate_rates_are_safe() {
        let p = Pattern::fast(0.0, Pattern::steps([Some(1.0)]));
        assert!(p.query(Span::new(0.0, 1.0)).is_empty());
    }

    // ---- sample ----

    #[test]
    fn sample_reads_the_step_holding_that_instant() {
        let p = Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        assert_eq!(p.sample(0.0), Some(1.0));
        assert_eq!(p.sample(0.24), Some(1.0));
        assert_eq!(p.sample(0.25), Some(2.0));
        assert_eq!(p.sample(0.99), Some(4.0));
    }

    /// A lane is sampled at absolute cycle time, so it has to repeat like the
    /// pattern it accompanies rather than run out after cycle 0.
    #[test]
    fn sample_repeats_every_cycle() {
        let p = Pattern::steps([Some(1.0), Some(2.0)]);
        assert_eq!(p.sample(7.75), Some(2.0));
        assert_eq!(p.sample(-0.25), Some(2.0));
        assert_eq!(p.sample(-0.75), Some(1.0));
    }

    /// The case the design is for: a lane shorter or longer than the pattern
    /// it decorates, drifting against it instead of erroring.
    #[test]
    fn a_lane_of_a_different_length_drifts() {
        let notes = Pattern::steps([Some(0.0), Some(0.0), Some(0.0), Some(0.0)]);
        let lane = Pattern::steps([Some(10.0), Some(20.0), Some(30.0)]);

        let onsets: Vec<f64> = notes.query(Span::new(0.0, 2.0)).iter().map(|e| e.begin).collect();
        let sampled: Vec<Option<f64>> = onsets.iter().map(|t| lane.sample(*t)).collect();

        assert_eq!(sampled, vec![
            Some(10.0), Some(10.0), Some(20.0), Some(30.0),   // cycle 0
            Some(10.0), Some(10.0), Some(20.0), Some(30.0),   // cycle 1
        ]);
    }

    #[test]
    fn sample_over_a_rest_is_none() {
        let p = Pattern::steps([Some(1.0), None]);
        assert_eq!(p.sample(0.5), None);
        assert_eq!(Pattern::Silence.sample(0.3), None);
        assert_eq!(Pattern::steps([]).sample(0.3), None);
    }

    #[test]
    fn sample_descends_into_a_group() {
        let p = Pattern::seq(vec![
            Step::Value(1.0),
            Step::Group(Box::new(Pattern::steps([Some(2.0), Some(3.0)]))),
        ]);
        assert_eq!(p.sample(0.0), Some(1.0));
        assert_eq!(p.sample(0.5), Some(2.0));
        assert_eq!(p.sample(0.76), Some(3.0));
    }

    /// Sampling has to agree with `query`: a lane written step-for-step against
    /// its pattern must line up with it at any rate.
    #[test]
    fn sample_and_query_agree_under_fast() {
        let p = Pattern::fast(2.0, Pattern::steps([Some(1.0), Some(2.0)]));
        for e in p.query(Span::new(0.0, 4.0)) {
            assert_eq!(p.sample(e.begin), Some(e.value), "at {}", e.begin);
        }
    }

    #[test]
    fn degenerate_sample_inputs_are_safe() {
        let p = Pattern::steps([Some(1.0)]);
        assert_eq!(p.sample(f64::NAN), None);
        assert_eq!(p.sample(f64::INFINITY), None);
        assert_eq!(Pattern::fast(0.0, p).sample(0.5), None);
    }

    #[test]
    fn stack_merges_layers() {
        let p = Pattern::Stack(vec![
            Pattern::steps([Some(1.0)]),
            Pattern::steps([Some(2.0), Some(3.0)]),
        ]);
        let mut got = onsets(&p.query(Span::new(0.0, 1.0)));
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(got, vec![0.0, 0.0, 0.5]);
    }
}

/// Slot lengths: the rhythm a `;` writes.
///
/// These are about division alone — that a length moves the onsets and the
/// durations of a sequence, and that its absence leaves both exactly where they
/// were. What a length is allowed to *be*, and what it means once a lane rather
/// than a cycle is reading it, is checked where those decisions are made.
#[cfg(test)]
mod length_tests {
    use super::*;

    fn onsets(events: &[Event]) -> Vec<f64> {
        events.iter().map(|e| e.begin).collect()
    }

    fn durations(events: &[Event]) -> Vec<f64> {
        events.iter().map(|e| e.duration()).collect()
    }

    fn sized(pairs: impl IntoIterator<Item = (f64, f64)>) -> Pattern {
        Pattern::Steps(
            pairs.into_iter().map(|(v, len)| Slot::sized(Step::Value(v), len)).collect(),
        )
    }

    /// The promise the whole feature rests on: no lengths at all is the even
    /// division patterns have always had.
    #[test]
    fn absent_lengths_divide_evenly() {
        let plain = Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let ones = sized([(1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0)]);
        assert_eq!(plain.query(Span::new(0.0, 1.0)), ones.query(Span::new(0.0, 1.0)));
    }

    /// A length takes its share of the cycle, and pushes everything after it
    /// along — the onsets move, which is what makes this rhythm and not sustain.
    #[test]
    fn a_length_takes_its_share_of_the_cycle() {
        let p = sized([(1.0, 2.0), (2.0, 1.0), (3.0, 1.0)]);
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(onsets(&evs), vec![0.0, 0.5, 0.75]);
        assert_eq!(durations(&evs), vec![0.5, 0.25, 0.25]);
    }

    /// Lengths are relative, so scaling every one of them changes nothing.
    #[test]
    fn only_the_ratio_matters() {
        let small = sized([(1.0, 2.0), (2.0, 1.0), (3.0, 1.0)]);
        let large = sized([(1.0, 8.0), (2.0, 4.0), (3.0, 4.0)]);
        assert_eq!(small.query(Span::new(0.0, 1.0)), large.query(Span::new(0.0, 1.0)));
    }

    /// The rhythm that started this: a quarter and two eighths, then two beats
    /// of rest, in one bar.
    #[test]
    fn a_quarter_and_two_eighths() {
        let p = Pattern::Steps(vec![
            Slot::sized(Step::Value(220.0), 2.0),
            Slot::sized(Step::Value(330.0), 1.0),
            Slot::sized(Step::Value(440.0), 1.0),
            Slot::sized(Step::Rest, 4.0),
        ]);
        let evs = p.query(Span::new(0.0, 1.0));
        // Eighths of a bar: the quarter at 0 for two of them, then one each.
        assert_eq!(onsets(&evs), vec![0.0, 0.25, 0.375]);
        assert_eq!(durations(&evs), vec![0.25, 0.125, 0.125]);
    }

    /// A length is one event, not several. This is the distinction that keeps
    /// lengths on the slot instead of being desugared into repeats on the way in
    /// — three strikes and one held note are different music.
    #[test]
    fn a_long_note_is_sustained_not_repeated() {
        let evs = sized([(60.0, 3.0), (64.0, 1.0)]).query(Span::new(0.0, 1.0));
        assert_eq!(evs.len(), 2, "a length must not multiply the note");
        assert_eq!(evs[0].value, 60.0);
        assert_eq!(evs[0].duration(), 0.75);
    }

    /// Lengths repeat with the cycle like everything else in a sequence.
    #[test]
    fn lengths_hold_across_cycles() {
        let p = sized([(1.0, 3.0), (2.0, 1.0)]);
        assert_eq!(onsets(&p.query(Span::new(0.0, 2.0))), vec![0.0, 0.75, 1.0, 1.75]);
    }

    /// A group takes its slot's share, and divides that share by its own
    /// lengths. Nesting and `;` are the same mechanism at two scales.
    #[test]
    fn a_group_may_be_given_a_length() {
        let inner = Pattern::seq([Step::Value(2.0), Step::Value(3.0)]);
        let p = Pattern::Steps(vec![
            Slot::sized(Step::Value(1.0), 3.0),
            Slot::sized(Step::Group(Box::new(inner)), 1.0),
        ]);
        let evs = p.query(Span::new(0.0, 1.0));
        // The group owns the last quarter, split in two.
        assert_eq!(onsets(&evs), vec![0.0, 0.75, 0.875]);
        assert_eq!(durations(&evs), vec![0.75, 0.125, 0.125]);
    }

    /// `sample` asks what is sounding at an instant, so it has to walk the
    /// lengths rather than divide by the slot count.
    #[test]
    fn sample_follows_the_lengths() {
        let p = sized([(1.0, 3.0), (2.0, 1.0)]);
        assert_eq!(p.sample(0.0), Some(1.0));
        assert_eq!(p.sample(0.5), Some(1.0), "still inside the long first slot");
        assert_eq!(p.sample(0.74), Some(1.0));
        assert_eq!(p.sample(0.8), Some(2.0));
        // And it repeats, so the same questions answer the same way next cycle.
        assert_eq!(p.sample(1.5), Some(1.0));
    }

    /// A lane reads by note, so a length there is how many notes the value
    /// covers — held and repeated being the same thing where there is no attack.
    #[test]
    fn values_repeat_a_length_for_a_lane() {
        assert_eq!(
            sized([(400.0, 2.0), (2000.0, 1.0)]).values(),
            vec![Some(400.0), Some(400.0), Some(2000.0)],
        );
    }

    /// Rests take a length too, which is how a bar ends in silence.
    #[test]
    fn a_rest_may_be_given_a_length() {
        let p = Pattern::Steps(vec![
            Slot::sized(Step::Value(1.0), 1.0),
            Slot::sized(Step::Rest, 3.0),
        ]);
        let evs = p.query(Span::new(0.0, 1.0));
        assert_eq!(onsets(&evs), vec![0.0]);
        assert_eq!(durations(&evs), vec![0.25], "the rest still takes its share");
    }

    /// A length that is not a number cannot be allowed to take the sequence
    /// silent — `Slot::sized` falls back rather than dividing by nothing.
    #[test]
    fn nonsense_lengths_fall_back_to_one() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Slot::sized(Step::Value(1.0), bad).length, UNIT, "length {bad}");
        }
    }

    /// The lane's own position count has to follow the lengths too, or a lane
    /// walks out of step with the pattern it is decorating.
    #[test]
    fn onsets_are_counted_at_their_new_places() {
        let p = sized([(1.0, 3.0), (2.0, 1.0)]);
        assert_eq!(p.onsets_before(0.5), 1, "only the first slot has begun");
        assert_eq!(p.onsets_before(0.75), 1);
        assert_eq!(p.onsets_before(1.0), 2, "both, by the end of the cycle");
    }
}
