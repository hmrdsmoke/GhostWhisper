// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// GhostWhisper/src/notify.rs
// SPDX-License-Identifier: GPL-3.0-only

//! Desktop notifications, sent from their own thread so nothing waits on D-Bus.

pub fn send(body: &str) {
    let body = body.to_owned();
    std::thread::spawn(move || {
        let result = notify_rust::Notification::new()
            .appname("GhostWhisper")
            .summary("GhostWhisper")
            .body(&body)
            .icon("audio-input-microphone")
            .show();
        if let Err(e) = result {
            eprintln!("ghostwhisper: notification failed: {e}");
        }
    });
}
