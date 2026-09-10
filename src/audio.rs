// SPDX-License-Identifier: GPL-3.0

//! Microphone capture.
//!
//! The cpal stream lives on its own thread for the whole recording. `stop()`
//! tears it down and hands back the audio as 16 kHz mono f32, which is the
//! only thing Whisper accepts.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

/// Sample rate Whisper expects.
pub const WHISPER_RATE: u32 = 16_000;

pub struct Recorder {
    stop: mpsc::Sender<()>,
    done: mpsc::Receiver<Vec<f32>>,
}

impl Recorder {
    /// Opens the default input device and starts capturing immediately.
    pub fn start() -> Result<Self, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<Vec<f32>>();
        // Handshake so device errors surface here instead of in a dead thread.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        thread::Builder::new()
            .name("ghostwriter-capture".into())
            .spawn(move || {
                let capture = match Capture::open() {
                    Ok(capture) => capture,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));

                // Blocks until stop() is called or the Recorder is dropped.
                let _ = stop_rx.recv();
                let audio = capture.finish();
                let _ = done_tx.send(audio);
            })
            .map_err(|e| format!("failed to spawn capture thread: {e}"))?;

        ready_rx
            .recv()
            .map_err(|_| String::from("capture thread died before reporting ready"))??;

        Ok(Self {
            stop: stop_tx,
            done: done_rx,
        })
    }

    /// Stops capture and returns the recording as 16 kHz mono samples.
    pub fn stop(self) -> Result<Vec<f32>, String> {
        let _ = self.stop.send(());
        self.done
            .recv()
            .map_err(|_| String::from("capture thread died before returning audio"))
    }
}

/// An open input stream plus the buffer its callback fills.
struct Capture {
    stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    channels: u16,
}

impl Capture {
    fn open() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| String::from("no default input device"))?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("no default input config: {e}"))?;

        let rate = supported.sample_rate();
        let channels = supported.channels();
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sink = Arc::clone(&buffer);

        let stream = match format {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, config, sink)?,
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, config, sink)?,
            cpal::SampleFormat::I32 => build_stream::<i32>(&device, config, sink)?,
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, config, sink)?,
            other => return Err(format!("unsupported input sample format {other:?}")),
        };
        stream
            .play()
            .map_err(|e| format!("failed to start capture: {e}"))?;

        eprintln!("ghostwriter: capturing at {rate} Hz, {channels} channel(s), {format:?}");

        Ok(Self {
            stream,
            buffer,
            rate,
            channels,
        })
    }

    /// Drops the stream and converts what was captured to 16 kHz mono.
    fn finish(self) -> Vec<f32> {
        drop(self.stream);
        let raw = std::mem::take(&mut *self.buffer.lock().unwrap_or_else(|p| p.into_inner()));
        let mono = downmix(&raw, self.channels);
        resample(&mono, self.rate, WHISPER_RATE)
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sink: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                if let Ok(mut buf) = sink.lock() {
                    buf.extend(data.iter().map(|s| f32::from_sample(*s)));
                }
            },
            |err| eprintln!("ghostwriter: capture error: {err}"),
            None,
        )
        .map_err(|e| format!("failed to build input stream: {e}"))
}

/// Averages interleaved channels down to mono.
fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = usize::from(channels.max(1));
    if ch == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Linear-interpolation resampler. Plenty for speech; swap in `rubato` if it ever isn't.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = input.get(idx + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}
