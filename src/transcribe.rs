// SPDX-License-Identifier: GPL-3.0

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

/// Where model files live: `$XDG_DATA_HOME/ghostwriter/models`
/// (normally `~/.local/share/ghostwriter/models`).
pub fn models_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("ghostwriter").join("models")
}

/// Runs Whisper over 16 kHz mono audio and returns the text.
///
/// Blocking — call it from `spawn_blocking`, never on the UI thread.
pub fn transcribe(model_path: &Path, language: &str, audio: &[f32]) -> Result<String, String> {
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
        eprintln!("ghostwriter: loading model {}", model_path.display());
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

    let started = std::time::Instant::now();
    state
        .full(params, &audio)
        .map_err(|e| format!("whisper inference failed: {e}"))?;

    let mut text = String::new();
    for segment in state.as_iter() {
        let piece = segment
            .to_str_lossy()
            .map_err(|e| format!("bad segment text: {e}"))?;
        // Belt and braces: a marker that gets past suppress_nst is never dictation.
        if is_marker(piece.trim()) {
            continue;
        }
        text.push_str(&piece);
    }

    eprintln!(
        "ghostwriter: transcribed {:.1}s of audio in {:.2}s",
        audio.len() as f32 / WHISPER_RATE as f32,
        started.elapsed().as_secs_f32()
    );

    Ok(text.trim().to_string())
}

/// `[BLANK_AUDIO]`, `(silence)`, `*music*` — Whisper's ways of saying "nothing said".
fn is_marker(segment: &str) -> bool {
    let wrapped = |open: char, close: char| {
        segment.starts_with(open) && segment.ends_with(close) && segment.len() > 1
    };
    wrapped('[', ']') || wrapped('(', ')') || wrapped('*', '*')
}

fn threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16) as i32
}