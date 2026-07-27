use crate::pattern::pattern::{Event, Pattern, Span};

/// The lane name that never reaches the instrument: it scales the event's
/// length instead, so `legato: 0.2` is staccato and `1.5` overlaps the next
/// note. An instrument parameter of this name is unreachable, which `play`
/// reports at bind time rather than leaving to be discovered by ear.
pub const LEGATO: &str = "legato";

/// One named parameter, patterned. Sampled at each event's onset rather than
/// queried, so a lane may be any length — three cutoffs against four notes is a
/// deliberate 3-against-4, not a mismatch to pad out.
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
            let span = match b.cycles {
                None => span,
                Some(cycles) => match self.window(cycles, span) {
                    Some(s) => s,
                    // Bounded, and this span is either before it opens or past
                    // where it closed.
                    None => return Vec::new(),
                },
            };
            b.pattern.query(span).into_iter().map(|mut event| {
                let mut args = Vec::with_capacity(b.lanes.len());
                for lane in &b.lanes {
                    let Some(v) = lane.pattern.sample(event.begin) else { continue };
                    if lane.name == LEGATO {
                        // Applied here so `dur` and the voice's own lifetime
                        // stay the same number: the scheduler derives both from
                        // the event's span.
                        if v.is_finite() && v > 0.0 {
                            event.end = event.begin + (event.end - event.begin) * v;
                        }
                    } else {
                        args.push((lane.name.clone(), v));
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
    fn window(&self, cycles: f64, span: Span) -> Option<Span> {
        // Also catches NaN, which would otherwise open a window nothing closes.
        if !(cycles > 0.0) {
            return None;
        }
        let start = self.origin.ceil();
        let begin = span.begin.max(start);
        let end = span.end.min(start + cycles);
        (end > begin).then(|| Span::new(begin, end))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_events_carry_their_instrument() {
        let pats = Patterns {
            bindings: vec![
                Binding {
                    instrument: "kick".into(),
                    pattern: Pattern::steps([Some(1.0), None]),
                    lanes: Vec::new(),
                    cycles: None,
                },
                Binding {
                    instrument: "hat".into(),
                    pattern: Pattern::steps([Some(1.0), Some(1.0)]),
                    lanes: Vec::new(),
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
            bindings: vec![Binding { instrument: "i".into(), pattern, lanes, cycles: None }],
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

    /// A shorter lane repeats against a longer pattern, note for note.
    #[test]
    fn a_short_lane_repeats_under_a_long_pattern() {
        let evs = bound(
            Pattern::steps([Some(1.0), Some(2.0), Some(3.0), Some(4.0)]),
            vec![lane("cut", vec![Some(10.0), Some(20.0)])],
        );

        let cuts: Vec<f64> = evs.iter().map(|e| e.args[0].1).collect();
        assert_eq!(cuts, vec![10.0, 10.0, 20.0, 20.0]);
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
                    cycles: Some(1.0),
                },
                Binding {
                    instrument: "loop".into(),
                    pattern: Pattern::steps([Some(1.0)]),
                    lanes: Vec::new(),
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
}
