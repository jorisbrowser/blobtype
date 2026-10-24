//! Microphone capture with cpal. The stream lives on its own thread because
//! `cpal::Stream` must not cross threads.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use tokio::sync::mpsc::UnboundedSender;

/// One block of mono samples at the device's native rate.
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct CaptureHandle {
    stop_tx: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Names of all input devices, for the microphone picker.
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut names: Vec<String> = host
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

fn pick_device(host: &cpal::Host, wanted: &str) -> Result<cpal::Device, String> {
    let wanted = wanted.trim().to_lowercase();
    if !wanted.is_empty() {
        if let Ok(devices) = host.input_devices() {
            for d in devices {
                if d.name()
                    .map(|n| n.to_lowercase().contains(&wanted))
                    .unwrap_or(false)
                {
                    return Ok(d);
                }
            }
        }
        log::warn!("configured microphone not found, using the default one");
    }
    host.default_input_device()
        .ok_or_else(|| "No microphone found".to_string())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: UnboundedSender<AudioChunk>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let sample_rate = config.sample_rate.0;
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mono: Vec<f32> = data
                .chunks(channels)
                .map(|f| f.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / f.len() as f32)
                .collect();
            let _ = tx.send(AudioChunk {
                samples: mono,
                sample_rate,
            });
        },
        |err| log::error!("audio input stream error: {err}"),
        None,
    )
}

impl CaptureHandle {
    pub fn start(device_name: &str, tx: UnboundedSender<AudioChunk>) -> Result<Self, String> {
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let device_name = device_name.to_string();

        let thread = std::thread::spawn(move || {
            let outcome = (|| -> Result<cpal::Stream, String> {
                let host = cpal::default_host();
                let device = pick_device(&host, &device_name)?;
                let supported = device
                    .default_input_config()
                    .map_err(|e| format!("Microphone unavailable: {e}"))?;
                let format = supported.sample_format();
                let config: cpal::StreamConfig = supported.into();
                let stream = match format {
                    cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, tx),
                    cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, tx),
                    cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, tx),
                    cpal::SampleFormat::I32 => build_stream::<i32>(&device, &config, tx),
                    other => return Err(format!("Unsupported microphone format: {other:?}")),
                }
                .map_err(|e| format!("Cannot open microphone: {e}"))?;
                stream
                    .play()
                    .map_err(|e| format!("Cannot start microphone: {e}"))?;
                Ok(stream)
            })();
            match outcome {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv(); // keep the stream alive until told to stop
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });

        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop_tx,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Microphone did not start in time".into()),
        }
    }

    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
