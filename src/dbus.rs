// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// GhostWhisper/src/dbus.rs
// SPDX-License-Identifier: GPL-3.0-only

//! Session-bus endpoint so a keyboard shortcut can reach the running applet.
//!
//! Server side: the applet owns the well-known name `NAME` and exposes one
//! method, `Toggle`. Client side: `ghostwhisper --toggle` calls it and exits.
//! Bind a COSMIC custom shortcut to that command and you have the dictation
//! hotkey — and unlike clicking the panel, a shortcut doesn't move focus, so
//! the text lands where the cursor is.

use crate::app::{AppModel, Message};
use cosmic::iced::{Subscription, futures};
use futures::SinkExt;
use futures::channel::mpsc::Sender;

/// Same string as `AppModel::APP_ID`.
pub const NAME: &str = <AppModel as cosmic::Application>::APP_ID;
pub const PATH: &str = "/io/github/hmrdsmoke/GhostWhisper";

struct Server {
    tx: Sender<Message>,
}

// The interface name must be a literal here; keep it equal to NAME.
#[zbus::interface(name = "io.github.hmrdsmoke.GhostWhisper")]
impl Server {
    /// Start listening, or stop and type what was said.
    async fn toggle(&self) {
        let _ = self.tx.clone().send(Message::ToggleListening).await;
    }
}

/// Long-lived subscription that serves `Toggle` for the life of the applet.
pub fn subscription() -> Subscription<Message> {
    Subscription::run(|| {
        cosmic::iced::stream::channel(4, |tx: Sender<Message>| async move {
            let builder = zbus::connection::Builder::session()
                .and_then(|b| b.name(NAME))
                .and_then(|b| b.serve_at(PATH, Server { tx }));

            match builder {
                Ok(builder) => match builder.build().await {
                    Ok(_connection) => {
                        eprintln!("ghostwhisper: hotkey endpoint up on D-Bus as {NAME}");
                        // Keep the connection alive forever.
                        futures::future::pending::<()>().await;
                    }
                    Err(e) => eprintln!(
                        "ghostwhisper: could not own D-Bus name {NAME} ({e}); hotkey disabled"
                    ),
                },
                Err(e) => eprintln!("ghostwhisper: D-Bus setup failed ({e}); hotkey disabled"),
            }

            futures::future::pending::<()>().await;
        })
    })
}

/// Asks the running applet to toggle. Used by `ghostwhisper --toggle`.
pub fn send_toggle() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;

    runtime.block_on(async {
        let connection = zbus::Connection::session()
            .await
            .map_err(|e| format!("session bus: {e}"))?;
        connection
            .call_method(Some(NAME), PATH, Some(NAME), "Toggle", &())
            .await
            .map_err(|e| format!("is the applet running? ({e})"))?;
        Ok(())
    })
}