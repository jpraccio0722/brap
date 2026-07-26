//! The scheduler thread.
//!
//! It free-runs from app start and is never "triggered": an eval simply
//! replaces the patterns and instruments it reads on the next pass. That
//! inversion is what keeps the clock running across re-evals.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fundsp::prelude64::AudioUnit;
use fundsp::sequencer::{Fade, Sequencer};

use crate::pattern::pattern::Span;
use crate::pattern::patterns::Patterns;
use crate::scheduler::clock::Clock;
use crate::scheduler::voice::{Instruments, build_voice};

/// How far ahead of the audio clock we schedule.
const LOOKAHEAD_SECS: f64 = 0.2;
/// How often the thread wakes. Must be well under LOOKAHEAD_SECS.
const TICK: Duration = Duration::from_millis(25);
/// Per-voice fades, clamped against short notes before pushing.
const FADE_IN_SECS: f64 = 0.005;
const FADE_OUT_SECS: f64 = 0.02;

/// Shared handles the scheduler reads each pass. An eval swaps their contents.
#[derive(Clone)]
pub struct SchedulerState {
    pub patterns: Arc<Mutex<Patterns>>,
    pub instruments: Arc<Mutex<Instruments>>,
}

impl SchedulerState {
    pub fn new() -> Self {
        SchedulerState {
            patterns: Arc::new(Mutex::new(Patterns::default())),
            instruments: Arc::new(Mutex::new(Instruments::default())),
        }
    }
}

impl Default for SchedulerState {
    fn default() -> Self {
        SchedulerState::new()
    }
}

/// Spawn the scheduler. Call once, at startup.
pub fn start(seq: Sequencer, clock: Clock, state: SchedulerState) {
    std::thread::spawn(move || run(seq, clock, state));
}

fn run(mut seq: Sequencer, clock: Clock, state: SchedulerState) {
    // `None` until the first pass, so a long idle period before the first eval
    // does not look like a huge backlog to catch up on.
    let mut scheduled_through: Option<f64> = None;

    loop {
        std::thread::sleep(TICK);
        scheduled_through = schedule_pass(&mut seq, &clock, &state, scheduled_through);
    }
}

/// One pass of the loop: query the horizon, push whatever falls in it, and
/// return the new watermark. Split out from `run` so it can be tested without
/// threads or sleeping.
fn schedule_pass(
    seq: &mut Sequencer,
    clock: &Clock,
    state: &SchedulerState,
    scheduled_through: Option<f64>,
) -> Option<f64> {
    let now_cycles = clock.now_cycles();
    let horizon = clock.cycles_at(clock.now_secs() + LOOKAHEAD_SECS);

    // Never schedule into the past: if this thread stalled, skip the missed
    // events rather than firing a burst of late ones.
    let from = scheduled_through.unwrap_or(now_cycles).max(now_cycles);
    let next = Some(from.max(horizon));

    if horizon <= from {
        return next;
    }

    let events = match state.patterns.lock() {
        Ok(p) => {
            if p.is_empty() {
                return next;
            }
            p.query(Span::new(from, horizon))
        }
        Err(e) => {
            eprintln!("scheduler: patterns lock poisoned: {e}");
            return next;
        }
    };
    if events.is_empty() {
        return next;
    }

    // Clone the definitions so voice lowering happens outside the lock.
    let instruments = match state.instruments.lock() {
        Ok(i) => i.clone(),
        Err(e) => {
            eprintln!("scheduler: instruments lock poisoned: {e}");
            return next;
        }
    };

    for bound in events {
        let begin_secs = clock.secs_at(bound.event.begin);
        let dur_secs = clock.secs_at(bound.event.end) - begin_secs;

        match build_voice(&instruments, &bound.instrument, bound.event.value) {
            // A bad instrument must not kill the thread: log it and let the
            // rest of the pattern keep playing.
            Err(e) => eprintln!("scheduler: {}: {e}", bound.instrument),
            Ok(net) => push_voice(seq, begin_secs, dur_secs, net),
        }
    }

    next
}

/// Push one voice, defending against every case `Sequencer::push` asserts on.
fn push_voice(seq: &mut Sequencer, start_secs: f64, dur_secs: f64, net: fundsp::net::Net) {
    if !start_secs.is_finite() || !dur_secs.is_finite() || dur_secs <= 0.0 {
        eprintln!("scheduler: skipping voice with bad timing ({start_secs}, {dur_secs})");
        return;
    }
    if net.inputs() != 0 || net.outputs() != 2 {
        eprintln!(
            "scheduler: voice must be 0-in/2-out, got {}-in/{}-out",
            net.inputs(),
            net.outputs()
        );
        return;
    }

    // push asserts each fade is no longer than the event; short notes clamp.
    let half = dur_secs * 0.5;
    let fade_in = FADE_IN_SECS.min(half);
    let fade_out = FADE_OUT_SECS.min(half);

    seq.push_duration(start_secs, dur_secs, Fade::Smooth, fade_in, fade_out, Box::new(net));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brap_graph::realizer::realize;
    use crate::lowerer::lower::lower;
    use crate::parser::parser::parse;
    use crate::pattern::pattern::Pattern;
    use crate::pattern::patterns::Binding;
    use fundsp::sequencer::ReplayMode;

    fn voice_net() -> fundsp::net::Net {
        let items = parse("sin(220)\n".to_string()).unwrap();
        realize(&lower(&items).unwrap().graph).unwrap()
    }

    #[test]
    fn pushed_voice_renders_audio() {
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);
        push_voice(&mut seq, 0.0, 1.0, voice_net());

        let mut peak = 0.0f32;
        for _ in 0..22050 {
            let (l, r) = seq.get_stereo();
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak > 0.5, "voice should be audible, peak was {peak}");
    }

    /// A note shorter than the nominal fades must still push, not panic.
    #[test]
    fn very_short_notes_clamp_their_fades() {
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);
        push_voice(&mut seq, 0.0, 0.001, voice_net());

        for _ in 0..100 {
            let _ = seq.get_stereo();
        }
    }

    #[test]
    fn bad_timing_is_skipped_not_panicked() {
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        push_voice(&mut seq, 0.0, 0.0, voice_net());
        push_voice(&mut seq, f64::NAN, 1.0, voice_net());
        push_voice(&mut seq, 0.0, -1.0, voice_net());
    }

    /// A mono unit must be rejected rather than tripping push's arity assert.
    #[test]
    fn wrong_arity_voice_is_rejected() {
        use fundsp::prelude64::*;
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        let mut mono = Net::new(0, 1);
        let n = mono.push(Box::new(dc(0.5)));
        mono.connect_output(n, 0, 0);
        push_voice(&mut seq, 0.0, 1.0, mono);
    }

    /// The scheduler's own timing math: an event at cycle 1 with cps 0.5
    /// starts at 2 seconds and lasts one second at 2 steps per cycle.
    #[test]
    fn event_times_convert_to_seconds() {
        let clock = Clock::with_cps(44100.0, 0.5);
        let pats = Patterns {
            bindings: vec![Binding {
                instrument: "kick".into(),
                pattern: Pattern::steps([Some(1.0), Some(2.0)]),
            }],
        };

        let events = pats.query(Span::new(1.0, 2.0));
        assert_eq!(events.len(), 2);

        let first = &events[0].event;
        let begin_secs = clock.secs_at(first.begin);
        let dur_secs = clock.secs_at(first.end) - begin_secs;
        assert!((begin_secs - 2.0).abs() < 1e-9, "got {begin_secs}");
        assert!((dur_secs - 1.0).abs() < 1e-9, "got {dur_secs}");
    }
}

#[cfg(test)]
mod pass_tests {
    use super::*;
    use crate::pattern::pattern::Pattern;
    use crate::pattern::patterns::Binding;
    use crate::parser::parser::parse;
    use fundsp::sequencer::ReplayMode;

    fn state_with_kick(steps: Vec<Option<f64>>) -> SchedulerState {
        let s = SchedulerState::new();
        let ast = parse("fn kick(f) = sin(f)\n".to_string()).unwrap();
        *s.instruments.lock().unwrap() = Instruments::from_program(&ast);
        *s.patterns.lock().unwrap() = Patterns {
            bindings: vec![Binding {
                instrument: "kick".into(),
                pattern: Pattern::steps(steps),
            }],
        };
        s
    }

    fn peak_over(seq: &mut Sequencer, frames: usize) -> f32 {
        let mut peak = 0.0f32;
        for _ in 0..frames {
            let (l, r) = seq.get_stereo();
            peak = peak.max(l.abs()).max(r.abs());
        }
        peak
    }

    /// End to end: a pattern plus an instrument becomes audible voices in the
    /// sequencer, with no thread and no audio device involved.
    #[test]
    fn a_pass_schedules_audible_voices() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = state_with_kick(vec![Some(220.0), Some(330.0), Some(440.0), Some(550.0)]);
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);

        let mark = schedule_pass(&mut seq, &clock, &state, None);
        // cps 1.0, 0.2s lookahead -> horizon is cycle 0.2.
        assert!((mark.unwrap() - 0.2).abs() < 1e-9, "watermark: {mark:?}");

        // Only the step at cycle 0.0 falls inside [0, 0.2).
        assert!(peak_over(&mut seq, 4410) > 0.5, "the first step should sound");
    }

    /// Re-running with no clock movement must not re-schedule the same notes.
    #[test]
    fn repeated_passes_do_not_double_trigger() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = state_with_kick(vec![Some(220.0)]);
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);

        let mark = schedule_pass(&mut seq, &clock, &state, None);
        let peak_once = peak_over(&mut seq, 2205);

        let mut seq2 = Sequencer::new(0, 2, ReplayMode::None);
        seq2.set_sample_rate(44100.0);
        let mark2 = schedule_pass(&mut seq2, &clock, &state, mark);
        assert_eq!(mark, mark2, "watermark should not move without the clock");
        assert!(peak_over(&mut seq2, 2205) < peak_once * 0.5,
                "second pass should have scheduled nothing");
    }

    /// A stalled scheduler skips missed events instead of firing them late.
    #[test]
    fn a_stale_watermark_is_clamped_to_now() {
        let clock = Clock::with_cps(44100.0, 1.0);
        clock.advance(44100 * 10); // ten seconds elapsed
        let state = state_with_kick(vec![Some(220.0)]);
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);

        // Watermark claims we only got as far as cycle 0, ten cycles ago.
        let mark = schedule_pass(&mut seq, &clock, &state, Some(0.0)).unwrap();
        assert!(mark >= 10.0, "should jump to the present, got {mark}");
    }

    /// An empty pattern set is cheap and silent, which is the state before the
    /// first eval.
    #[test]
    fn empty_patterns_schedule_nothing() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = SchedulerState::new();
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);

        let mark = schedule_pass(&mut seq, &clock, &state, None);
        assert!(mark.is_some());
        assert!(peak_over(&mut seq, 4410) == 0.0);
    }

    /// A pattern naming an instrument that does not exist logs and continues.
    #[test]
    fn missing_instrument_does_not_stop_the_pass() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = SchedulerState::new();
        *state.patterns.lock().unwrap() = Patterns {
            bindings: vec![Binding {
                instrument: "ghost".into(),
                pattern: Pattern::steps(vec![Some(1.0)]),
            }],
        };
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);

        let mark = schedule_pass(&mut seq, &clock, &state, None);
        assert!(mark.is_some(), "pass should complete despite the bad instrument");
    }
}