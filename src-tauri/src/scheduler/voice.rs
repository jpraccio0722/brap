//! Building one voice: an instrument function plus an event value, run through
//! the ordinary lowering and realization pipeline.

use fundsp::net::Net;

use crate::brap_graph::realizer::realize;
use crate::lowerer::lower::lower;
use crate::parser::parser::{BrapItem, Expr, Ident};

/// The instrument definitions from the most recent eval.
///
/// These are the `fn` items of the program, kept verbatim so a voice can be
/// lowered on demand. An eval replaces this wholesale, exactly like `Patterns`.
#[derive(Clone, Debug, Default)]
pub struct Instruments {
    pub defs: Vec<BrapItem>,
}

impl Instruments {
    /// Keep only the function definitions; everything else in a program is
    /// the persistent graph's business, not a voice's.
    pub fn from_program(items: &[BrapItem]) -> Instruments {
        Instruments {
            defs: items
                .iter()
                .filter(|i| matches!(i, BrapItem::Function { .. }))
                .cloned()
                .collect(),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.defs.iter().any(|i| match i {
            BrapItem::Function { name: n, .. } => n.0 == name,
            _ => false,
        })
    }
}

/// Lower and realize `instrument(value)` into a playable 0-in / 2-out network.
///
/// Synthesizing a call and running the normal pipeline means voices get every
/// language feature for free — `for`, `if`, nested calls, the lot.
pub fn build_voice(
    instruments: &Instruments,
    instrument: &str,
    value: f64,
) -> Result<Net, String> {
    if !instruments.has(instrument) {
        return Err(format!("no instrument named `{instrument}`"));
    }

    let mut items = instruments.defs.clone();
    items.push(BrapItem::Expr(Expr::Call {
        func: Ident(instrument.to_string()),
        args: vec![Expr::Num(value)],
    }));

    let lowered = lower(&items)?;
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
        let net = build_voice(&ins, "kick", 220.0).expect("should build");
        assert_eq!(net.inputs(), 0);
        assert_eq!(net.outputs(), 2);
    }

    /// The event value really reaches the instrument's parameter.
    #[test]
    fn event_value_reaches_the_instrument() {
        let ins = instruments("fn tone(f) = sin(f)\n");
        let mut net = build_voice(&ins, "tone", 110.0).expect("should build");
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
        let net = build_voice(&ins, "rich", 110.0).expect("should build");
        assert_eq!(net.outputs(), 2);
    }

    #[test]
    fn unknown_instrument_is_an_error() {
        let ins = instruments("fn kick(f) = sin(f)\n");
        // Net has no Debug, so unwrap_err() is unavailable here.
        let err = match build_voice(&ins, "snare", 1.0) {
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
        assert!(build_voice(&ins, "bad", 1.0).is_err());
    }
}
