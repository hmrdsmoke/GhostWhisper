// SPDX-License-Identifier: GPL-3.0

use crate::audio::{Recorder, WHISPER_RATE};
use crate::config::Config;
use crate::{dbus, model, notify, transcribe, typer};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{Limits, Subscription, window::Id};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::{Duration, Instant};

/// Pause before the first keystroke so the hotkey's modifiers have come up;
/// otherwise Super is still held and the letters fire as shortcuts.
const HOTKEY_GRACE: Duration = Duration::from_millis(300);

/// How often the popup refreshes its progress bar while a download runs.
const PROGRESS_TICK: Duration = Duration::from_millis(500);

/// Panel state. The hotkey toggles listening over D-Bus; the panel icon opens
/// the status popup, which is also where the model download is offered.
#[derive(Default)]
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// Configuration data that persists between application runs.
    config: Config,
    /// The status popup, when open.
    popup: Option<Id>,
    /// Live microphone capture while listening.
    recorder: Option<Recorder>,
    /// True while Whisper (and the typer) run in the background.
    transcribing: bool,
    /// True while the model file is being fetched.
    downloading: bool,
    /// Why the last download attempt failed, shown in the popup.
    download_error: Option<String>,
    /// Whether the virtual keyboard came up. False means no /dev/uinput access.
    can_type: bool,
    /// The most recent dictation, for the popup's status line.
    last: Option<Dictation>,
}

/// One finished dictation.
#[derive(Debug, Clone)]
pub struct Dictation {
    pub text: String,
    pub audio_secs: f32,
    pub took_secs: f32,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    /// From the hotkey over D-Bus. Start listening, or stop and type.
    ToggleListening,
    Transcribed(Result<Dictation, String>),
    /// The Download button in the popup.
    StartDownload,
    ModelReady(Result<(), String>),
    /// Progress refresh while downloading.
    Tick,
    UpdateConfig(Config),
}

impl AppModel {
    /// Fetches the configured model in the background. Only ever called from
    /// the popup's Download button — nothing downloads without being asked.
    fn start_download(&mut self) -> Task<cosmic::Action<Message>> {
        if self.downloading {
            return Task::none();
        }
        let name = self.config.model.clone();
        if model::is_ready(&name) {
            return Task::none();
        }

        self.downloading = true;
        self.download_error = None;
        eprintln!("ghostwriter: downloading {name}");
        notify::send(&format!(
            "Downloading {name} ({}). You'll get a notification when it's ready.",
            model::size_hint(&name)
        ));

        cosmic::task::future(async move {
            let result = tokio::task::spawn_blocking(move || model::ensure(&name).map(|_| ()))
                .await
                .unwrap_or_else(|e| Err(format!("download task panicked: {e}")));
            Message::ModelReady(result)
        })
    }

    fn model_missing_notice(&self) -> String {
        if self.downloading {
            format!(
                "Still downloading the speech model ({}%). Try again in a bit.",
                model::progress()
            )
        } else {
            format!(
                "GhostWriter needs the {} speech model ({}). Click the panel icon to download it.",
                self.config.model,
                model::size_hint(&self.config.model)
            )
        }
    }
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
        // the popup explains what's missing.
        let can_type = match typer::init() {
            Ok(()) => true,
            Err(e) => {
                eprintln!("ghostwriter: {e}; typing disabled");
                notify::send("Can't open /dev/uinput, so dictation can't type yet. Click the panel icon for the fix.");
                false
            }
        };

        let app = AppModel {
            core,
            config,
            can_type,
            ..Default::default()
        };

        if !model::is_ready(&app.config.model) {
            notify::send(&app.model_missing_notice());
        }

        (app, Task::none())
    }

    /// The panel button. The icon reflects what the applet is doing.
    fn view(&self) -> Element<'_, Self::Message> {
        let icon = if self.recorder.is_some() {
            "media-record-symbolic"
        } else if self.transcribing {
            "content-loading-symbolic"
        } else if self.downloading {
            "folder-download-symbolic"
        } else {
            "audio-input-microphone-symbolic"
        };

        self.core
            .applet
            .icon_button(icon)
            .on_press(Message::TogglePopup)
            .into()
    }

    /// The status popup: what the applet is doing, the model and its
    /// download, whether typing works, and how to bind the hotkey.
    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let spacing = cosmic::theme::spacing();
        let model_name = self.config.model.as_str();

        let status = if self.recorder.is_some() {
            "Listening…"
        } else if self.transcribing {
            "Transcribing…"
        } else if self.downloading {
            "Downloading the speech model…"
        } else {
            "Idle"
        };

        let mut rows: Vec<Element<'_, Message>> = vec![
            widget::text::title4("GhostWriter").into(),
            widget::text::body(format!("Status: {status}")).into(),
            widget::divider::horizontal::default().into(),
            widget::text::caption_heading("Model").into(),
            widget::text::body(model_name).into(),
        ];

        if self.downloading {
            let pct = model::progress();
            rows.push(widget::progress_bar::determinate_linear(f32::from(pct) / 100.0).into());
            rows.push(widget::text::caption(format!("Downloading… {pct}%")).into());
        } else if model::is_ready(model_name) {
            rows.push(widget::text::caption("Ready").into());
        } else {
            rows.push(
                widget::text::caption(format!(
                    "Not downloaded yet ({}). It comes from Hugging Face and is kept in {}.",
                    model::size_hint(model_name),
                    model::models_dir().display()
                ))
                .into(),
            );
            if let Some(err) = &self.download_error {
                rows.push(widget::text::caption(format!("Last attempt failed: {err}")).into());
            }
            rows.push(
                widget::button::suggested("Download")
                    .on_press(Message::StartDownload)
                    .into(),
            );
        }

        rows.push(widget::divider::horizontal::default().into());
        rows.push(widget::text::caption_heading("Typing").into());
        rows.push(
            widget::text::caption(if self.can_type {
                "Ready"
            } else {
                "Can't open /dev/uinput. Run the one-time udev setup from the README, \
                 then log out and back in."
            })
            .into(),
        );

        rows.push(widget::divider::horizontal::default().into());
        rows.push(widget::text::caption_heading("Hotkey").into());
        rows.push(
            widget::text::caption(
                "Bind a custom keyboard shortcut to:  ghostwriter --toggle",
            )
            .into(),
        );

        rows.push(widget::divider::horizontal::default().into());
        rows.push(widget::text::caption_heading("Last dictation").into());
        rows.push(
            widget::text::caption(match &self.last {
                Some(d) => format!(
                    "{:.1} s of speech transcribed in {:.2} s",
                    d.audio_secs, d.took_secs
                ),
                None => String::from("Nothing yet"),
            })
            .into(),
        );

        let content = widget::Column::with_children(rows)
            .spacing(f32::from(spacing.space_xs))
            .padding([f32::from(spacing.space_xs), f32::from(spacing.space_s)]);

        self.core.applet.popup_container(content).into()
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// Config changes, the D-Bus endpoint the hotkey talks to, and a progress
    /// tick while a download runs.
    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
            dbus::subscription(),
        ];
        if self.downloading {
            subscriptions.push(cosmic::iced::time::every(PROGRESS_TICK).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    /// Handles messages emitted by the application and its widgets.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::TogglePopup => {
                return if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(372.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(1080.0);
                    get_popup(popup_settings)
                };
            }

            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }

            Message::Tick => {}

            Message::StartDownload => return self.start_download(),

            Message::ModelReady(result) => {
                self.downloading = false;
                match result {
                    Ok(()) => {
                        self.download_error = None;
                        notify::send("Speech model ready. Hotkey, talk, hotkey.");
                    }
                    Err(e) => {
                        eprintln!("ghostwriter: {e}");
                        notify::send(&format!("Couldn't download the speech model: {e}"));
                        self.download_error = Some(e);
                    }
                }
            }

            Message::UpdateConfig(config) => {
                let model_changed = config.model != self.config.model;
                self.config = config;
                if model_changed {
                    self.download_error = None;
                    if !model::is_ready(&self.config.model) {
                        notify::send(&self.model_missing_notice());
                    }
                }
            }

            Message::ToggleListening => {
                if let Some(recorder) = self.recorder.take() {
                    // Second toggle: stop capturing first so the mic is released either way.
                    let audio = match recorder.stop() {
                        Ok(audio) => audio,
                        Err(e) => {
                            eprintln!("ghostwriter: {e}");
                            return Task::none();
                        }
                    };

                    // No model: say so and drop the clip. Typing it minutes later
                    // into whatever has focus by then would be worse.
                    if !model::is_ready(&self.config.model) {
                        let notice = self.model_missing_notice();
                        eprintln!("ghostwriter: {notice}");
                        notify::send(&notice);
                        return Task::none();
                    }

                    self.transcribing = true;
                    let model_path = model::path_for(&self.config.model);
                    let language = self.config.language.clone();
                    let can_type = self.can_type;
                    let audio_secs = audio.len() as f32 / WHISPER_RATE as f32;
                    eprintln!("ghostwriter: captured {audio_secs:.1}s of speech, transcribing");
                    return cosmic::task::future(async move {
                        let result = tokio::task::spawn_blocking(
                            move || -> Result<Dictation, String> {
                                let started = Instant::now();
                                let text =
                                    transcribe::transcribe(&model_path, &language, &audio)?;
                                let took_secs = started.elapsed().as_secs_f32();
                                if can_type && !text.is_empty() {
                                    std::thread::sleep(HOTKEY_GRACE);
                                    // Trailing space so back-to-back dictations
                                    // don't run together.
                                    typer::type_text(&format!("{text} "))?;
                                }
                                Ok(Dictation {
                                    text,
                                    audio_secs,
                                    took_secs,
                                })
                            },
                        )
                        .await
                        .unwrap_or_else(|e| Err(format!("dictation task panicked: {e}")));
                        Message::Transcribed(result)
                    });
                } else {
                    // First toggle: start listening.
                    match Recorder::start() {
                        Ok(recorder) => {
                            self.recorder = Some(recorder);
                            eprintln!("ghostwriter: listening");
                        }
                        Err(e) => {
                            eprintln!("ghostwriter: {e}");
                            notify::send(&format!("Can't record: {e}"));
                        }
                    }
                }
            }

            Message::Transcribed(result) => {
                self.transcribing = false;
                match result {
                    Ok(dictation) => {
                        println!("{}", dictation.text);
                        self.last = Some(dictation);
                    }
                    Err(e) => {
                        eprintln!("ghostwriter: {e}");
                        notify::send(&format!("Dictation failed: {e}"));
                    }
                }
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}