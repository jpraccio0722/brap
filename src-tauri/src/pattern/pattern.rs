//! Patterns: pure descriptions of what happens when.
//!
//! Time is measured in *cycles* (one full repetition). A pattern has no cursor
//! and no state — you ask what falls inside a span and it tells you. That is
//! what makes swapping one mid-performance trivial.

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

/// One slot in a sequence.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Silence for the slot's duration.
    Rest,
    /// Sound, carrying the argument handed to the instrument.
    Value(f64),
    /// A whole pattern squeezed into this slot — one cycle of it, compressed.
    Group(Box<Pattern>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Silence,
    /// Steps evenly dividing one cycle.
    Steps(Vec<Step>),
    Stack(Vec<Pattern>),
    /// Compress into `1/rate` of the time, repeating to fill the cycle.
    Fast(f64, Box<Pattern>),
}


impl Pattern {
    /// Convenience for a flat sequence with no subdivision.
    pub fn steps(values: impl IntoIterator<Item = Option<f64>>) -> Pattern {
        Pattern::Steps(
            values
                .into_iter()
                .map(|v| match v {
                    Some(x) => Step::Value(x),
                    None => Step::Rest,
                })
                .collect(),
        )
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
                let step_dur = 1.0 / steps.len() as f64;
                let mut out = Vec::new();
                for cycle in span.begin.floor() as i64 ..= span.end.ceil() as i64 {
                    for (i, step) in steps.iter().enumerate() {
                        let slot = cycle as f64 + i as f64 * step_dur;
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

    /// What this pattern is *holding* at one instant, or `None` over a rest.
    ///
    /// This is how a lane is read: structure comes from the pattern being
    /// played, and every lane is asked what it has at that event's onset.
    /// Matching lanes to events by onset instead would need float equality, and
    /// would have nothing to say when the lanes are of different lengths — which
    /// is the case worth having, since that is where polymeter comes from.
    pub fn sample(&self, at: f64) -> Option<f64> {
        if !at.is_finite() { return None; }
        match self {
            Pattern::Silence => None,

            Pattern::Steps(steps) => {
                if steps.is_empty() { return None; }
                let pos = at.rem_euclid(1.0);
                let len = steps.len() as f64;
                // `min` guards the case where rem_euclid rounds up to exactly 1.
                let i = ((pos * len) as usize).min(steps.len() - 1);
                match &steps[i] {
                    Step::Rest => None,
                    Step::Value(v) => Some(*v),
                    // Slot-local time: one cycle of the inner pattern fills the
                    // slot, exactly as `query` treats a group.
                    Step::Group(inner) => inner.sample(pos * len - i as f64),
                }
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
        let p = Pattern::Steps(vec![
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
