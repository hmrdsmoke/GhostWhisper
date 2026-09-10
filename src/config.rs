// SPDX-License-Identifier: GPL-3.0

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};

#[derive(Debug, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    /// File name of the ggml model inside `transcribe::models_dir()`.
    pub model: String,
    /// Spoken language passed to Whisper. "en", "de", ... or "auto" to detect.
    pub language: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: String::from("ggml-base.en.bin"),
            language: String::from("en"),
        }
    }
}