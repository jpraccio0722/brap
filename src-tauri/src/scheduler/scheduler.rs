//! The scheduler thread.
//!
//! It free-runs from app start and is never "triggered": an eval simply
//! replaces the patterns and instruments it reads on the next pass. That
//! inversion is what keeps the clock running across re-evals.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fundsp::prelude64::AudioUnit;
use fundsp::sequencer::{EventId, Fade, Sequencer};

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
    /// Raised by `stop`, consumed by the scheduler thread. Clearing the
    /// patterns stops *new* voices; only the thread that owns the sequencer
    /// can cut the ones already pushed into the lookahead window.
    stop: Arc<AtomicBool>,
}

impl SchedulerState {
    pub fn new() -> Self {
        SchedulerState {
            patterns: Arc::new(Mutex::new(Patterns::default())),
            instruments: Arc::new(Mutex::new(Instruments::default())),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Ask the scheduler thread to silence everything it has pushed.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn take_stop(&self) -> bool {
        self.stop.swap(false, Ordering::Acquire)
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

/// A voice we have pushed that may still be sounding. Stop cuts these short;
/// otherwise they retire on their own and we just forget about them.
struct Live {
    id: EventId,
    end_secs: f64,
}

fn run(mut seq: Sequencer, clock: Clock, state: SchedulerState) {
    // `None` until the first pass, so a long idle period before the first eval
    // does not look like a huge backlog to catch up on.
    let mut scheduled_through: Option<f64> = None;
    let mut live: Vec<Live> = Vec::new();

    loop {
        std::thread::sleep(TICK);

        if state.take_stop() {
            silence(&mut seq, &mut live);
            // The horizon restarts from the present on the next eval.
            scheduled_through = None;
            continue;
        }

        scheduled_through = schedule_pass(&mut seq, &clock, &state, scheduled_through, &mut live);
        retire(&mut live, clock.now_secs());
    }
}

/// Cut every voice we have pushed and forget them.
///
/// `edit_relative` with equal end and fade times starts the fade immediately.
/// A voice that has not started yet ends before it begins, so the sequencer
/// retires it without ever rendering a sample.
fn silence(seq: &mut Sequencer, live: &mut Vec<Live>) {
    for voice in live.drain(..) {
        seq.edit_relative(voice.id, FADE_OUT_SECS, FADE_OUT_SECS);
    }
}

/// Drop voices the sequencer has already finished with, so the list tracks the
/// lookahead window rather than growing for the life of the app.
fn retire(live: &mut Vec<Live>, now_secs: f64) {
    live.retain(|voice| voice.end_secs > now_secs);
}

/// One pass of the loop: query the horizon, push whatever falls in it, and
/// return the new watermark. Split out from `run` so it can be tested without
/// threads or sleeping.
fn schedule_pass(
    seq: &mut Sequencer,
    clock: &Clock,
    state: &SchedulerState,
    scheduled_through: Option<f64>,
    live: &mut Vec<Live>,
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

    // A stop that landed while we were reading state wins: these events were
    // queried before it, and pushing them now would sound after the silence.
    if state.stop_requested() {
        return None;
    }

    for bound in events {
        let begin_secs = clock.secs_at(bound.event.begin);
        let dur_secs = clock.secs_at(bound.event.end) - begin_secs;

        match build_voice(&instruments, &bound.instrument, bound.event.value, dur_secs) {
            // A bad instrument must not kill the thread: log it and let the
            // rest of the pattern keep playing.
            Err(e) => eprintln!("scheduler: {}: {e}", bound.instrument),
            Ok(net) => {
                if let Some(id) = push_voice(seq, begin_secs, dur_secs, net) {
                    live.push(Live { id, end_secs: begin_secs + dur_secs });
                }
            }
        }
    }

    next
}

/// Push one voice, defending against every case `Sequencer::push` asserts on.
/// Returns the event's id so stop can cut it short, or `None` if it was
/// rejected and nothing was pushed.
fn push_voice(
    seq: &mut Sequencer,
    start_secs: f64,
    dur_secs: f64,
    net: fundsp::net::Net,
) -> Option<EventId> {
    if !start_secs.is_finite() || !dur_secs.is_finite() || dur_secs <= 0.0 {
        eprintln!("scheduler: skipping voice with bad timing ({start_secs}, {dur_secs})");
        return None;
    }
    if net.inputs() != 0 || net.outputs() != 2 {
        eprintln!(
            "scheduler: voice must be 0-in/2-out, got {}-in/{}-out",
            net.inputs(),
            net.outputs()
        );
        return None;
    }

    // push asserts each fade is no longer than the event; short notes clamp.
    let half = dur_secs * 0.5;
    let fade_in = FADE_IN_SECS.min(half);
    let fade_out = FADE_OUT_SECS.min(half);

    Some(seq.push_duration(start_secs, dur_secs, Fade::Smooth, fade_in, fade_out, Box::new(net)))
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
        assert!(push_voice(&mut seq, 0.0, 1.0, voice_net()).is_some());

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
        assert!(push_voice(&mut seq, 0.0, 0.001, voice_net()).is_some());

        for _ in 0..100 {
            let _ = seq.get_stereo();
        }
    }

    #[test]
    fn bad_timing_is_skipped_not_panicked() {
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        assert!(push_voice(&mut seq, 0.0, 0.0, voice_net()).is_none());
        assert!(push_voice(&mut seq, f64::NAN, 1.0, voice_net()).is_none());
        assert!(push_voice(&mut seq, 0.0, -1.0, voice_net()).is_none());
    }

    /// A mono unit must be rejected rather than tripping push's arity assert.
    #[test]
    fn wrong_arity_voice_is_rejected() {
        use fundsp::prelude64::*;
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        let mut mono = Net::new(0, 1);
        let n = mono.push(Box::new(dc(0.5)));
        mono.connect_output(n, 0, 0);
        assert!(push_voice(&mut seq, 0.0, 1.0, mono).is_none());
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

    /// A pass for the tests that do not care which voices came out of it.
    fn pass(
        seq: &mut Sequencer,
        clock: &Clock,
        state: &SchedulerState,
        from: Option<f64>,
    ) -> Option<f64> {
        schedule_pass(seq, clock, state, from, &mut Vec::new())
    }

    /// End to end: a pattern plus an instrument becomes audible voices in the
    /// sequencer, with no thread and no audio device involved.
    #[test]
    fn a_pass_schedules_audible_voices() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = state_with_kick(vec![Some(220.0), Some(330.0), Some(440.0), Some(550.0)]);
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);

        let mark = pass(&mut seq, &clock, &state, None);
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

        let mark = pass(&mut seq, &clock, &state, None);
        let peak_once = peak_over(&mut seq, 2205);

        let mut seq2 = Sequencer::new(0, 2, ReplayMode::None);
        seq2.set_sample_rate(44100.0);
        let mark2 = pass(&mut seq2, &clock, &state, mark);
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
        let mark = pass(&mut seq, &clock, &state, Some(0.0)).unwrap();
        assert!(mark >= 10.0, "should jump to the present, got {mark}");
    }

    /// An empty pattern set is cheap and silent, which is the state before the
    /// first eval.
    #[test]
    fn empty_patterns_schedule_nothing() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = SchedulerState::new();
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);

        let mark = pass(&mut seq, &clock, &state, None);
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

        let mark = pass(&mut seq, &clock, &state, None);
        assert!(mark.is_some(), "pass should complete despite the bad instrument");
    }

    /// The bug this guards: a stop that clears the patterns still leaves the
    /// voices already pushed into the lookahead window sounding.
    #[test]
    fn stop_cuts_a_sounding_voice() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = state_with_kick(vec![Some(220.0)]);
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);

        let mut live = Vec::new();
        schedule_pass(&mut seq, &clock, &state, None, &mut live);
        assert_eq!(live.len(), 1, "the pass should have pushed one voice");

        // The voice lasts a full cycle; interrupt it 50ms in.
        assert!(peak_over(&mut seq, 2205) > 0.5, "voice should be sounding");

        silence(&mut seq, &mut live);
        assert!(live.is_empty(), "stop should forget the voices it cut");

        // Render past the fade out, then listen for what should be silence.
        let _ = peak_over(&mut seq, 1323); // 30ms, comfortably past a 20ms fade
        assert_eq!(peak_over(&mut seq, 22050), 0.0, "nothing should sound after stop");
    }

    /// Voices in the lookahead window have not started yet; stop must cancel
    /// them rather than let them fire on schedule.
    #[test]
    fn stop_cancels_a_voice_that_has_not_started() {
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);

        let items = parse("sin(220)\n".to_string()).unwrap();
        let net = crate::brap_graph::realizer::realize(
            &crate::lowerer::lower::lower(&items).unwrap().graph,
        )
        .unwrap();

        let id = push_voice(&mut seq, 0.1, 0.5, net).unwrap();
        let mut live = vec![Live { id, end_secs: 0.6 }];

        silence(&mut seq, &mut live);
        assert_eq!(peak_over(&mut seq, 44100), 0.0, "the voice should never sound");
    }

    /// A stop landing mid-pass must abort it, or the pass pushes voices the
    /// stop has already walked past.
    #[test]
    fn a_pending_stop_aborts_the_pass() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = state_with_kick(vec![Some(220.0)]);
        state.request_stop();

        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);
        let mut live = Vec::new();

        let mark = schedule_pass(&mut seq, &clock, &state, None, &mut live);
        assert!(mark.is_none(), "an aborted pass must not claim a watermark");
        assert!(live.is_empty(), "nothing should have been pushed");
        assert_eq!(peak_over(&mut seq, 4410), 0.0);
    }

    /// The live list is a window, not a log: finished voices fall out of it.
    #[test]
    fn finished_voices_are_retired() {
        let clock = Clock::with_cps(44100.0, 1.0);
        let state = state_with_kick(vec![Some(220.0), Some(330.0), Some(440.0), Some(550.0)]);
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(44100.0);
        let mut live = Vec::new();

        // Each step is a quarter cycle, so voice n runs [n/4, (n+1)/4).
        let mut mark = None;
        for _ in 0..4 {
            mark = schedule_pass(&mut seq, &clock, &state, mark, &mut live);
            clock.advance(44100 / 4);
        }
        assert!(live.len() > 1, "several voices should be in flight");

        retire(&mut live, clock.now_secs());
        assert!(
            live.iter().all(|v| v.end_secs > clock.now_secs()),
            "only unfinished voices should remain",
        );
    }
}