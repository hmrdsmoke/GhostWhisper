// SPDX-License-Identifier: GPL-3.0-only

//! Whisper inference.
//!
//! The model is loaded on first use and then stays resident for the life of
//! the applet, so only the first press pays the load cost. Changing the model
//! in config swaps it on the next press.

use crate::audio::WHISPER_RATE;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

struct Loaded {
    path: PathBuf,
    ctx: WhisperContext,
}

static MODEL: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();

/// Segments Whisper itself rates this likely to be non-speech are discarded.
const NO_SPEECH_THRESHOLD: f32 = 0.6;

/// Runs Whisper over 16 kHz mono audio and returns the text. Empty audio
/// (nothing above the noise floor) returns an empty string without running
/// the model — pure silence is where Whisper hallucinates "Thank you."
///
/// Blocking — call it from `spawn_blocking`, never on the UI thread.
pub fn transcribe(model_path: &Path, language: &str, audio: &[f32]) -> Result<String, String> {
    if audio.is_empty() {
        eprintln!("ghostwhisper: no speech in the clip, skipping");
        return Ok(String::new());
    }

    // whisper.cpp refuses anything under a second; pad short clips with silence.
    let min_len = WHISPER_RATE as usize * 6 / 5;
    let mut audio = audio.to_vec();
    if audio.len() < min_len {
        audio.resize(min_len, 0.0);
    }

    let slot = MODEL.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());

    let needs_load = guard.as_ref().is_none_or(|loaded| loaded.path != model_path);
    if needs_load {
        eprintln!("ghostwhisper: loading model {}", model_path.display());
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|e| format!("failed to load model {}: {e}", model_path.display()))?;
        *guard = Some(Loaded {
            path: model_path.to_path_buf(),
            ctx,
        });
    }
    let ctx = &guard.as_ref().expect("model loaded above").ctx;

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("failed to create whisper state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(if language == "auto" { None } else { Some(language) });
    params.set_n_threads(threads());
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    // Drop non-speech tokens like [BLANK_AUDIO] and [MUSIC].
    params.set_suppress_nst(true);
    // Don't feed each 30 s window the previous one's text; that's how one
    // hallucinated sentence turns into twenty.
    params.set_no_context(true);

    let started = std::time::Instant::now();
    state
        .full(params, &audio)
        .map_err(|e| format!("whisper inference failed: {e}"))?;

    let mut text = String::new();
    for segment in state.as_iter() {
        let piece = segment
            .to_str_lossy()
            .map_err(|e| format!("bad segment text: {e}"))?;
        // Whisper's own estimate that nobody spoke here. Its captions for
        // silence and noise ("Thank you.") come with a high one.
        let no_speech = segment.no_speech_probability();
        if no_speech > NO_SPEECH_THRESHOLD {
            eprintln!(
                "ghostwhisper: dropped segment {:?} (no-speech probability {no_speech:.2})",
                piece.trim()
            );
            continue;
        }
        // Belt and braces: a marker that gets past suppress_nst is never dictation.
        if is_marker(piece.trim()) {
            continue;
        }
        text.push_str(&piece);
    }

    eprintln!(
        "ghostwhisper: transcribed {:.1}s of audio in {:.2}s",
        audio.len() as f32 / WHISPER_RATE as f32,
        started.elapsed().as_secs_f32()
    );

    Ok(collapse_repeats(text.trim()))
}

/// `[BLANK_AUDIO]`, `(silence)`, `*music*` — Whisper's ways of saying "nothing said".
fn is_marker(segment: &str) -> bool {
    let wrapped = |open: char, close: char| {
        segment.starts_with(open) && segment.ends_with(close) && segment.len() > 1
    };
    wrapped('[', ']') || wrapped('(', ')') || wrapped('*', '*')
}

/// Keeps at most two consecutive copies of the same sentence. Saying
/// "No. No." on purpose survives; a hallucinated "Thank you." × 20 doesn't.
fn collapse_repeats(text: &str) -> String {
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let ends_sentence = matches!(c, '.' | '!' | '?')
            && chars.peek().is_none_or(|next| next.is_whitespace());
        if ends_sentence {
            sentences.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current);
    }

    let mut out = String::new();
    let mut previous: Option<String> = None;
    let mut run = 0usize;
    for sentence in sentences {
        let key = sentence.trim().to_lowercase();
        if previous.as_deref() == Some(key.as_str()) {
            run += 1;
        } else {
            previous = Some(key);
            run = 1;
        }
        if run <= 2 {
            out.push_str(&sentence);
        }
    }
    out.trim().to_string()
}

fn threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16) as i32
}