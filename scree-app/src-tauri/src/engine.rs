use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use fundsp::net::{Net, NodeId};
use fundsp::prelude::*;

use crate::scheduler::clock::Clock;

/// Unity: the master control only ever attenuates what a program asked for.
pub const DEFAULT_MASTER_VOLUME: f32 = 1.0;

/// How long the master gain takes to reach a new setting. Long enough that
/// dragging a fader is smooth rather than a stairway of clicks, short enough
/// that it still feels immediate.
const MASTER_GLIDE_SECS: f64 = 0.02;

pub struct AudioEngine {
    pub net: Net,     // control-side handle; lives in tauri state
    pub slot: NodeId, // the one node programs get swapped into
    pub clock: Clock,
    /// Master output gain, read per sample by the graph's last node. Setting
    /// it is all a volume control has to do — nothing is rebuilt.
    pub master: Shared,
}

/// A stereo insert that scales what passes through it by a shared control.
///
/// The control is glided rather than read raw: a fader jumping straight from
/// one gain to the next steps the waveform, and a step is a click.
fn master_gain(volume: &Shared) -> Box<dyn AudioUnit> {
    Box::new(
        (pass() | pass())
            * ((var(volume) >> follow(MASTER_GLIDE_SECS))
                | (var(volume) >> follow(MASTER_GLIDE_SECS))),
    )
}

/// Start audio. The `Sequencer` is returned separately because the scheduler
/// thread owns it outright — nothing else touches it, so it needs no mutex.
pub fn start() -> Result<(AudioEngine, Sequencer), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;

    let mut net = Net::new(0, 2);

    let slot = net.push(Box::new(dc((0.0, 0.0))));      // silence until first eval
    
    let mut seq = Sequencer::new(0, 2, ReplayMode::None);
    let seq_node = net.push(Box::new(seq.backend()));

    let mixer = net.push(Box::new((pass() | pass()) + (pass() | pass())));

    // Everything that can make a sound is mixed by now, so this is the one
    // place a master gain belongs: after the graph *and* the sequencer.
    let master = Shared::new(DEFAULT_MASTER_VOLUME);
    let master_node = net.push(master_gain(&master));

    net.connect(slot, 0, mixer, 0);
    net.connect(slot, 1, mixer, 1);
    net.connect(seq_node, 0, mixer, 2);
    net.connect(seq_node, 1, mixer, 3);

    net.connect(mixer, 0, master_node, 0);
    net.connect(mixer, 1, master_node, 1);

    net.connect_output(master_node, 0, 0);
    net.connect_output(master_node, 1, 1);

    net.set_sample_rate(config.sample_rate() as f64);
    let backend = net.backend();

    let clock = Clock::new(config.sample_rate() as f64);
    let audio_clock = clock.clone();

    std::thread::spawn(move || {
        let result = match config.sample_format() {
            cpal::SampleFormat::F32 => run_stream::<f32>(&device, &config.into(), backend, audio_clock),
            cpal::SampleFormat::I16 => run_stream::<i16>(&device, &config.into(), backend, audio_clock),
            cpal::SampleFormat::U16 => run_stream::<u16>(&device, &config.into(), backend, audio_clock),
            other => Err(format!("unsupported sample format: {other}")),
        };
        if let Err(e) = result {
            eprintln!("audio thread failed: {e}");
        }
    });

    Ok((AudioEngine { net, slot, clock, master }, seq))
}

fn run_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut backend: impl AudioUnit + 'static,
    clock: Clock,
) -> Result<(), String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let stream = device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _| {
                let rendered = (data.len() / channels) as u64;
                for frame in data.chunks_mut(channels) {
                    let (l, r) = backend.get_stereo();
                    frame[0] = T::from_sample(l);
                    if channels > 1 {
                        frame[1] = T::from_sample(r);
                    }
                }
                clock.advance(rendered);
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    std::thread::park(); // keep the !Send stream alive on this thread, forever
    Ok(())
}

pub fn swap_program(engine: &mut AudioEngine, program: Net) {
    let slot = engine.slot;
    engine.net.crossfade(slot, Fade::Smooth, 0.2, Box::new(program));
    engine.net.commit();
}

pub fn stop(engine: &mut AudioEngine) {
    let slot = engine.slot;
    // Must be 2-out: crossfade asserts the replacement matches the slot.
    engine.net.crossfade(slot, Fade::Smooth, 0.2, Box::new(dc((0.0, 0.0))));
    engine.net.commit();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the insert for a while at a fixed input, and report the last
    /// output — past the glide, so this measures the setting rather than the
    /// ramp towards it.
    fn settled(unit: &mut Box<dyn AudioUnit>, input: f32) -> (f32, f32) {
        let mut out = [0.0f32; 2];
        for _ in 0..4410 {
            // 100ms at 44.1k, well past the glide
            unit.tick(&[input, input], &mut out);
        }
        (out[0], out[1])
    }

    /// `Net::connect` asserts on arity, so an insert of the wrong shape does
    /// not sound wrong — it panics the app on startup.
    #[test]
    fn the_master_is_a_stereo_insert() {
        let unit = master_gain(&Shared::new(1.0));
        assert_eq!(unit.inputs(), 2);
        assert_eq!(unit.outputs(), 2);
    }

    #[test]
    fn master_volume_scales_the_output() {
        let volume = Shared::new(DEFAULT_MASTER_VOLUME);
        let mut unit = master_gain(&volume);
        unit.set_sample_rate(44100.0);

        // The default must be unity: a program should be heard as written.
        let (l, r) = settled(&mut unit, 0.5);
        assert!((l - 0.5).abs() < 1e-3, "left was {l}");
        assert!((r - 0.5).abs() < 1e-3, "right was {r}");

        volume.set(0.25);
        let (l, _) = settled(&mut unit, 0.5);
        assert!((l - 0.125).abs() < 1e-3, "left was {l}");

        volume.set(0.0);
        let (l, r) = settled(&mut unit, 0.5);
        assert!(l.abs() < 1e-4 && r.abs() < 1e-4, "zero should silence, got ({l}, {r})");
    }

    /// The point of the glide: a jump in the setting must not become a jump in
    /// the signal.
    #[test]
    fn a_volume_jump_is_glided_not_stepped() {
        let volume = Shared::new(1.0);
        let mut unit = master_gain(&volume);
        unit.set_sample_rate(44100.0);
        let _ = settled(&mut unit, 1.0);

        volume.set(0.0);
        let mut out = [0.0f32; 2];
        unit.tick(&[1.0, 1.0], &mut out);
        assert!(out[0] > 0.9, "one sample later it should barely have moved, got {}", out[0]);
    }
}
