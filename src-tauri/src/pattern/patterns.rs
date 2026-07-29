use crate::pattern::pattern::{Event, Pattern, Span};

/// The lane name that never reaches the instrument: it scales the event's
/// length instead, so `legato: 0.2` is staccato and `1.5` overlaps the next
/// note. An instrument parameter of this name is unreachable, which `play`
/// reports at bind time rather than leaving to be discovered by ear.
pub const LEGATO: &str = "legato";

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
            // Before it opens, or past where it closed.
            let Some(span) = self.window(b.start, b.cycles, span) else {
                return Vec::new();
            };
            // Flattened once per binding rather than per event: a lane is the
            // same line of values whichever note is asking.
            let lanes: Vec<(&str, Vec<Option<f64>>)> = b.lanes.iter()
                .map(|l| (l.name.as_str(), l.pattern.values()))
                .collect();

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
        }).collect()
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
                    cycles: None,
                },
                Binding {
                    instrument: "hat".into(),
                    pattern: Pattern::steps([Some(1.0), Some(1.0)]),
                    lanes: Vec::new(),
                    start: 0.0,
                    cycles: None,
                },
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
            bindings: vec![Binding { instrument: "i".into(), pattern, lanes, start: 0.0, cycles: None }],
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
                cycles: None,
            }],
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
                cycles: None,
            }],
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
                cycles: None,
            }],
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
                pattern: Pattern::Steps(vec![
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
                cycles,
            }],
            origin,
        }
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
                    cycles: Some(1.0),
                },
                Binding {
                    instrument: "loop".into(),
                    pattern: Pattern::steps([Some(1.0)]),
                    lanes: Vec::new(),
                    start: 0.0,
                    cycles: None,
                },
            ],
            origin: 0.0,
        };

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
                cycles,
            }],
            origin: 0.0,
        }
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
                cycles: Some(1.0),
            }],
            origin: 3.2,
        };
        // Origin 3.2 rounds up to 4, plus two cycles of waiting.
        let onsets: Vec<f64> = pats
            .query(Span::new(3.2, 9.0))
            .iter()
            .map(|e| e.event.begin)
            .collect();
        assert_eq!(onsets, vec![6.0]);
    }
}
