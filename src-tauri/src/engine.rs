use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use fundsp::net::{Net, NodeId};
use fundsp::prelude::*;

pub struct AudioEngine {
    pub net: Net,     // control-side handle; lives in tauri state
    pub slot: NodeId, // the one node programs get swapped into
}

pub fn start() -> Result<AudioEngine, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;

    let mut net = Net::new(0, 2);
    let slot = net.push(Box::new(dc(0.0)));      // silence until first eval
    net.connect_output(slot, 0, 0);              // mono slot fans out
    net.connect_output(slot, 0, 1);              //   to both channels
    net.set_sample_rate(config.sample_rate() as f64);
    let backend = net.backend();

    std::thread::spawn(move || {
        let result = match config.sample_format() {
            cpal::SampleFormat::F32 => run_stream::<f32>(&device, &config.into(), backend),
            cpal::SampleFormat::I16 => run_stream::<i16>(&device, &config.into(), backend),
            cpal::SampleFormat::U16 => run_stream::<u16>(&device, &config.into(), backend),
            other => Err(format!("unsupported sample format: {other}")),
        };
        if let Err(e) = result {
            eprintln!("audio thread failed: {e}");
        }
    });

    Ok(AudioEngine { net, slot })
}

fn run_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut backend: impl AudioUnit + 'static,
) -> Result<(), String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let stream = device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _| {
                for frame in data.chunks_mut(channels) {
                    let (l, r) = backend.get_stereo();
                    frame[0] = T::from_sample(l);
                    if channels > 1 {
                        frame[1] = T::from_sample(r);
                    }
                }
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
    engine.net.crossfade(slot, Fade::Smooth, 0.2, Box::new(dc(0.0)));
    engine.net.commit();
}