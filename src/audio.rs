// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// GhostWhisper/src/audio.rs
// SPDX-License-Identifier: GPL-3.0-only

//! Microphone capture.
//!
//! The cpal stream lives on its own thread for the whole recording. `stop()`
//! tears it down and hands back the audio as 16 kHz mono f32 with the
//! silence trimmed, which is what Whisper wants — and silence is what makes
//! Whisper invent things, so the less of it the better.

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
            .name("ghostwhisper-capture".into())
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

    /// Stops capture and returns the recording as 16 kHz mono samples with
    /// leading, trailing and long internal silences trimmed. Empty if nobody
    /// said anything.
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

        eprintln!("ghostwhisper: capturing at {rate} Hz, {channels} channel(s), {format:?}");

        Ok(Self {
            stream,
            buffer,
            rate,
            channels,
        })
    }

    /// Drops the stream and converts what was captured to trimmed 16 kHz mono.
    fn finish(self) -> Vec<f32> {
        drop(self.stream);
        let raw = std::mem::take(&mut *self.buffer.lock().unwrap_or_else(|p| p.into_inner()));
        let mono = downmix(&raw, self.channels);
        let resampled = resample(&mono, self.rate, WHISPER_RATE);
        let trimmed = trim_silence(&resampled);
        eprintln!(
            "ghostwhisper: {:.1}s recorded, {:.1}s after trimming silence",
            resampled.len() as f32 / WHISPER_RATE as f32,
            trimmed.len() as f32 / WHISPER_RATE as f32
        );
        trimmed
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
            |err| eprintln!("ghostwhisper: capture error: {err}"),
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

/// Analysis window for the silence detector.
const FRAME: usize = WHISPER_RATE as usize / 50; // 20 ms
/// Audio kept on either side of detected speech.
const PAD_FRAMES: usize = 15; // 300 ms
/// Longest pause kept inside a recording.
const MAX_GAP_FRAMES: usize = 50; // 1 s
/// Loud audio shorter than this is a click, not speech. The hotkey press
/// itself shows up in the recording as one or two loud frames.
const MIN_SPEECH_FRAMES: usize = 15; // 300 ms

/// Cuts leading and trailing silence and shortens long pauses. The threshold
/// adapts to the recording's own noise floor, so a hissy mic doesn't get
/// everything cut and a quiet one doesn't keep everything.
///
/// Returns an empty buffer when nothing in the clip rises above the floor
/// for long enough to be speech.
fn trim_silence(audio: &[f32]) -> Vec<f32> {
    let frames: Vec<&[f32]> = audio.chunks(FRAME).collect();
    if frames.is_empty() {
        return Vec::new();
    }

    let rms: Vec<f32> = frames
        .iter()
        .map(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())
        .collect();

    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[sorted.len() / 5]; // 20th percentile ≈ the room
    let threshold = (floor * 4.0).clamp(0.005, 0.03);

    let loud: Vec<bool> = rms.iter().map(|&r| r > threshold).collect();
    if loud.iter().filter(|&&l| l).count() < MIN_SPEECH_FRAMES {
        return Vec::new();
    }
    let Some(first) = loud.iter().position(|&l| l) else {
        return Vec::new();
    };
    let last = loud.iter().rposition(|&l| l).unwrap_or(first);

    let start = first.saturating_sub(PAD_FRAMES);
    let end = (last + PAD_FRAMES + 1).min(frames.len());

    let mut out = Vec::with_capacity((end - start) * FRAME);
    let mut quiet_run = 0usize;
    for i in start..end {
        if loud[i] {
            quiet_run = 0;
        } else {
            quiet_run += 1;
        }
        if quiet_run <= MAX_GAP_FRAMES {
            out.extend_from_slice(frames[i]);
        }
    }
    out
}