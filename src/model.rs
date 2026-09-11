// SPDX-License-Identifier: GPL-3.0

//! Model files: where they live, which one this build defaults to, and
//! fetching them from Hugging Face on first run so the applet works out of
//! the box.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// The model a fresh install downloads. GPU builds get the big one.
pub const DEFAULT_MODEL: &str = if cfg!(any(feature = "cuda", feature = "vulkan")) {
    "ggml-large-v3-turbo.bin"
} else {
    "ggml-base.en.bin"
};

/// Download progress in percent, for the "still downloading" notice.
static PROGRESS: AtomicU8 = AtomicU8::new(0);

/// Serialises downloads so two callers can't fetch the same file at once.
static DOWNLOAD: Mutex<()> = Mutex::new(());

/// Where model files live: `$XDG_DATA_HOME/ghostwriter/models`
/// (normally `~/.local/share/ghostwriter/models`).
pub fn models_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("ghostwriter").join("models")
}

pub fn path_for(name: &str) -> PathBuf {
    models_dir().join(name)
}

/// True when the model file is fully on disk.
pub fn is_ready(name: &str) -> bool {
    path_for(name).is_file()
}

pub fn progress() -> u8 {
    PROGRESS.load(Ordering::Relaxed)
}

/// Makes sure `name` is on disk, downloading it if it isn't. Blocking.
pub fn ensure(name: &str) -> Result<PathBuf, String> {
    let _guard = DOWNLOAD.lock().unwrap_or_else(|p| p.into_inner());
    let path = path_for(name);
    if path.is_file() {
        return Ok(path);
    }
    download(name, &path)?;
    Ok(path)
}

/// Streams the file to `<name>.part` and renames it into place at the end,
/// so a killed download can never leave a truncated model behind.
fn download(name: &str, path: &Path) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| String::from("model path has no parent directory"))?;
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let url = format!("{BASE_URL}/{name}");
    eprintln!("ghostwriter: downloading {url}");
    PROGRESS.store(0, Ordering::Relaxed);

    let mut response = ureq::get(&url)
        .call()
        .map_err(|e| format!("download of {name} failed: {e}"))?;
    let total: Option<u64> = response
        .headers()
        .get(ureq::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());
    // ureq caps bodies at 10 MB unless told otherwise.
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();

    let part = path.with_extension("part");
    let mut file =
        File::create(&part).map_err(|e| format!("cannot create {}: {e}", part.display()))?;

    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    let mut last_logged: u8 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download of {name} interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        done += n as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = (done * 100 / total).min(100) as u8;
            PROGRESS.store(pct, Ordering::Relaxed);
            if pct / 10 > last_logged / 10 {
                eprintln!("ghostwriter: {name} {pct}%");
                last_logged = pct;
            }
        }
    }
    file.sync_all()
        .map_err(|e| format!("cannot flush {}: {e}", part.display()))?;
    drop(file);

    if let Some(total) = total
        && done != total
    {
        let _ = fs::remove_file(&part);
        return Err(format!(
            "download of {name} ended early: {done} of {total} bytes"
        ));
    }

    fs::rename(&part, path)
        .map_err(|e| format!("cannot move model into place at {}: {e}", path.display()))?;
    PROGRESS.store(100, Ordering::Relaxed);
    eprintln!("ghostwriter: model ready at {}", path.display());
    Ok(())
}
