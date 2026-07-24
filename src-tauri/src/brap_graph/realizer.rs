use fundsp::prelude64::*;

use crate::brap_graph::{graph::BrapGraph, ugen_nodes::{NodeInput, NodeKind, UGenNode}};

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

pub fn realize(graph: &BrapGraph) -> Result<Net, String> {
    let mut net = Net::new(0, 1);
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

    if let Some(out) = graph.output {
        net.pipe_output(ids[out.0]);
    }

    Ok(net)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brap_graph::ugen_nodes::NodeId;
    use crate::lowerer::lower::lower;
    use crate::parser::parser::parse;

    /// Full pipeline: parse → lower → realize, then render actual samples.
    #[test]
    fn added_signals_produce_audio() {
        let items = parse("sin(220) + sin(330)\n".to_string()).unwrap();
        let graph = lower(&items).unwrap();
        let mut net = realize(&graph).unwrap();

        net.check(); // validates every port is wired
        net.set_sample_rate(44100.0);

        let samples: Vec<f32> = (0..4410).map(|_| net.get_mono()).collect();
        assert!(samples.iter().all(|s| s.is_finite()));
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "expected audible signal, peak was {peak}");
        assert!(peak <= 2.0, "two unit sines can't exceed 2.0, got {peak}");
    }

    /// A 2 Hz sine gate retriggers the envelope; output must stay in 0..=1
    /// and actually move.
    #[test]
    fn adsr_gated_by_slow_sine() {
        let graph = BrapGraph {
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
        let graph = BrapGraph {
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

