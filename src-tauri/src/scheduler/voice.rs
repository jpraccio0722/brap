//! Building one voice: an instrument function plus an event value, run through
//! the ordinary lowering and realization pipeline.

use fundsp::net::Net;

use crate::scree_graph::realizer::realize;
use crate::lowerer::lower::lower_voice;
use crate::parser::parser::{Arg, ScreeItem, Expr, Ident};

/// The instrument definitions from the most recent eval.
///
/// These are the `fn` items of the program, kept verbatim so a voice can be
/// lowered on demand. An eval replaces this wholesale, exactly like `Patterns`.
#[derive(Clone, Debug, Default)]
pub struct Instruments {
    pub defs: Vec<ScreeItem>,
}

impl Instruments {
    /// Keep only the function definitions; everything else in a program is
    /// the persistent graph's business, not a voice's.
    pub fn from_program(items: &[ScreeItem]) -> Instruments {
        Instruments {
            defs: items
                .iter()
                .filter(|i| matches!(i, ScreeItem::Function { .. }))
                .cloned()
                .collect(),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.param_count(name).is_some()
    }

    /// How many parameters an instrument declares, or `None` if there is no
    /// such instrument. A zero-parameter instrument is called with no
    /// arguments, so a fixed drum needs no placeholder parameter.
    pub fn param_count(&self, name: &str) -> Option<usize> {
        self.defs.iter().find_map(|i| match i {
            ScreeItem::Function { name: n, params, .. } if n.0 == name => Some(params.len()),
            _ => None,
        })
    }
}

/// Lower and realize `instrument(value, name: v, ...)` into a playable
/// 0-in / 2-out network.
///
/// Synthesizing a call and running the normal pipeline means voices get every
/// language feature for free — `for`, `if`, nested calls, the lot. Lane values
/// are passed by name for the same reason: a parameter no lane filled then
/// falls to its own default, evaluated in the callee's scope where the earlier
/// parameters are already bound.
pub fn build_voice(
    instruments: &Instruments,
    instrument: &str,
    value: f64,
    lanes: &[(String, f64)],
    dur_secs: f64,
) -> Result<Net, String> {
    let Some(params) = instruments.param_count(instrument) else {
        return Err(format!("no instrument named `{instrument}`"));
    };

    // An instrument that declares no parameters is called with none — the
    // event's value is simply unused, which is what `\` in a pattern means.
    let mut args = if params == 0 { vec![] } else { vec![Arg::positional(Expr::Num(value))] };
    args.extend(lanes.iter().map(|(name, v)| Arg::named(name, Expr::Num(*v))));

    let mut items = instruments.defs.clone();
    items.push(ScreeItem::Expr(Expr::Call {
        func: Ident(instrument.to_string()),
        args,
    }));

    let lowered = lower_voice(&items, dur_secs)?;
    realize(&lowered.graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parser::parse;
    use fundsp::prelude64::AudioUnit;

    fn instruments(src: &str) -> Instruments {
        Instruments::from_program(&parse(src.to_string()).expect("parse failed"))
    }

    #[test]
    fn only_function_items_are_kept() {
        let ins = instruments("fn kick(f) = sin(f)\nlet x = 3\nsin(220)\n");
        assert_eq!(ins.defs.len(), 1);
        assert!(ins.has("kick"));
        assert!(!ins.has("nope"));
    }

    /// A voice is always 0-in / 2-out, which is what `Sequencer::push` asserts.
    #[test]
    fn voice_is_stereo_and_sourceless() {
        let ins = instruments("fn kick(f) = sin(f) / 4\n");
        let net = build_voice(&ins, "kick", 220.0, &[], 1.0).expect("should build");
        assert_eq!(net.inputs(), 0);
        assert_eq!(net.outputs(), 2);
    }

    /// The event value really reaches the instrument's parameter.
    #[test]
    fn event_value_reaches_the_instrument() {
        let ins = instruments("fn tone(f) = sin(f)\n");
        let mut net = build_voice(&ins, "tone", 110.0, &[], 1.0).expect("should build");
        net.set_sample_rate(44100.0);

        let samples: Vec<f32> = (0..44100).map(|_| net.get_mono()).collect();
        assert!(samples.iter().all(|s| s.is_finite()));

        // A 110 Hz sine crosses zero going positive 110 times per second.
        let crossings = samples
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        assert!(
            (crossings as i64 - 110).abs() <= 1,
            "expected ~110 rising zero crossings, got {crossings}"
        );
    }

    /// Instruments can use the full language, not a restricted subset.
    #[test]
    fn voices_may_use_language_features() {
        let ins = instruments("fn rich(f) = for i in 1..=3 { sin(f * i) / 6 }\n");
        let net = build_voice(&ins, "rich", 110.0, &[], 1.0).expect("should build");
        assert_eq!(net.outputs(), 2);
    }

    /// A voice is lowered afresh for each note, so a random draw written inside
    /// an instrument is a new number every time it sounds — the counterpart to
    /// a draw in a *pattern*, which settles once per eval. Both are documented
    /// behaviour, and this is the half that only exists here.
    #[test]
    fn a_draw_inside_an_instrument_is_rerolled_per_note() {
        // Detune is inaudible but moves the pitch, so counting zero crossings
        // reads back what the draw was.
        let ins = instruments("fn tone(f) = sin(f + randi(0, 40))\n");
        let crossings = |_| {
            let mut net = build_voice(&ins, "tone", 200.0, &[], 1.0).expect("should build");
            net.set_sample_rate(44100.0);
            let samples: Vec<f32> = (0..44100).map(|_| net.get_mono()).collect();
            samples.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
        };
        let counts: std::collections::HashSet<usize> = (0..12).map(crossings).collect();
        assert!(counts.len() > 1, "every note drew the same number: {counts:?}");
        assert!(counts.iter().all(|c| (200..=240).contains(c)), "out of range: {counts:?}");
    }

    #[test]
    fn unknown_instrument_is_an_error() {
        let ins = instruments("fn kick(f) = sin(f)\n");
        // Net has no Debug, so unwrap_err() is unavailable here.
        let err = match build_voice(&ins, "snare", 1.0, &[], 1.0) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for an unknown instrument"),
        };
        assert!(err.contains("no instrument"), "got: {err}");
    }

    /// Lowering errors surface as errors, not panics — the scheduler logs and
    /// keeps running.
    #[test]
    fn broken_instrument_reports_an_error() {
        let ins = instruments("fn bad(f) = nope(f)\n");
        assert!(build_voice(&ins, "bad", 1.0, &[], 1.0).is_err());
    }

    // ---- lanes ----

    fn rising_crossings(ins: &Instruments, lanes: &[(String, f64)]) -> usize {
        let mut net = build_voice(ins, "tone", 110.0, lanes, 1.0).expect("should build");
        net.set_sample_rate(44100.0);
        let s: Vec<f32> = (0..44100).map(|_| net.get_mono()).collect();
        s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    /// A lane value really reaches the parameter it names: doubling `mul`
    /// doubles the pitch of a 110 Hz tone.
    #[test]
    fn a_lane_value_reaches_its_parameter() {
        let ins = instruments("fn tone(n, mul = 1) = sin(n * mul)\n");

        let plain = rising_crossings(&ins, &[]);
        let doubled = rising_crossings(&ins, &[("mul".to_string(), 2.0)]);

        assert!((plain as i64 - 110).abs() <= 1, "expected ~110, got {plain}");
        assert!((doubled as i64 - 220).abs() <= 1, "expected ~220, got {doubled}");
    }

    /// Lanes bind by name, so the order the scheduler happens to collect them
    /// in cannot matter.
    #[test]
    fn lane_order_does_not_matter() {
        let ins = instruments("fn tone(n, mul = 1, div = 1) = sin(n * mul / div)\n");

        let forward = rising_crossings(
            &ins, &[("mul".to_string(), 4.0), ("div".to_string(), 2.0)]);
        let backward = rising_crossings(
            &ins, &[("div".to_string(), 2.0), ("mul".to_string(), 4.0)]);

        assert_eq!(forward, backward);
        assert!((forward as i64 - 220).abs() <= 1, "expected ~220, got {forward}");
    }

    /// A parameter no lane filled falls to its own default — which is what a
    /// resting lane relies on.
    #[test]
    fn an_unfilled_parameter_uses_its_default() {
        let ins = instruments("fn tone(n, mul = 2) = sin(n * mul)\n");
        assert!((rising_crossings(&ins, &[]) as i64 - 220).abs() <= 1);
    }

    /// A default that reads an earlier parameter still works when a later lane
    /// is supplied by name — the reason lanes are passed as named arguments
    /// rather than flattened into positions.
    #[test]
    fn a_default_may_read_the_event_value() {
        let ins = instruments("fn tone(n, hz = n * 2, mul = 1) = sin(hz * mul)\n");
        assert!((rising_crossings(&ins, &[("mul".to_string(), 1.0)]) as i64 - 220).abs() <= 1);
    }

    /// A lane the instrument has no parameter for is refused at bind time, but
    /// the voice builder must not panic if one reaches it anyway.
    #[test]
    fn an_unknown_lane_is_an_error_not_a_panic() {
        let ins = instruments("fn tone(n) = sin(n)\n");
        let err = match build_voice(&ins, "tone", 110.0, &[("nope".to_string(), 1.0)], 1.0) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for an unknown lane"),
        };
        assert!(err.contains("no parameter named 'nope'"), "got: {err}");
    }
}

#[cfg(test)]
mod envelope_voice_tests {
    use super::*;
    use crate::lowerer::lower::lower;
    use crate::parser::parser::parse;
    use fundsp::prelude64::AudioUnit;

    const PROGRAM: &str = "\
fn kick(f) = sin(f) * perc(0.001, 0.25)
fn bass(f) = (saw(f) >> lowpass(800, 1)) * env(0.01, 0.15, 0.4, 0.2, dur)

play([55, `, 55, [55, 55]], kick)
play([110, 165], bass, 0.5)
";

    /// The full example: two enveloped instruments plus two patterns.
    #[test]
    fn the_example_program_lowers_and_binds() {
        let items = parse(PROGRAM.to_string()).expect("parse failed");
        let lowered = lower(&items).expect("lower failed");
        assert_eq!(lowered.bindings.len(), 2);

        let ins = Instruments::from_program(&items);
        assert!(ins.has("kick") && ins.has("bass"));
    }

    /// Both instruments build into playable voices and go quiet by note end.
    #[test]
    fn enveloped_voices_start_immediately_and_decay() {
        let ins = Instruments::from_program(&parse(PROGRAM.to_string()).unwrap());

        for (name, freq, dur) in [("kick", 55.0, 1.0), ("bass", 110.0, 1.0)] {
            let mut net = build_voice(&ins, name, freq, &[], dur).expect("should build");
            assert_eq!(net.outputs(), 2);
            net.set_sample_rate(44100.0);

            let s: Vec<f32> = (0..44100).map(|_| net.get_mono()).collect();
            // Audible within the first 50 ms — no swallowed first note.
            let onset = s[..2205].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            // The last millisecond: `env`'s release lands exactly on `dur`, so
            // anything earlier is still legitimately ringing out.
            let tail = s[44050..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(onset > 0.05, "{name} should sound immediately, got {onset}");
            assert!(tail < 0.02, "{name} should be quiet by the note end, got {tail}");
        }
    }
}

#[cfg(test)]
mod kick_example_tests {
    use super::*;
    use fundsp::prelude64::AudioUnit;

    /// A synth kick. The pattern number is the fundamental; everything else is
    /// shaped by envelopes inside the instrument.
    const KICK: &str = "\
fn kick(f) = {
  let sweep = f + f * 3 * perc(0.001, 0.05)
  let body  = sin(sweep) * perc(0.002, 0.4)
  let click = noise() * perc(0.001, 0.006)
  body * 0.9 + click * 0.1
}
";

    fn render(freq: f64, dur: f64, secs: f64) -> Vec<f32> {
        let ins = Instruments::from_program(
            &crate::parser::parser::parse(KICK.to_string()).expect("parse failed"),
        );
        let mut net = build_voice(&ins, "kick", freq, &[], dur).expect("should build");
        net.set_sample_rate(44100.0);
        (0..(secs * 44100.0) as usize).map(|_| net.get_mono()).collect()
    }

    fn rising_crossings(s: &[f32]) -> usize {
        s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn the_kick_sounds_and_decays() {
        let s = render(50.0, 1.0, 1.0);
        assert!(s.iter().all(|v| v.is_finite()));

        let onset = s[..2205].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let tail = s[35000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(onset > 0.5, "should hit hard, peak {onset}");
        assert!(tail < 0.02, "should have decayed, peak {tail}");
        assert!(s.iter().all(|v| v.abs() <= 1.05), "should not clip badly");
    }

    /// The pitch envelope really sweeps: the first 50 ms contains far more
    /// cycles than a later window of the same length.
    #[test]
    fn the_pitch_sweeps_downward() {
        let s = render(50.0, 1.0, 1.0);
        let early = rising_crossings(&s[..2205]);        // 0 - 50 ms
        let late = rising_crossings(&s[6615..8820]);     // 150 - 200 ms
        assert!(
            early > late * 2,
            "expected a downward sweep, early={early} late={late}"
        );
    }

    /// The pattern number scales the whole drum, so it is a real parameter.
    #[test]
    fn the_pattern_number_sets_the_fundamental() {
        let low = rising_crossings(&render(40.0, 1.0, 0.4)[4410..8820]);
        let high = rising_crossings(&render(120.0, 1.0, 0.4)[4410..8820]);
        assert!(high > low * 2, "120 Hz should cycle faster: {high} vs {low}");
    }
}

#[cfg(test)]
mod zero_param_tests {
    use super::*;
    use crate::parser::parser::parse;
    use fundsp::prelude64::AudioUnit;

    fn instruments(src: &str) -> Instruments {
        Instruments::from_program(&parse(src.to_string()).expect("parse failed"))
    }

    /// A fixed drum needs no placeholder parameter.
    #[test]
    fn a_zero_parameter_instrument_builds() {
        let ins = instruments("fn kick() = sin(50) * perc(0.002, 0.3)\n");
        assert_eq!(ins.param_count("kick"), Some(0));

        let mut net = build_voice(&ins, "kick", 1.0, &[], 1.0).expect("should build");
        assert_eq!(net.outputs(), 2);
        net.set_sample_rate(44100.0);

        let s: Vec<f32> = (0..44100).map(|_| net.get_mono()).collect();
        let onset = s[..2205].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(onset > 0.5, "should sound, peak {onset}");
    }

    /// The event value is ignored, so every trigger sounds identical.
    #[test]
    fn the_event_value_is_ignored() {
        let ins = instruments("fn kick() = sin(50) * perc(0.002, 0.3)\n");

        let render = |v: f64| {
            let mut net = build_voice(&ins, "kick", v, &[], 1.0).unwrap();
            net.set_sample_rate(44100.0);
            (0..4410).map(|_| net.get_mono()).collect::<Vec<f32>>()
        };
        assert_eq!(render(1.0), render(9999.0), "the value must not matter");
    }

    /// One-parameter instruments still receive the value.
    #[test]
    fn one_parameter_instruments_are_unaffected() {
        let ins = instruments("fn tone(f) = sin(f)\n");
        assert_eq!(ins.param_count("tone"), Some(1));
        assert!(build_voice(&ins, "tone", 220.0, &[], 1.0).is_ok());
    }

    /// Declaring a parameter and ignoring it still works, for compatibility.
    #[test]
    fn an_ignored_parameter_still_works() {
        let ins = instruments("fn kick(_) = sin(50) * perc(0.002, 0.3)\n");
        assert_eq!(ins.param_count("kick"), Some(1));
        assert!(build_voice(&ins, "kick", 1.0, &[], 1.0).is_ok());
    }
}
