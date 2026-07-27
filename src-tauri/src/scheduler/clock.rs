use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Default tempo: one cycle every two seconds, which is 120 bpm.
pub const DEFAULT_CPS: f64 = 0.5;

/// One cycle is four beats, so 120 bpm is the 0.5 cps default tempo. Shared
/// with the language's `bpm` builtin, which must agree with the transport.
pub const BEATS_PER_CYCLE: f64 = 4.0;

pub fn cps_from_bpm(bpm: f64) -> f64 {
    bpm / 60.0 / BEATS_PER_CYCLE
}

pub fn bpm_from_cps(cps: f64) -> f64 {
    cps * 60.0 * BEATS_PER_CYCLE
}

/// How far ahead of the present `reset` places cycle 0.
///
/// The scheduler works ahead of the audio clock and matches onsets half-open,
/// so a window never reaches back for a step it has already passed. Pinning
/// cycle 0 to "now" hands it an origin the next pass has already walked past:
/// the step at cycle 0 falls behind that window and never sounds, and the
/// pattern is heard from its second step. The lead-in has to clear one
/// scheduler tick plus the eval that follows it, so the pass that first sees
/// the new bindings still has the whole first cycle in front of it.
pub const START_LEAD_SECS: f64 = 0.1;

/// Tempo and phase, kept behind one lock because a tempo change moves both:
/// a reader that saw the new rate against the old origin would place cycle 0
/// somewhere the beat never was. Only the command and scheduler threads read
/// them — the audio callback touches nothing but the frame counter — so a
/// mutex here costs nothing real time.
#[derive(Clone, Copy)]
struct Tempo {
    /// Audio time of cycle 0.
    origin_secs: f64,
    /// Cycles per second.
    cps: f64,
    /// Bumped by `reset`. Cycle time jumps backwards there, so anything
    /// measured in cycles against an older epoch is meaningless.
    epoch: u64,
}

/// The clock, in two layers.
///
/// *Audio time* is frames rendered, counted by the audio callback and read by
/// the scheduler — the only source of truth, because `Sequencer::time()`
/// returns `None` once a backend exists.
///
/// *Musical time* is cycles, derived from audio time by the tempo and an
/// **origin** — the audio time at which cycle 0 sits. Re-evaluating never
/// moves the origin, so the beat survives an edit; `reset` moves it to just
/// ahead of the present so playing from silence starts a pattern at its first
/// step.
///
/// The origin exists because the frame counter cannot be zeroed: it is what
/// stays aligned with the sequencer's own internal clock, and rewinding it
/// would put every scheduled start time in the past.
#[derive(Clone)]
pub struct Clock {
    frames: Arc<AtomicU64>,
    tempo: Arc<Mutex<Tempo>>,
    sample_rate: f64,
}

impl Clock {
    pub fn new(sample_rate: f64) -> Self {
        Clock::with_cps(sample_rate, DEFAULT_CPS)
    }

    pub fn with_cps(sample_rate: f64, cps: f64) -> Self {
        Clock {
            frames: Arc::new(AtomicU64::new(0)),
            tempo: Arc::new(Mutex::new(Tempo {
                origin_secs: 0.0,
                cps,
                epoch: 0,
            })),
            sample_rate,
        }
    }

    /// A consistent view of tempo and phase.
    ///
    /// Poisoning is recovered from rather than propagated: every writer here
    /// leaves the tempo whole, and killing the scheduler thread over a panic
    /// elsewhere would take the music with it.
    fn tempo(&self) -> Tempo {
        *self.tempo.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called by the audio callback, once per buffer, after rendering it.
    /// Post-increment keeps this in step with the sequencer's own internal
    /// time, so the absolute start times we push line up with what it renders.
    pub fn advance(&self, frames: u64) {
        self.frames.fetch_add(frames, Ordering::Relaxed);
    }

    /// Seconds of audio rendered so far — the scheduler's "now".
    pub fn now_secs(&self) -> f64 {
        self.frames.load(Ordering::Relaxed) as f64 / self.sample_rate
    }

    pub fn now_cycles(&self) -> f64 {
        self.cycles_at(self.now_secs())
    }

    /// Audio time of cycle 0.
    pub fn origin_secs(&self) -> f64 {
        self.tempo().origin_secs
    }

    /// Put cycle 0 just ahead of the present, so the next pattern starts at its
    /// first step. Called when playing from silence and when stopping — never
    /// on a re-eval while something is already playing, which is what keeps an
    /// edit from jolting the groove.
    ///
    /// The lead-in is what makes the first step audible rather than merely
    /// nominal; see `START_LEAD_SECS`.
    pub fn reset(&self) {
        let now = self.now_secs();
        let mut tempo = self.tempo.lock().unwrap_or_else(|e| e.into_inner());
        tempo.origin_secs = now + START_LEAD_SECS;
        tempo.epoch = tempo.epoch.wrapping_add(1);
    }

    /// How many times cycle time has jumped backwards. A cycle figure only
    /// means anything against the epoch it was measured in.
    pub fn epoch(&self) -> u64 {
        self.tempo().epoch
    }

    /// Change the tempo without moving the beat.
    ///
    /// The origin moves to hold the current cycle position fixed: a change
    /// made three quarters of the way through a cycle stays three quarters of
    /// the way through, and only the rate ahead of it differs. Zeroing the
    /// phase instead would stutter the groove on every drag of a tempo
    /// control.
    ///
    /// The epoch does not move: cycle time stays continuous, so watermarks
    /// taken before the change are still good.
    ///
    /// A tempo that is not a positive, finite number is ignored — there is no
    /// sensible beat for it, and it would poison every time the scheduler
    /// computes from here on.
    pub fn set_cps(&self, cps: f64) {
        if !cps.is_finite() || cps <= 0.0 {
            return;
        }
        let now = self.now_secs();
        let mut tempo = self.tempo.lock().unwrap_or_else(|e| e.into_inner());
        let cycles = (now - tempo.origin_secs) * tempo.cps;
        tempo.cps = cps;
        tempo.origin_secs = now - cycles / cps;
    }

    pub fn cycles_at(&self, secs: f64) -> f64 {
        let tempo = self.tempo();
        (secs - tempo.origin_secs) * tempo.cps
    }

    pub fn secs_at(&self, cycles: f64) -> f64 {
        let tempo = self.tempo();
        cycles / tempo.cps + tempo.origin_secs
    }

    pub fn cps(&self) -> f64 {
        self.tempo().cps
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_convert_to_seconds() {
        let clock = Clock::new(44100.0);
        assert_eq!(clock.now_secs(), 0.0);

        clock.advance(44100);
        assert!((clock.now_secs() - 1.0).abs() < 1e-9);

        clock.advance(22050);
        assert!((clock.now_secs() - 1.5).abs() < 1e-9);
    }

    /// A clone shares the counter — this is what lets the scheduler thread
    /// read what the audio thread wrote.
    #[test]
    fn clones_share_the_counter() {
        let clock = Clock::new(48000.0);
        let reader = clock.clone();

        clock.advance(48000);
        assert!((reader.now_secs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn accumulates_across_buffers() {
        let clock = Clock::new(44100.0);
        for _ in 0..100 {
            clock.advance(441); // 10ms buffers
        }
        assert!((clock.now_secs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn seconds_and_cycles_round_trip() {
        let clock = Clock::with_cps(44100.0, 0.5);
        assert!((clock.cycles_at(2.0) - 1.0).abs() < 1e-9);
        assert!((clock.secs_at(1.0) - 2.0).abs() < 1e-9);

        for cycles in [0.0, 0.25, 3.75, 100.5] {
            let round = clock.cycles_at(clock.secs_at(cycles));
            assert!((round - cycles).abs() < 1e-9, "round trip failed for {cycles}");
        }
    }

    #[test]
    fn tempo_scales_cycle_time() {
        let fast = Clock::with_cps(44100.0, 2.0);
        assert!((fast.cycles_at(1.0) - 2.0).abs() < 1e-9);
        assert!((fast.secs_at(2.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn now_cycles_tracks_rendered_audio() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.advance(44100 * 4); // four seconds
        assert!((clock.now_cycles() - 2.0).abs() < 1e-9);
    }

    /// Reset puts cycle 0 just ahead of the present without touching the frame
    /// counter, which has to stay aligned with the sequencer's own clock.
    #[test]
    fn reset_moves_cycle_zero_to_just_ahead_of_now() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.advance(44100 * 5); // five seconds in
        assert!((clock.now_cycles() - 2.5).abs() < 1e-9);

        clock.reset();
        // Audio time is untouched: only the origin moved.
        assert!((clock.now_secs() - 5.0).abs() < 1e-9);
        assert!((clock.origin_secs() - (5.0 + START_LEAD_SECS)).abs() < 1e-9);
        // The present sits a lead-in *before* cycle 0, which is what leaves the
        // first step in front of the scheduler rather than behind it.
        assert!(clock.now_cycles() < 0.0, "cycle 0 must still be ahead");
        assert!((clock.now_cycles() + START_LEAD_SECS * 0.5).abs() < 1e-9);
    }

    /// The bug the lead-in exists for: the scheduler reads the clock a tick
    /// after the eval that reset it, and its window has to still contain
    /// cycle 0 by then.
    #[test]
    fn cycle_zero_is_still_ahead_one_scheduler_tick_later() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.reset();
        // The audio thread keeps rendering while the scheduler sleeps.
        clock.advance((44100.0 * super::super::scheduler::TICK.as_secs_f64()) as u64);

        assert!(
            clock.now_cycles() < 0.0,
            "a pass one tick after the reset must not have passed cycle 0, got {}",
            clock.now_cycles(),
        );
    }

    /// After a reset, cycle 0 maps back to the audio time it was pinned to —
    /// so a scheduled start time still lands in the sequencer's future.
    #[test]
    fn seconds_and_cycles_round_trip_after_a_reset() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.advance(44100 * 7);
        clock.reset();

        assert!((clock.secs_at(0.0) - (7.0 + START_LEAD_SECS)).abs() < 1e-9);
        assert!((clock.secs_at(1.0) - (9.0 + START_LEAD_SECS)).abs() < 1e-9);
        for cycles in [0.0, 0.25, 3.75, 100.5] {
            let round = clock.cycles_at(clock.secs_at(cycles));
            assert!((round - cycles).abs() < 1e-9, "round trip failed for {cycles}");
        }
    }

    /// Clones share the origin, so the scheduler thread sees a reset made on
    /// the command thread.
    #[test]
    fn clones_share_the_origin() {
        let clock = Clock::with_cps(44100.0, 0.5);
        let reader = clock.clone();
        clock.advance(44100 * 4);
        clock.reset();
        assert!((reader.origin_secs() - (4.0 + START_LEAD_SECS)).abs() < 1e-9);
    }

    /// A tempo change is heard from here on, not retroactively: the beat we
    /// are standing on does not move.
    #[test]
    fn changing_tempo_holds_the_current_cycle() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.advance(44100 * 5); // 2.5 cycles in
        assert!((clock.now_cycles() - 2.5).abs() < 1e-9);

        clock.set_cps(1.0);
        assert!((clock.now_cycles() - 2.5).abs() < 1e-9, "the beat must not jump");

        // And from here a cycle takes a second rather than two.
        clock.advance(44100);
        assert!((clock.now_cycles() - 3.5).abs() < 1e-9);
    }

    /// Times still round trip after a tempo change, at the new rate.
    #[test]
    fn seconds_and_cycles_round_trip_after_a_tempo_change() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.advance(44100 * 3);
        clock.set_cps(2.0);

        assert!((clock.cps() - 2.0).abs() < 1e-9);
        for cycles in [0.0, 0.25, 3.75, 100.5] {
            let round = clock.cycles_at(clock.secs_at(cycles));
            assert!((round - cycles).abs() < 1e-9, "round trip failed for {cycles}");
        }
    }

    /// Nonsense from a control surface must not poison every later reading.
    #[test]
    fn a_bad_tempo_is_ignored() {
        let clock = Clock::with_cps(44100.0, 0.5);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            clock.set_cps(bad);
            assert!((clock.cps() - 0.5).abs() < 1e-9, "{bad} should have been ignored");
        }
        assert!(clock.now_cycles().is_finite());
    }

    /// Clones share the tempo, so the scheduler thread sees a change made on
    /// the command thread.
    #[test]
    fn clones_share_the_tempo() {
        let clock = Clock::with_cps(44100.0, 0.5);
        let reader = clock.clone();
        clock.set_cps(1.5);
        assert!((reader.cps() - 1.5).abs() < 1e-9);
    }

    /// The epoch marks the discontinuity: a reset moves cycle time, a tempo
    /// change does not.
    #[test]
    fn only_a_reset_moves_the_epoch() {
        let clock = Clock::with_cps(44100.0, 0.5);
        let start = clock.epoch();

        clock.set_cps(1.0);
        assert_eq!(clock.epoch(), start, "tempo changes keep cycle time continuous");

        clock.reset();
        assert_ne!(clock.epoch(), start, "a reset invalidates cycle figures");
    }

    #[test]
    fn beats_per_minute_convert_to_cycles_per_second() {
        assert!((cps_from_bpm(120.0) - DEFAULT_CPS).abs() < 1e-9);
        assert!((bpm_from_cps(DEFAULT_CPS) - 120.0).abs() < 1e-9);
        for bpm in [20.0, 96.0, 135.0, 300.0] {
            assert!((bpm_from_cps(cps_from_bpm(bpm)) - bpm).abs() < 1e-9);
        }
    }

    /// Time keeps running after a reset, from the new zero — which the lead-in
    /// puts a moment after the reset itself.
    #[test]
    fn cycles_advance_from_the_new_origin() {
        let clock = Clock::with_cps(44100.0, 0.5);
        clock.advance(44100 * 5);
        clock.reset();
        clock.advance((44100.0 * (2.0 + START_LEAD_SECS)) as u64);
        assert!((clock.now_cycles() - 1.0).abs() < 1e-6);
    }
}
