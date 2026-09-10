// SPDX-License-Identifier: GPL-3.0

use crate::audio::{Recorder, WHISPER_RATE};
use crate::config::Config;
use crate::{dbus, transcribe, typer};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::Subscription;
use cosmic::prelude::*;
use std::time::Duration;

/// Pause before the first keystroke so the hotkey's modifiers have come up;
/// otherwise Super is still held and the letters fire as shortcuts.
const HOTKEY_GRACE: Duration = Duration::from_millis(300);

/// Panel state. One toggle starts listening, the next stops and transcribes.
/// Toggled by hotkey, the text is typed into the focused window; toggled by
/// clicking the panel, it only goes to stdout, because the click has moved
/// focus to the panel.
#[derive(Default)]
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// Configuration data that persists between application runs.
    config: Config,
    /// Live microphone capture while listening.
    recorder: Option<Recorder>,
    /// True while Whisper (and the typer) run in the background.
    transcribing: bool,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    /// `typing` is true when the toggle came in over D-Bus from the hotkey.
    ToggleListening { typing: bool },
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

        // Bring the virtual keyboard up now so the compositor knows it long
        // before the first dictation. Without /dev/uinput access we still run;
        // transcriptions just stay on stdout.
        if let Err(e) = typer::init() {
            eprintln!("ghostwriter: {e}; typing disabled, text will go to stdout only");
        }

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
            .on_press(Message::ToggleListening { typing: false })
            .into()
    }

    /// Config changes plus the D-Bus endpoint the hotkey talks to.
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch(vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
            dbus::subscription(),
        ])
    }

    /// Handles messages emitted by the application and its widgets.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::UpdateConfig(config) => self.config = config,

            Message::ToggleListening { typing } => {
                if let Some(recorder) = self.recorder.take() {
                    // Second toggle: stop capturing, then transcribe and type off the UI thread.
                    match recorder.stop() {
                        Ok(audio) => {
                            self.transcribing = true;
                            let model = transcribe::models_dir().join(&self.config.model);
                            let language = self.config.language.clone();
                            eprintln!(
                                "ghostwriter: captured {:.1}s, transcribing{}",
                                audio.len() as f32 / WHISPER_RATE as f32,
                                if typing { " and typing" } else { "" }
                            );
                            return cosmic::task::future(async move {
                                let result = tokio::task::spawn_blocking(
                                    move || -> Result<String, String> {
                                        let text =
                                            transcribe::transcribe(&model, &language, &audio)?;
                                        if typing && !text.is_empty() {
                                            std::thread::sleep(HOTKEY_GRACE);
                                            // Trailing space so back-to-back dictations
                                            // don't run together.
                                            typer::type_text(&format!("{text} "))?;
                                        }
                                        Ok(text)
                                    },
                                )
                                .await
                                .unwrap_or_else(|e| Err(format!("dictation task panicked: {e}")));
                                Message::Transcribed(result)
                            });
                        }
                        Err(e) => eprintln!("ghostwriter: {e}"),
                    }
                } else {
                    // First toggle: start listening.
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