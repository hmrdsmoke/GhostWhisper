// SPDX-License-Identifier: GPL-3.0

use crate::audio::{Recorder, WHISPER_RATE};
use crate::config::Config;
use crate::transcribe;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::Subscription;
use cosmic::prelude::*;

/// Panel state. First click starts listening, second click stops and
/// transcribes. For now the text goes to stdout; uinput comes next.
#[derive(Default)]
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// Configuration data that persists between application runs.
    config: Config,
    /// Live microphone capture while listening.
    recorder: Option<Recorder>,
    /// True while Whisper is running in the background.
    transcribing: bool,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    ToggleListening,
    Transcribed(Result<String, String>),
    UpdateConfig(Config),
}

/// Create a COSMIC application from the app model
impl cosmic::Application for AppModel {
    /// The async executor that will be used to run your application's commands.
    type Executor = cosmic::executor::Default;

    /// Data that your application receives to its init method.
    type Flags = ();

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "io.github.hmrdsmoke.GhostWriter";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) => config,
                Err((_errors, config)) => config,
            })
            .unwrap_or_default();

        let app = AppModel {
            core,
            config,
            ..Default::default()
        };

        (app, Task::none())
    }

    /// The panel button. The icon reflects what the applet is doing.
    fn view(&self) -> Element<'_, Self::Message> {
        let icon = if self.recorder.is_some() {
            "media-record-symbolic"
        } else if self.transcribing {
            "content-loading-symbolic"
        } else {
            "audio-input-microphone-symbolic"
        };

        self.core
            .applet
            .icon_button(icon)
            .on_press(Message::ToggleListening)
            .into()
    }

    /// Watch for application configuration changes.
    fn subscription(&self) -> Subscription<Self::Message> {
        self.core()
            .watch_config::<Config>(Self::APP_ID)
            .map(|update| Message::UpdateConfig(update.config))
    }

    /// Handles messages emitted by the application and its widgets.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::UpdateConfig(config) => self.config = config,

            Message::ToggleListening => {
                if let Some(recorder) = self.recorder.take() {
                    // Second press: stop capturing and transcribe off the UI thread.
                    match recorder.stop() {
                        Ok(audio) => {
                            self.transcribing = true;
                            let model = transcribe::models_dir().join(&self.config.model);
                            let language = self.config.language.clone();
                            eprintln!(
                                "ghostwriter: captured {:.1}s, transcribing",
                                audio.len() as f32 / WHISPER_RATE as f32
                            );
                            return cosmic::task::future(async move {
                                let result = tokio::task::spawn_blocking(move || {
                                    transcribe::transcribe(&model, &language, &audio)
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("transcription task panicked: {e}")));
                                Message::Transcribed(result)
                            });
                        }
                        Err(e) => eprintln!("ghostwriter: {e}"),
                    }
                } else {
                    // First press: start listening.
                    match Recorder::start() {
                        Ok(recorder) => {
                            self.recorder = Some(recorder);
                            eprintln!("ghostwriter: listening");
                        }
                        Err(e) => eprintln!("ghostwriter: {e}"),
                    }
                }
            }

            Message::Transcribed(result) => {
                self.transcribing = false;
                match result {
                    Ok(text) => println!("{text}"),
                    Err(e) => eprintln!("ghostwriter: {e}"),
                }
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}