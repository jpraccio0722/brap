use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use fundsp::net::{Net, NodeId};
use fundsp::prelude::*;

use crate::scheduler::clock::Clock;

pub struct AudioEngine {
    pub net: Net,     // control-side handle; lives in tauri state
    pub slot: NodeId, // the one node programs get swapped into
    pub clock: Clock,
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

    net.connect(slot, 0, mixer, 0);
    net.connect(slot, 1, mixer, 1);
    net.connect(seq_node, 0, mixer, 2);
    net.connect(seq_node, 1, mixer, 3);

    net.connect_output(mixer, 0, 0);
    net.connect_output(mixer, 1, 1);

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

    Ok((AudioEngine { net, slot, clock }, seq))
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