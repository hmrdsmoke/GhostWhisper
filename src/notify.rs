// SPDX-License-Identifier: GPL-3.0

//! Desktop notifications, sent from their own thread so nothing waits on D-Bus.

pub fn send(body: &str) {
    let body = body.to_owned();
    std::thread::spawn(move || {
        let result = notify_rust::Notification::new()
            .appname("GhostWriter")
            .summary("GhostWriter")
            .body(&body)
            .icon("audio-input-microphone")
            .show();
        if let Err(e) = result {
            eprintln!("ghostwriter: notification failed: {e}");
        }
    });
}
