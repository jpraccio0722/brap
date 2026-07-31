use crate::pattern::pattern::{Event, Pattern, Span};

/// Scales the event's length rather than reaching the instrument, so
/// `legato: 0.2` is staccato and `1.5` overlaps the next note. Applied here,
/// in `query`, because a length is a property of the event.
pub const LEGATO: &str = "legato";

/// Places the voice in the stereo field rather than reaching the instrument:
/// -1 is hard left, 0 centre, 1 hard right. Applied in `build_voice`, because
/// a position is a property of the voice — it rides through here as an
/// ordinary lane value and is taken off the end.
pub const PAN: &str = "pan";

/// The lane names that never reach the instrument, each with what it does
/// instead. A parameter of one of these names could never be filled, so `play`
/// refuses it at bind time rather than leaving it to be discovered by ear.
pub const RESERVED: [(&str, &str); 2] = [
    (LEGATO, "sets the note's length"),
    (PAN, "places the voice in the stereo field"),
];

/// One named parameter, as a sequence of values rather than a shape in time.
///
/// A lane is read by position: the nth note of the binding takes the nth value,
/// wrapping when it runs out. So the two lengths are free of each other — three
/// cutoffs against four notes is a real 3-against-4, rotating a step each cycle
/// and coming back into phase after three, and twenty cutoffs against two notes
/// walks all twenty over ten cycles. Reading a lane by *time* instead would
/// squeeze it into the one cycle it shares with the pattern, where the extra
/// values are duplicated or skipped and nothing ever moves.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub name: String,
    pub pattern: Pattern,
}

/// A pattern paired with the instrument that plays it.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub instrument: String,   // name of the user `fn`
    /// Structure, and the instrument's first parameter.
    pub pattern: Pattern,
    pub lanes: Vec<Lane>,
    /// Cycles to wait after the origin's downbeat before this binding starts.
    /// Zero for everything `play` writes directly; `.then` sets it so what
    /// follows begins where the previous one stopped.
    pub start: f64,
    /// How long this binding sounds for, in cycles, counted from the eval that
    /// published it. `None` is `play`: it loops for as long as it is playing.
    /// `play_once` and `playn` set it, and it is measured in cycles rather than
    /// repeats because `rate` has already been folded into the pattern.
    pub cycles: Option<f64>,
    /// How often the whole window comes back around, in cycles. `None` is
    /// every binding written before `wthen` existed: the window opens once.
    ///
    /// A repeating binding sounds during `[start, start + cycles)` and again
    /// every `repeat` cycles after that, forever. It is what makes a choice
    /// worth rerolling — without somewhere to come back to, a branch would be
    /// picked once and that would be the end of it.
    pub repeat: Option<f64>,
    /// Which arm of which choice this binding belongs to, if it belongs to
    /// one. The window is gated on the arm being the one drawn for that
    /// repetition, so of a choice's arms exactly one sounds each time around.
    pub choice: Option<ChoiceRef>,
}

/// A binding's membership of a choice: which choice, and which of its arms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChoiceRef {
    pub group: usize,
    pub arm: usize,
}

/// One `wthen`: the weights its arms were given, and the seed its draws come
/// from.
///
/// The draw is a *hash*, not a running RNG, and that is the whole design.
/// `query` is called once per scheduler pass over a lookahead window that
/// overlaps the last one, and a binding near a boundary is asked about more
/// than once — so the answer has to depend only on which repetition is being
/// asked about, never on how many times it has been asked. A hash of
/// `(seed, group, repetition)` gives a fresh draw each time around and the
/// same draw every time that repetition comes up, which is what keeps a note
/// from flickering in and out as the horizon creeps past it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceGroup {
    /// Non-negative, summing to something positive. Normalised at draw time
    /// rather than at build time, so weights need not be given as fractions.
    pub weights: Vec<f64>,
    /// Fixed per eval, so re-evaluating deals a new hand and `seed` pins one.
    pub seed: u64,
}

impl ChoiceGroup {
    /// Which arm sounds on repetition `n`.
    ///
    /// Returns `arm == weights.len()` for the "nothing" arm `maybe` adds, so a
    /// choice can also come up silent.
    pub fn arm_at(&self, group: usize, n: u64) -> usize {
        let total: f64 = self.weights.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
        if !(total > 0.0) {
            return usize::MAX; // no arm can match: the choice is silent
        }
        let r = unit_hash(self.seed, group as u64, n) * total;
        let mut acc = 0.0;
        for (i, w) in self.weights.iter().enumerate() {
            if !w.is_finite() || *w <= 0.0 {
                continue;
            }
            acc += w;
            if r < acc {
                return i;
            }
        }
        // Only reachable when the running sum falls a bit short of `total`
        // through rounding, and then the last positive arm is the right answer.
        self.weights.iter().rposition(|w| w.is_finite() && *w > 0.0).unwrap_or(usize::MAX)
    }
}

/// SplitMix64, the finalising mix. Cheap, and it decorrelates the low bits —
/// which matters here because two of the three inputs are small integers that
/// differ by one.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// A number in `[0, 1)` from three integers. 53 bits, the most an `f64` holds
/// without a gap.
fn unit_hash(seed: u64, group: u64, n: u64) -> f64 {
    let h = mix(seed ^ mix(group.wrapping_mul(0x9E3779B97F4A7C15) ^ mix(n)));
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Everything currently playing. An eval replaces this wholesale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patterns {
    pub bindings: Vec<Binding>,
    /// Cycle time when this set was published — what a bounded binding counts
    /// its cycles from, so a one-shot fires at the eval that wrote it rather
    /// than at wherever the free-running clock happens to be.
    ///
    /// Safe to hold as a bare number because the only thing that moves cycle
    /// time under it is `Clock::reset`, and both callers of that either
    /// republish immediately after (an eval from silence) or clear the
    /// bindings entirely (stop).
    pub origin: f64,
    /// The choices this set's bindings refer to by index. Empty for a program
    /// with no `wthen` in it, which is most of them.
    pub choices: Vec<ChoiceGroup>,
}

/// An event with its instrument attached — what the scheduler consumes.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundEvent {
    pub instrument: String,
    pub event: Event,
    /// Lane values sampled at this event's onset, ready to be passed by name.
    /// A lane resting here is absent, so the parameter falls back to its own
    /// default rather than erroring.
    pub args: Vec<(String, f64)>,
}

impl Patterns {
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn query(&self, span: Span) -> Vec<BoundEvent> {
        self.bindings.iter().flat_map(|b| {
            // Before it opens, past where it closed, or — for a binding under
            // a choice — in a repetition where a different arm was drawn.
            let windows = self.windows(b, span);
            if windows.is_empty() {
                return Vec::new();
            }
            // Flattened once per binding rather than per event: a lane is the
            // same line of values whichever note is asking.
            let lanes: Vec<(&str, Vec<Option<f64>>)> = b.lanes.iter()
                .map(|l| (l.name.as_str(), l.pattern.values()))
                .collect();

            windows.into_iter().flat_map(|span| {
            b.pattern.query(span).into_iter().map(|mut event| {
                // Which note this is, counted from the origin — the lane's
                // position, not a time to look up.
                let nth = b.pattern.onsets_before(event.begin);
                let mut args = Vec::with_capacity(lanes.len());
                for (name, values) in &lanes {
                    if values.is_empty() { continue }
                    let Some(v) = values[nth % values.len()] else { continue };
                    if *name == LEGATO {
                        // Applied here so `dur` and the voice's own lifetime
                        // stay the same number: the scheduler derives both from
                        // the event's span.
                        if v.is_finite() && v > 0.0 {
                            event.end = event.begin + (event.end - event.begin) * v;
                        }
                    } else {
                        args.push((name.to_string(), v));
                    }
                }
                BoundEvent { instrument: b.instrument.clone(), event, args }
            }).collect::<Vec<_>>()
            }).collect::<Vec<_>>()
        }).collect()
    }

    /// Every part of `span` a binding may sound in.
    ///
    /// One window for everything that opens once, which is `window` unchanged.
    /// A repeating binding gets one per repetition the span reaches, and a
    /// binding under a choice keeps only the repetitions its own arm was drawn
    /// for. Several, rather than one, because a lookahead window is free to
    /// straddle a repetition boundary — and at fast tempos it often does.
    fn windows(&self, b: &Binding, span: Span) -> Vec<Span> {
        let Some(period) = b.repeat else {
            // Nothing repeats, so a choice would be drawn once and stay drawn;
            // `wthen` always sets both, and this is the path everything else
            // takes.
            return self.window(b.start, b.cycles, span).into_iter().collect();
        };
        // A repetition is only meaningful against a window that closes: an
        // open-ended one already covers every later repetition of itself.
        let (Some(cycles), true) = (b.cycles, period.is_finite() && period > 0.0) else {
            return self.window(b.start, b.cycles, span).into_iter().collect();
        };

        let opens = self.origin.ceil() + b.start;
        // Which repetitions can overlap the span at all. The window is
        // `cycles` long, so one starting up to `cycles` before the span may
        // still reach into it.
        let first = ((span.begin - opens - cycles) / period).floor().max(0.0);
        let last = ((span.end - opens) / period).floor();
        if !first.is_finite() || !last.is_finite() || last < first {
            return Vec::new();
        }
        // Cheap insurance against a tiny period and a distant span asking for
        // millions of windows: past this many the repetitions are far shorter
        // than a note and nobody could hear them apart.
        const MAX_REPETITIONS: f64 = 4096.0;
        let last = last.min(first + MAX_REPETITIONS);

        let mut out = Vec::new();
        let mut n = first;
        while n <= last {
            let repetition = n as u64;
            let sounds = match b.choice {
                None => true,
                Some(ChoiceRef { group, arm }) => self
                    .choices
                    .get(group)
                    .is_some_and(|c| c.arm_at(group, repetition) == arm),
            };
            if sounds {
                let begin = span.begin.max(opens + n * period);
                let end = span.end.min(opens + n * period + cycles);
                if end > begin {
                    out.push(Span::new(begin, end));
                }
            }
            n += 1.0;
        }
        out
    }

    /// The part of `span` a binding bounded to `cycles` may still sound in, or
    /// `None` if none of it is.
    ///
    /// The window opens at the first whole cycle at or after the origin, so a
    /// one-shot dropped into a running performance lands on a downbeat and is
    /// heard from its first step rather than joining halfway through. Playing
    /// from silence puts the origin a lead-in *before* cycle 0, which rounds up
    /// to 0 — the one-shot starts at once.
    fn window(&self, start: f64, cycles: Option<f64>, span: Span) -> Option<Span> {
        // Plain `play` with nothing before it joins the performance already in
        // progress, so a re-eval mid-cycle does not gap until the next downbeat.
        if start == 0.0 && cycles.is_none() {
            return Some(span);
        }

        let opens = self.origin.ceil() + start;
        let begin = span.begin.max(opens);
        let end = match cycles {
            None => span.end,
            // Also catches NaN, which would otherwise open a window nothing
            // closes.
            Some(c) if c > 0.0 => span.end.min(opens + c),
            Some(_) => return None,
        };
        (end > begin).then(|| Span::new(begin, end))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::pattern::Step;

    #[test]
    fn bound_events_carry_their_instrument() {
        let pats = Patterns {
            bindings: vec![
                Binding {
                    instrument: "kick".into(),
                    pattern: Pattern::steps([Some(1.0), None]),
                    lanes: Vec::new(),
                    start: 0.0,
                    cycles: None, repeat: None, choice: None },
                Binding {
                    instrument: "hat".into(),
                    pattern: Pattern::steps([Some(1.0), Some(1.0)]),
                    lanes: Vec::new(),
                    start: 0.0,
                    cycles: None, repeat: None, choice: None },
            ],
            ..Default::default()
        };

        let evs = pats.query(Span::new(0.0, 1.0));
        let mut names: Vec<_> = evs.iter().map(|b| b.instrument.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["hat", "hat", "kick"]);
    }

    #[test]
    fn empty_pattern_set_queries_to_nothing() {
        let pats = Patterns::default();
        assert!(pats.is_empty());
        assert!(pats.query(Span::new(0.0, 100.0)).is_empty());
    }

    // ---- lanes ----

    fn bound(pattern: Pattern, lanes: Vec<Lane>) -> Vec<super::BoundEvent> {
        Patterns {
            bindings: vec![Binding { instrument: "i".into(), pattern, lanes, start: 0.0, cycles: None, repeat: None, choice: None }],
            ..Default::default()
        }
        .query(Span::new(0.0, 1.0))
    }

    fn lane(name: &str, steps: Vec<Option<f64>>) -> Lane {
        Lane { name: name.into(), pattern: Pattern::steps(steps) }
    }

    #[test]
    fn lanes_are_sampled_at_each_onset() {
        let evs = bound(
            Pattern::steps([Some(60.0), Some(62.0)]),
            vec![lane("cut", vec![Some(400.0), Some(2000.0)])],
        );

        assert_eq!(evs[0].args, vec![("cut".to_string(), 400.0)]);
        assert_eq!(evs[1].args, vec![("cut".to_string(), 2000.0)]);
    }

    /// A shorter lane repeats against a longer pattern, note for note — not
    /// stretched to cover it. Two values under four notes is heard twice.
    #[test]
    fn a_short_lane_repeats_under_a_long_pattern() {
        let evs = bound(
            Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]),
            vec![lane("cut", vec![Some(10.0), Some(20.0)])],
        );

        let cuts: Vec<f64> = evs.iter().map(|e| e.args[0].1).collect();
        assert_eq!(cuts, vec![10.0, 20.0, 10.0, 20.0]);
    }

    /// The case the whole positional reading exists for: a lane longer than the
    /// pattern is not squeezed into the cycle it shares with it. Two notes and
    /// six cutoffs take three cycles to come back around, and every value is
    /// heard on the way.
    #[test]
    fn a_long_lane_walks_across_cycles() {
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "i".into(),
                pattern: Pattern::steps([Some(1.0), Some(2.0)]),
                lanes: vec![lane("cut", (1..=6).map(|i| Some(i as f64 * 100.0)).collect())],
                start: 0.0,
                cycles: None, repeat: None, choice: None }],
            ..Default::default()
        };

        let cuts: Vec<f64> = (0..4)
            .flat_map(|c| pats.query(Span::new(c as f64, c as f64 + 1.0)))
            .map(|e| e.args[0].1)
            .collect();

        assert_eq!(cuts, vec![
            100.0, 200.0,   // cycle 0
            300.0, 400.0,   // cycle 1
            500.0, 600.0,   // cycle 2
            100.0, 200.0,   // cycle 3 — back in phase
        ]);
    }

    /// Lengths with a common factor rotate rather than repeat: three against
    /// four is a step further along each cycle, in phase again after three.
    #[test]
    fn uneven_lengths_rotate_against_each_other() {
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "i".into(),
                pattern: Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]),
                lanes: vec![lane("cut", vec![Some(10.0), Some(20.0), Some(30.0)])],
                start: 0.0,
                cycles: None, repeat: None, choice: None }],
            ..Default::default()
        };

        let cycle = |c: i32| -> Vec<f64> {
            pats.query(Span::new(c as f64, c as f64 + 1.0))
                .iter().map(|e| e.args[0].1).collect()
        };

        assert_eq!(cycle(0), vec![10.0, 20.0, 30.0, 10.0]);
        assert_eq!(cycle(1), vec![20.0, 30.0, 10.0, 20.0]);
        assert_eq!(cycle(2), vec![30.0, 10.0, 20.0, 30.0]);
        assert_eq!(cycle(3), vec![10.0, 20.0, 30.0, 10.0]);
    }

    /// A lane counts notes, not time, so the speed the notes go at does not
    /// move it: the nth note takes the nth value at any rate.
    #[test]
    fn rate_does_not_shift_a_lane() {
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "i".into(),
                pattern: Pattern::fast(2.0, Pattern::steps([Some(1.0), Some(2.0)])),
                lanes: vec![lane("cut", vec![Some(10.0), Some(20.0), Some(30.0)])],
                start: 0.0,
                cycles: None, repeat: None, choice: None }],
            ..Default::default()
        };

        // Twice as fast is four notes a cycle, still taking the lane in order.
        let cuts: Vec<f64> = pats.query(Span::new(0.0, 1.0))
            .iter().map(|e| e.args[0].1).collect();
        assert_eq!(cuts, vec![10.0, 20.0, 30.0, 10.0]);
    }

    /// A rest in the *pattern* is not a note, so it takes no lane value with
    /// it: the lane advances by what sounds, not by what was written.
    #[test]
    fn a_rest_in_the_pattern_does_not_consume_a_lane_value() {
        let evs = bound(
            Pattern::steps([Some(1.0), None, Some(2.0), Some(3.0)]),
            vec![lane("cut", vec![Some(10.0), Some(20.0), Some(30.0)])],
        );

        let cuts: Vec<f64> = evs.iter().map(|e| e.args[0].1).collect();
        assert_eq!(cuts, vec![10.0, 20.0, 30.0]);
    }

    /// A nested list in a lane is more values, not a subdivision: lanes are
    /// read by position, so nesting only affects the order.
    #[test]
    fn a_nested_lane_flattens_into_the_line() {
        let evs = bound(
            Pattern::steps([Some(1.0), Some(2.0), Some(3.0)]),
            vec![Lane {
                name: "cut".into(),
                pattern: Pattern::seq(vec![
                    Step::Value(10.0),
                    Step::Group(Box::new(Pattern::steps([Some(20.0), Some(30.0)]))),
                ]),
            }],
        );

        let cuts: Vec<f64> = evs.iter().map(|e| e.args[0].1).collect();
        assert_eq!(cuts, vec![10.0, 20.0, 30.0]);
    }

    /// A lane resting says nothing, so the parameter falls to its own default
    /// rather than being passed a value the lane never had.
    #[test]
    fn a_resting_lane_passes_nothing() {
        let evs = bound(
            Pattern::steps([Some(1.0), Some(2.0)]),
            vec![lane("cut", vec![Some(400.0), None])],
        );

        assert_eq!(evs[0].args.len(), 1);
        assert!(evs[1].args.is_empty(), "a rest in a lane must not pass a value");
    }

    /// Legato scales the event's length instead of being passed on: the
    /// scheduler derives both `dur` and the voice's lifetime from that span.
    #[test]
    fn legato_shortens_the_event_and_is_not_an_argument() {
        let evs = bound(
            Pattern::steps([Some(1.0), Some(2.0)]),
            vec![lane(LEGATO, vec![Some(0.5), Some(2.0)])],
        );

        assert!(evs.iter().all(|e| e.args.is_empty()), "legato is not passed to the instrument");
        assert!((evs[0].event.duration() - 0.25).abs() < 1e-9, "got {:?}", evs[0].event);
        assert!((evs[1].event.duration() - 1.0).abs() < 1e-9, "got {:?}", evs[1].event);
        // Onsets never move — only the end does.
        assert_eq!(evs[0].event.begin, 0.0);
        assert_eq!(evs[1].event.begin, 0.5);
    }

    /// A nonsense legato value leaves the note at its natural length rather
    /// than producing an event the sequencer would reject.
    #[test]
    fn a_bad_legato_value_is_ignored() {
        for bad in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            let evs = bound(
                Pattern::steps([Some(1.0)]),
                vec![Lane { name: LEGATO.into(), pattern: Pattern::steps([Some(bad)]) }],
            );
            assert!((evs[0].event.duration() - 1.0).abs() < 1e-9, "legato {bad} changed the note");
        }
    }

    // ---- bounded bindings ----

    fn bounded(cycles: Option<f64>, origin: f64, span: Span) -> Vec<f64> {
        Patterns {
            bindings: vec![Binding {
                instrument: "i".into(),
                pattern: Pattern::steps([Some(1.0), Some(2.0)]),
                lanes: Vec::new(),
                start: 0.0,
                cycles, repeat: None, choice: None }],
            origin, choices: Vec::new() }
        .query(span)
        .iter()
        .map(|e| e.event.begin)
        .collect()
    }

    /// The default: `play` keeps going for as long as it is playing.
    #[test]
    fn an_unbounded_binding_never_stops() {
        assert_eq!(bounded(None, 0.0, Span::new(8.0, 9.0)), vec![8.0, 8.5]);
    }

    /// `play_once` from silence: the reset puts the origin a lead-in before
    /// cycle 0, and the pattern plays exactly one cycle from there.
    #[test]
    fn one_cycle_plays_then_stops() {
        assert_eq!(bounded(Some(1.0), -0.05, Span::new(0.0, 1.0)), vec![0.0, 0.5]);
        assert!(bounded(Some(1.0), -0.05, Span::new(1.0, 4.0)).is_empty());
    }

    #[test]
    fn a_counted_binding_plays_that_many_cycles() {
        assert_eq!(
            bounded(Some(3.0), 0.0, Span::new(0.0, 4.0)),
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5],
        );
    }

    /// Dropped into a running performance, a one-shot waits for the downbeat
    /// rather than starting halfway through its own pattern.
    #[test]
    fn a_one_shot_started_mid_cycle_begins_at_the_next_one() {
        assert_eq!(bounded(Some(1.0), 3.2, Span::new(3.2, 6.0)), vec![4.0, 4.5]);
    }

    /// The scheduler queries in small spans, so the window has to be assembled
    /// from pieces exactly as one big query would have it.
    #[test]
    fn a_window_queried_in_pieces_matches_one_query() {
        let whole = bounded(Some(2.0), 0.0, Span::new(0.0, 5.0));

        let mut pieced = Vec::new();
        let mut t = 0.0;
        while t < 5.0 {
            pieced.extend(bounded(Some(2.0), 0.0, Span::new(t, t + 0.13)));
            t += 0.13;
        }

        assert_eq!(whole, pieced);
    }

    /// A window that never opens is silence, not a binding that plays forever.
    #[test]
    fn a_degenerate_count_sounds_nothing() {
        for bad in [0.0, -1.0, f64::NAN] {
            assert!(
                bounded(Some(bad), 0.0, Span::new(0.0, 8.0)).is_empty(),
                "a count of {bad} should have sounded nothing",
            );
        }
    }

    /// A bounded binding that has run out must not silence the ones still
    /// looping alongside it.
    #[test]
    fn a_finished_binding_leaves_the_others_playing() {
        let pats = Patterns {
            bindings: vec![
                Binding {
                    instrument: "once".into(),
                    pattern: Pattern::steps([Some(1.0)]),
                    lanes: Vec::new(),
                    start: 0.0,
                    cycles: Some(1.0), repeat: None, choice: None },
                Binding {
                    instrument: "loop".into(),
                    pattern: Pattern::steps([Some(1.0)]),
                    lanes: Vec::new(),
                    start: 0.0,
                    cycles: None, repeat: None, choice: None },
            ],
            origin: 0.0, choices: Vec::new() };

        let names: Vec<_> = pats
            .query(Span::new(5.0, 6.0))
            .iter()
            .map(|e| e.instrument.clone())
            .collect();
        assert_eq!(names, vec!["loop"]);
    }

    #[test]
    fn several_lanes_all_reach_the_event() {
        let evs = bound(
            Pattern::steps([Some(1.0)]),
            vec![lane("cut", vec![Some(400.0)]), lane("amp", vec![Some(0.8)])],
        );

        assert_eq!(evs[0].args, vec![("cut".into(), 400.0), ("amp".into(), 0.8)]);
    }

    // ---- sequenced bindings ----

    fn started(start: f64, cycles: Option<f64>, span: Span) -> Vec<f64> {
        Patterns {
            bindings: vec![Binding {
                instrument: "i".into(),
                pattern: Pattern::steps([Some(1.0), Some(2.0)]),
                lanes: Vec::new(),
                start,
                cycles, repeat: None, choice: None }],
            origin: 0.0, choices: Vec::new() }
        .query(span)
        .iter()
        .map(|e| e.event.begin)
        .collect()
    }

    /// What `.then` writes: silent until its offset, then playing normally.
    #[test]
    fn a_started_binding_waits_for_its_offset() {
        assert!(started(4.0, None, Span::new(0.0, 4.0)).is_empty());
        assert_eq!(started(4.0, None, Span::new(0.0, 5.0)), vec![4.0, 4.5]);
        assert_eq!(started(4.0, None, Span::new(9.0, 10.0)), vec![9.0, 9.5]);
    }

    /// An offset one-shot sounds for its own count, measured from its start.
    #[test]
    fn a_started_one_shot_runs_from_where_it_opens() {
        assert!(started(2.0, Some(1.0), Span::new(0.0, 2.0)).is_empty());
        assert_eq!(started(2.0, Some(1.0), Span::new(0.0, 8.0)), vec![2.0, 2.5]);
    }

    /// The scheduler queries in small spans; a sequenced binding has to be
    /// assembled from pieces exactly as one big query would have it.
    #[test]
    fn a_sequenced_window_queried_in_pieces_matches_one_query() {
        let whole = started(2.0, Some(2.0), Span::new(0.0, 8.0));

        let mut pieced = Vec::new();
        let mut t = 0.0;
        while t < 8.0 {
            pieced.extend(started(2.0, Some(2.0), Span::new(t, t + 0.13)));
            t += 0.13;
        }

        assert_eq!(whole, pieced);
    }

    /// Sequencing counts from the origin's downbeat, like a one-shot does, so
    /// a chain dropped into a running performance stays on the grid.
    #[test]
    fn an_offset_is_measured_from_the_downbeat() {
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "i".into(),
                pattern: Pattern::steps([Some(1.0)]),
                lanes: Vec::new(),
                start: 2.0,
                cycles: Some(1.0), repeat: None, choice: None }],
            origin: 3.2, choices: Vec::new() };
        // Origin 3.2 rounds up to 4, plus two cycles of waiting.
        let onsets: Vec<f64> = pats
            .query(Span::new(3.2, 9.0))
            .iter()
            .map(|e| e.event.begin)
            .collect();
        assert_eq!(onsets, vec![6.0]);
    }

    // ---- repeating windows and choice ----

    /// One arm of a two-armed choice, sounding once per cycle.
    fn arm(instrument: &str, group: usize, index: usize, period: f64) -> Binding {
        Binding {
            instrument: instrument.into(),
            pattern: Pattern::steps([Some(1.0)]),
            lanes: Vec::new(),
            start: 0.0,
            cycles: Some(1.0),
            repeat: Some(period),
            choice: Some(ChoiceRef { group, arm: index }),
        }
    }

    fn two_armed(seed: u64, weights: Vec<f64>) -> Patterns {
        Patterns {
            bindings: vec![arm("a", 0, 0, 1.0), arm("b", 0, 1, 1.0)],
            origin: 0.0,
            choices: vec![ChoiceGroup { weights, seed }],
        }
    }

    /// Which instrument sounded in each of the first `n` cycles.
    fn drawn(pats: &Patterns, n: i64) -> Vec<String> {
        (0..n)
            .map(|i| {
                let evs = pats.query(Span::new(i as f64, i as f64 + 1.0));
                assert_eq!(evs.len(), 1, "exactly one arm sounds in cycle {i}");
                evs[0].instrument.clone()
            })
            .collect()
    }

    /// The property the whole design rests on: of a choice's arms, exactly one
    /// sounds each time round — never both, never neither.
    #[test]
    fn exactly_one_arm_sounds_per_repetition() {
        for seed in [0u64, 1, 0x5EED, u64::MAX] {
            let pats = two_armed(seed, vec![1.0, 1.0]);
            // `drawn` asserts the count; reaching the end is the test.
            assert_eq!(drawn(&pats, 32).len(), 32);
        }
    }

    /// A choice really does come back around: over enough repetitions both
    /// arms are drawn, which a window that opened once could not manage.
    #[test]
    fn a_choice_rerolls_rather_than_settling() {
        let pats = two_armed(0x5EED, vec![1.0, 1.0]);
        let seen = drawn(&pats, 64);
        assert!(seen.iter().any(|i| i == "a"), "arm a never came up");
        assert!(seen.iter().any(|i| i == "b"), "arm b never came up");
    }

    /// Why the draw is a hash and not a running RNG.
    ///
    /// The scheduler queries an overlapping lookahead window every pass, so the
    /// same cycle is asked about several times. A stateful generator would
    /// answer differently each time and the note would flicker; this asks the
    /// same span three ways and insists all three agree.
    #[test]
    fn the_same_repetition_draws_the_same_arm_however_it_is_queried() {
        let pats = two_armed(0x9E3779B9, vec![1.0, 1.0]);

        // One sweep, cycle by cycle.
        let once = drawn(&pats, 24);

        // The whole span in a single query.
        let mut whole: Vec<(f64, String)> = pats
            .query(Span::new(0.0, 24.0))
            .into_iter()
            .map(|e| (e.event.begin, e.instrument))
            .collect();
        whole.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let whole: Vec<String> = whole.into_iter().map(|(_, i)| i).collect();
        assert_eq!(once, whole, "one big query disagreed with cycle-by-cycle");

        // Overlapping windows that straddle every boundary, as the lookahead
        // does at speed.
        for cycle in 0..23 {
            let straddle = pats.query(Span::new(cycle as f64 + 0.5, cycle as f64 + 1.5));
            assert_eq!(straddle.len(), 1, "cycle {cycle} boundary");
            assert_eq!(
                straddle[0].instrument, once[cycle + 1],
                "a window straddling cycle {cycle} drew a different arm"
            );
        }
    }

    /// Weights are relative and need not be fractions; over enough draws the
    /// split tracks them. Loose bounds — this is testing that the weighting is
    /// wired up at all, not the quality of the hash.
    #[test]
    fn weights_bias_the_draw() {
        let pats = two_armed(0xC0FFEE, vec![9.0, 1.0]);
        let seen = drawn(&pats, 1000);
        let a = seen.iter().filter(|i| *i == "a").count();
        assert!((820..=980).contains(&a), "expected ~900 of 1000, got {a}");
    }

    /// A weight of zero is never drawn, and the arm that keeps it is simply
    /// never heard.
    #[test]
    fn a_zero_weight_is_never_drawn() {
        let pats = two_armed(42, vec![1.0, 0.0]);
        assert!(drawn(&pats, 200).iter().all(|i| i == "a"));
    }

    /// `maybe`'s silent arm owns no bindings, so the choice can come up empty —
    /// and does, at roughly its weight.
    #[test]
    fn a_choice_may_draw_an_arm_that_owns_nothing() {
        let pats = Patterns {
            bindings: vec![arm("a", 0, 0, 1.0)],
            origin: 0.0,
            // Arm 1 is silence: nothing refers to it.
            choices: vec![ChoiceGroup { weights: vec![0.25, 0.75], seed: 7 }],
        };
        let sounded = (0..400)
            .filter(|i| !pats.query(Span::new(*i as f64, *i as f64 + 1.0)).is_empty())
            .count();
        assert!((60..=140).contains(&sounded), "expected ~100 of 400, got {sounded}");
    }

    /// A repeating window with no choice on it simply comes back every period,
    /// which is what the arm gating is layered on top of.
    #[test]
    fn a_repeating_window_reopens_every_period() {
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "a".into(),
                pattern: Pattern::steps([Some(1.0)]),
                lanes: Vec::new(),
                start: 0.0,
                // Sounds for one cycle in every four.
                cycles: Some(1.0),
                repeat: Some(4.0),
                choice: None,
            }],
            origin: 0.0,
            choices: Vec::new(),
        };
        let onsets: Vec<f64> = pats
            .query(Span::new(0.0, 12.0))
            .into_iter()
            .map(|e| e.event.begin)
            .collect();
        assert_eq!(onsets, vec![0.0, 4.0, 8.0]);
    }

    /// A binding that repeats still respects where it opens.
    #[test]
    fn a_repeating_window_does_not_open_before_its_start() {
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "a".into(),
                pattern: Pattern::steps([Some(1.0)]),
                lanes: Vec::new(),
                start: 3.0,
                cycles: Some(1.0),
                repeat: Some(2.0),
                choice: None,
            }],
            origin: 0.0,
            choices: Vec::new(),
        };
        let onsets: Vec<f64> = pats
            .query(Span::new(0.0, 8.0))
            .into_iter()
            .map(|e| e.event.begin)
            .collect();
        assert_eq!(onsets, vec![3.0, 5.0, 7.0]);
    }
}
