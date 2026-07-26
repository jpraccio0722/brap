use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Default tempo: one cycle every two seconds.
pub const DEFAULT_CPS: f64 = 0.5;

/// The clock, in two layers.
///
/// *Audio time* is frames rendered, counted by the audio callback and read by
/// the scheduler — the only source of truth, because `Sequencer::time()`
/// returns `None` once a backend exists.
///
/// *Musical time* is cycles, derived from audio time by the tempo. Cycle 0 is
/// audio time 0, so the beat survives every re-eval.
#[derive(Clone)]
pub struct Clock {
    frames: Arc<AtomicU64>,
    sample_rate: f64,
    /// Cycles per second.
    cps: f64,
}

impl Clock {
    pub fn new(sample_rate: f64) -> Self {
        Clock::with_cps(sample_rate, DEFAULT_CPS)
    }

    pub fn with_cps(sample_rate: f64, cps: f64) -> Self {
        Clock { frames: Arc::new(AtomicU64::new(0)), sample_rate, cps }
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

    pub fn cycles_at(&self, secs: f64) -> f64 {
        secs * self.cps
    }

    pub fn secs_at(&self, cycles: f64) -> f64 {
        cycles / self.cps
    }

    pub fn cps(&self) -> f64 {
        self.cps
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
}
