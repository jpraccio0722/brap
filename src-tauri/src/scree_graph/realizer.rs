use fundsp::prelude64::*;

use crate::scree_graph::{graph::ScreeGraph, ugen_nodes::{NodeInput, NodeKind, UGenNode}};

/// How often a time-based envelope samples its shape. `Envelope` linearly
/// interpolates between samples, and these shapes are piecewise linear, so
/// interpolation is exact except at the corners — 0.5 ms is inaudible while
/// keeping the closure off the per-sample path.
///
/// Note this is *not* `lfo()` / `envelope()`, which sample at 2 ms with
/// pseudorandom jitter — far too coarse for a short attack.
const ENV_INTERVAL: f64 = 0.0005;

/// Attack-decay-sustain shape, ignoring release. Returns 0..=1.
fn ads_level(t: f64, attack: f64, decay: f64, sustain: f64) -> f64 {
    let s = sustain.clamp(0.0, 1.0);
    if t < attack {
        // attack <= 0 is handled by the caller ordering: t < 0 is impossible,
        // so a zero attack falls straight through to the decay stage.
        t / attack
    } else if t < attack + decay {
        let x = (t - attack) / decay;
        1.0 + (s - 1.0) * x
    } else {
        s
    }
}

/// Pull input `idx` as a construction-time constant. Parameters like ADSR
/// times are baked into the unit when it is built — they are not ports, so a
/// signal wired here has nothing to connect to.
fn const_param(n: &UGenNode, idx: usize, name: &str) -> Result<f32, String> {
    match n.inputs.get(idx) {
        Some(NodeInput::Const(v)) => Ok(*v as f32),
        Some(NodeInput::Node(_)) => Err(format!("{name} must be a constant, not a signal")),
        None => Err(format!("missing {name}")),
    }
}

/// Realize an IR graph into a runnable network.
///
/// The result is always 0-in / 2-out. Both consumers require it: the engine's
/// program slot is stereo (crossfade asserts matching arity) and the sequencer
/// is stereo (push asserts it). A mono result fans out to both channels.
pub fn realize(graph: &ScreeGraph) -> Result<Net, String> {
    let mut net = Net::new(0, 2);
    let mut ids: Vec<fundsp::net::NodeId> = Vec::with_capacity(graph.nodes.len());

    for n in &graph.nodes {
        let (unit, wired): (Box<dyn AudioUnit>, usize) = match n.kind {
            NodeKind::Add => (Box::new(pass() + pass()), 2),
            NodeKind::ADSR => (
                Box::new(adsr_live(
                    const_param(n, 1, "adsr attack")?,
                    const_param(n, 2, "adsr decay")?,
                    const_param(n, 3, "adsr sustain")?,
                    const_param(n, 4, "adsr release")?,
                )),
                1,
            ),
            NodeKind::Afollow => (
                Box::new(afollow(
                    const_param(n, 1, "afollow attack")?,
                    const_param(n, 2, "afollow release")?,
                )),
                1,
            ),
            NodeKind::Allpass => (Box::new(allpass()), 3),
            NodeKind::Allpole => (Box::new(allpole()), 2),
            NodeKind::Bandpass => (Box::new(bandpass()), 3),
            NodeKind::Bandrez => (Box::new(bandrez()), 3),
            NodeKind::Bell => (Box::new(bell()), 4),
            NodeKind::Biquad => (
                Box::new(biquad(
                    const_param(n, 1, "biquad a1")?,
                    const_param(n, 2, "biquad a2")?,
                    const_param(n, 3, "biquad b0")?,
                    const_param(n, 4, "biquad b1")?,
                    const_param(n, 5, "biquad b2")?,
                )),
                1,
            ),
            NodeKind::Brown => (Box::new(brown()), 0),
            NodeKind::Butterpass => (Box::new(butterpass()), 2),
            NodeKind::Chorus => (
                Box::new(chorus(
                    const_param(n, 1, "chorus seed")? as u64,
                    const_param(n, 2, "chorus separation")?,
                    const_param(n, 3, "chorus variation")?,
                    const_param(n, 4, "chorus mod frequency")?,
                )),
                1,
            ),
            NodeKind::Clip => (Box::new(clip()), 1),
            NodeKind::ClipTo => (
                Box::new(clip_to(
                    const_param(n, 1, "clip_to minimum")?,
                    const_param(n, 2, "clip_to maximum")?,
                )),
                1,
            ),
            NodeKind::Dcblock => (Box::new(dcblock()), 1),
            NodeKind::Declick => (Box::new(declick()), 1),
            NodeKind::Delay => (Box::new(delay(const_param(n, 1, "delay time")?)), 1),
            NodeKind::Div => (Box::new(map(|i: &Frame<f32, U2>| i[0] / i[1])), 2),
            NodeKind::DsfSaw => (Box::new(dsf_saw()), 2),

            // Time-based ADSR for one-shot voices. The release lands exactly on
            // `dur`, so the shape fits inside the sequencer event and needs no
            // tail. Computed as ads * release_mult so degenerate cases (release
            // longer than the note, attack+decay overrunning it, zero-length
            // segments) fall out without special-casing.
            NodeKind::Env => {
                let attack = const_param(n, 0, "env attack")? as f64;
                let decay = const_param(n, 1, "env decay")? as f64;
                let sustain = const_param(n, 2, "env sustain")? as f64;
                let release = const_param(n, 3, "env release")? as f64;
                let dur = const_param(n, 4, "env duration")? as f64;
                let rel_start = (dur - release).max(0.0);
                (
                    Box::new(An(Envelope::new(ENV_INTERVAL, move |t: f64| -> f64 {
                        let level = ads_level(t, attack, decay, sustain);
                        let mult = if t < rel_start {
                            1.0
                        } else if release <= 0.0 {
                            0.0
                        } else {
                            (1.0 - (t - rel_start) / release).clamp(0.0, 1.0)
                        };
                        level * mult
                    }))),
                    0,
                )
            }
            NodeKind::DsfSquare => (Box::new(dsf_square()), 2),
            NodeKind::Fir3 => (Box::new(fir3(const_param(n, 1, "fir3 gain")?)), 1),
            NodeKind::Follow => (Box::new(follow(const_param(n, 1, "follow response time")?)), 1),
            NodeKind::Hammond => (Box::new(hammond()), 1),
            NodeKind::Highpass => (Box::new(highpass()), 3),
            NodeKind::Highpole => (Box::new(highpole()), 2),
            NodeKind::Highshelf => (Box::new(highshelf()), 4),
            NodeKind::Hold => (Box::new(hold(const_param(n, 2, "hold variability")?)), 2),
            NodeKind::Impulse => (Box::new(impulse::<U1>()), 0),
            NodeKind::Limiter => (
                Box::new(limiter(
                    const_param(n, 1, "limiter attack")?,
                    const_param(n, 2, "limiter release")?,
                )),
                1,
            ),
            NodeKind::Lorenz => (Box::new(lorenz()), 1),
            NodeKind::Lowpass => (Box::new(lowpass()), 3),
            NodeKind::Lowpole => (Box::new(lowpole()), 2),
            NodeKind::Lowrez => (Box::new(lowrez()), 3),
            NodeKind::Lowshelf => (Box::new(lowshelf()), 4),
            NodeKind::Mls => (Box::new(mls()), 0),
            NodeKind::MlsBits => (Box::new(mls_bits(const_param(n, 0, "mls_bits bits")? as u64)), 0),
            NodeKind::Moog => (Box::new(moog()), 3),
            NodeKind::Morph => (Box::new(morph()), 4),
            NodeKind::Mul => (Box::new(pass() * pass()), 2),
            NodeKind::Neg => (Box::new(-pass()), 1),
            NodeKind::Noise => (Box::new(noise()), 0),
            NodeKind::Notch => (Box::new(notch()), 3),
            NodeKind::Organ => (Box::new(organ()), 1),
            NodeKind::Peak => (Box::new(peak()), 3),

            // Self-contained percussive shape: rise, fall, silence. Needs no
            // note duration, so it works in a voice or the persistent graph.
            NodeKind::Perc => {
                let attack = const_param(n, 0, "perc attack")? as f64;
                let release = const_param(n, 1, "perc release")? as f64;
                (
                    Box::new(An(Envelope::new(ENV_INTERVAL, move |t: f64| -> f64 {
                        if t < attack {
                            t / attack
                        } else if release <= 0.0 {
                            0.0
                        } else {
                            (1.0 - (t - attack) / release).clamp(0.0, 1.0)
                        }
                    }))),
                    0,
                )
            }
            NodeKind::Pink => (Box::new(pink()), 0),
            NodeKind::Pinkpass => (Box::new(pinkpass()), 1),
            NodeKind::Pluck => (
                Box::new(pluck(
                    const_param(n, 1, "pluck frequency")?,
                    const_param(n, 2, "pluck gain per second")?,
                    const_param(n, 3, "pluck damping")?,
                )),
                1,
            ),
            NodeKind::PolyPulse => (Box::new(poly_pulse()), 2),
            NodeKind::PolySaw => (Box::new(poly_saw()), 1),
            NodeKind::PolySquare => (Box::new(poly_square()), 1),
            NodeKind::Pulse => (Box::new(pulse()), 2),
            NodeKind::Ramp => (Box::new(ramp()), 1),
            NodeKind::Resonator => (Box::new(resonator()), 3),
            NodeKind::Rossler => (Box::new(rossler()), 1),
            NodeKind::Saw => (Box::new(saw()), 1),
            NodeKind::Sin => (Box::new(sine()), 1),
            NodeKind::SoftSaw => (Box::new(soft_saw()), 1),
            NodeKind::Square => (Box::new(square()), 1),
            NodeKind::Sub => (Box::new(pass() - pass()), 2),
            NodeKind::Tap => (
                Box::new(tap(
                    const_param(n, 2, "tap min delay")?,
                    const_param(n, 3, "tap max delay")?,
                )),
                2,
            ),
            NodeKind::Tick => (Box::new(tick()), 1),
            NodeKind::Triangle => (Box::new(triangle()), 1),
        };

        let fid = net.push(unit);

        for (port, input) in n.inputs.iter().take(wired).enumerate() {
            match input {
                NodeInput::Node(id) => net.connect(ids[id.0], 0, fid, port),
                NodeInput::Const(v) => {
                    let c = net.push(Box::new(dc(*v as f32)));
                    net.connect(c, 0, fid, port);
                }
            }
        }

        ids.push(fid);
    }

    // A graph with no output is valid — it just makes silence.
    let out_node = match graph.output {
        Some(out) => ids[out.0],
        None => net.push(Box::new(dc(0.0))),
    };
    net.connect_output(out_node, 0, 0);
    net.connect_output(out_node, 0, 1);

    Ok(net)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scree_graph::ugen_nodes::NodeId;
    use crate::lowerer::lower::lower;
    use crate::parser::parser::parse;

    /// Full pipeline: parse → lower → realize, then render actual samples.
    #[test]
    fn added_signals_produce_audio() {
        let items = parse("sin(220) + sin(330)\n".to_string()).unwrap();
        let graph = lower(&items).unwrap().graph;
        let mut net = realize(&graph).unwrap();

        net.check(); // validates every port is wired
        net.set_sample_rate(44100.0);

        let samples: Vec<f32> = (0..4410).map(|_| net.get_mono()).collect();
        assert!(samples.iter().all(|s| s.is_finite()));
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "expected audible signal, peak was {peak}");
        assert!(peak <= 2.0, "two unit sines can't exceed 2.0, got {peak}");
    }

    /// A `for` loop unrolls to an ordinary graph, so it realizes and renders
    /// like any hand-written sum. Four harmonics of 110 Hz.
    #[test]
    fn for_loop_produces_audio() {
        let items = parse("for i in 1..=4 { sin(i * 110) / 4 }\n".to_string()).unwrap();
        let graph = lower(&items).unwrap().graph;
        let mut net = realize(&graph).unwrap();

        net.check();
        net.set_sample_rate(44100.0);

        let samples: Vec<f32> = (0..4410).map(|_| net.get_mono()).collect();
        assert!(samples.iter().all(|s| s.is_finite()));
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.3, "expected audible signal, peak was {peak}");
        assert!(peak <= 1.0, "four quarter-amplitude sines can't exceed 1.0, got {peak}");
    }

    /// A 2 Hz sine gate retriggers the envelope; output must stay in 0..=1
    /// and actually move.
    #[test]
    fn adsr_gated_by_slow_sine() {
        let graph = ScreeGraph {
            nodes: vec![
                UGenNode {
                    kind: NodeKind::Sin,
                    inputs: vec![NodeInput::Const(2.0)],
                    span: None,
                },
                UGenNode {
                    kind: NodeKind::ADSR,
                    inputs: vec![
                        NodeInput::Node(NodeId(0)), // gate
                        NodeInput::Const(0.01),     // attack
                        NodeInput::Const(0.05),     // decay
                        NodeInput::Const(0.5),      // sustain
                        NodeInput::Const(0.1),      // release
                    ],
                    span: None,
                },
            ],
            output: Some(NodeId(1)),
        };

        let mut net = realize(&graph).unwrap();
        net.check();
        net.set_sample_rate(44100.0);

        let samples: Vec<f32> = (0..44100).map(|_| net.get_mono()).collect();
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(*s));
        let floor = samples.iter().fold(1.0f32, |m, s| m.min(*s));
        assert!(peak > 0.9, "attack should approach 1.0, peak was {peak}");
        assert!(floor >= 0.0, "envelope must not go negative, floor was {floor}");
        assert!((floor - peak).abs() > 0.5, "envelope should move over a full gate cycle");
    }

    /// A signal in a parameter slot is a user error, not a wiring job.
    #[test]
    fn adsr_signal_parameter_is_an_error() {
        let graph = ScreeGraph {
            nodes: vec![
                UGenNode {
                    kind: NodeKind::Sin,
                    inputs: vec![NodeInput::Const(2.0)],
                    span: None,
                },
                UGenNode {
                    kind: NodeKind::ADSR,
                    inputs: vec![
                        NodeInput::Node(NodeId(0)),
                        NodeInput::Node(NodeId(0)), // attack as a signal: invalid
                        NodeInput::Const(0.05),
                        NodeInput::Const(0.5),
                        NodeInput::Const(0.1),
                    ],
                    span: None,
                },
            ],
            output: Some(NodeId(1)),
        };

        let err = match realize(&graph) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for a signal-valued adsr parameter"),
        };
        assert!(err.contains("constant"), "got: {err}");
    }
}


#[cfg(test)]
mod envelope_tests {
    use super::*;
    use crate::scree_graph::ugen_nodes::NodeId;
    use crate::lowerer::lower::{lower, lower_voice};
    use crate::parser::parser::parse;

    const SR: f64 = 44100.0;

    /// Render `src` as a voice with note length `dur` and return its samples.
    fn render_voice(src: &str, dur: f64, secs: f64) -> Vec<f32> {
        let items = parse(src.to_string()).expect("parse failed");
        let lowered = lower_voice(&items, dur).expect("lower failed");
        let mut net = realize(&lowered.graph).expect("realize failed");
        net.check();
        net.set_sample_rate(SR);
        (0..(secs * SR) as usize).map(|_| net.get_mono()).collect()
    }

    fn at(samples: &[f32], t: f64) -> f32 {
        samples[(t * SR) as usize]
    }

    /// perc rises to 1 by the end of its attack and is silent after a + r.
    #[test]
    fn perc_rises_then_falls_to_silence() {
        let s = render_voice("perc(0.05, 0.2)\n", 1.0, 0.5);

        assert!(at(&s, 0.0) < 0.05, "should start near zero, got {}", at(&s, 0.0));
        assert!((at(&s, 0.05) - 1.0).abs() < 0.02, "peak at attack end: {}", at(&s, 0.05));
        assert!((at(&s, 0.15) - 0.5).abs() < 0.03, "midway through release: {}", at(&s, 0.15));
        assert!(at(&s, 0.3) < 0.01, "silent after a + r, got {}", at(&s, 0.3));
        assert!(s.iter().all(|v| *v >= -0.001 && *v <= 1.001), "stayed in 0..=1");
    }

    /// perc needs no note duration, so it works in the persistent graph too.
    #[test]
    fn perc_works_outside_a_voice() {
        let items = parse("sin(220) * perc(0.01, 0.1)\n".to_string()).unwrap();
        let g = lower(&items).unwrap().graph;
        assert!(realize(&g).is_ok());
    }

    /// env holds at its sustain level, then releases to zero exactly at `dur`.
    #[test]
    fn env_sustains_then_releases_at_the_note_end() {
        // attack 0.1, decay 0.1, sustain 0.5, release 0.2, over a 1s note.
        let s = render_voice("env(0.1, 0.1, 0.5, 0.2, dur)\n", 1.0, 1.2);

        assert!((at(&s, 0.1) - 1.0).abs() < 0.02, "peak at attack end: {}", at(&s, 0.1));
        assert!((at(&s, 0.2) - 0.5).abs() < 0.02, "sustain after decay: {}", at(&s, 0.2));
        assert!((at(&s, 0.5) - 0.5).abs() < 0.02, "still sustaining: {}", at(&s, 0.5));
        assert!((at(&s, 0.9) - 0.25).abs() < 0.03, "mid-release: {}", at(&s, 0.9));
        assert!(at(&s, 0.999) < 0.02, "silent by the note end, got {}", at(&s, 0.999));
    }

    /// The note length really comes from `dur`, not a baked constant.
    #[test]
    fn env_tracks_the_note_length() {
        let short = render_voice("env(0.01, 0.01, 0.8, 0.1, dur)\n", 0.3, 0.5);
        let long = render_voice("env(0.01, 0.01, 0.8, 0.1, dur)\n", 0.8, 1.0);

        assert!(at(&short, 0.299) < 0.02, "short note done by 0.3s");
        assert!(at(&long, 0.3) > 0.5, "long note still sustaining at 0.3s");
        assert!(at(&long, 0.799) < 0.02, "long note done by 0.8s");
    }

    /// Degenerate shapes must not panic, go negative, or exceed 1.
    #[test]
    fn env_degenerate_shapes_stay_in_range() {
        for src in [
            "env(0.0, 0.1, 0.5, 0.1, dur)\n",   // no attack
            "env(0.1, 0.0, 0.5, 0.1, dur)\n",   // no decay
            "env(0.1, 0.1, 0.0, 0.1, dur)\n",   // sustain 0
            "env(0.1, 0.1, 0.5, 0.0, dur)\n",   // no release
            "env(0.1, 0.1, 0.5, 5.0, dur)\n",   // release longer than the note
            "env(5.0, 5.0, 0.5, 0.1, dur)\n",   // attack+decay overrun the note
            "env(0.1, 0.1, 2.0, 0.1, dur)\n",   // sustain above 1, clamped
        ] {
            let s = render_voice(src, 0.5, 0.8);
            assert!(
                s.iter().all(|v| v.is_finite() && *v >= -0.001 && *v <= 1.001),
                "{src} left 0..=1"
            );
        }
        let s = render_voice("perc(0.0, 0.0)\n", 0.5, 0.2);
        assert!(s.iter().all(|v| v.is_finite() && *v >= -0.001 && *v <= 1.001));
    }

    /// `dur` exists only inside a voice.
    #[test]
    fn env_outside_a_voice_has_no_duration() {
        let items = parse("env(0.1, 0.1, 0.5, 0.2, dur)\n".to_string()).unwrap();
        // Lowered has no Debug, so unwrap_err() is unavailable here.
        let err = match lower(&items) {
            Err(e) => e,
            Ok(_) => panic!("expected `dur` to be unbound outside a voice"),
        };
        assert!(err.contains("unbound name: dur"), "got: {err}");
    }

    /// Envelope arguments are baked at construction, so a signal is rejected.
    #[test]
    fn envelope_arguments_must_be_constants() {
        let graph = ScreeGraph {
            nodes: vec![
                UGenNode { kind: NodeKind::Sin, inputs: vec![NodeInput::Const(2.0)], span: None },
                UGenNode {
                    kind: NodeKind::Perc,
                    inputs: vec![NodeInput::Node(NodeId(0)), NodeInput::Const(0.1)],
                    span: None,
                },
            ],
            output: Some(NodeId(1)),
        };
        let err = match realize(&graph) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for a signal-valued perc attack"),
        };
        assert!(err.contains("must be a constant"), "got: {err}");
    }

    /// The end-to-end shape: an oscillator shaped by an envelope decays away.
    #[test]
    fn an_enveloped_voice_decays_to_silence() {
        let s = render_voice("sin(220) * perc(0.005, 0.15)\n", 1.0, 0.6);
        let early = s[..(0.1 * SR) as usize].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let late = s[(0.4 * SR) as usize..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(early > 0.5, "should be audible early, peak {early}");
        assert!(late < 0.01, "should have decayed, peak {late}");
    }
}
