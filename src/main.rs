// SPDX-License-Identifier: GPL-3.0

mod app;
mod audio;
mod config;
mod dbus;
mod i18n;
mod transcribe;
mod typer;

fn main() -> cosmic::iced::Result {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        // `ghostwriter --toggle`: poke the running applet. This is what the hotkey runs.
        Some("--toggle") => exit_with(dbus::send_toggle()),

        // `ghostwriter --type some words`: exercise the virtual keyboard without Whisper.
        Some("--type") => {
            let text = args.collect::<Vec<_>>().join(" ");
            exit_with(typer::init().and_then(|()| {
                eprintln!("ghostwriter: typing in 3 seconds, focus a text field now");
                std::thread::sleep(std::time::Duration::from_secs(3));
                typer::type_text(&text)
            }))
        }

        Some(other) => eprintln!("ghostwriter: ignoring unknown argument {other}"),
        None => {}
    }

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Starts the applet's event loop with `()` as the application's flags.
    cosmic::applet::run::<app::AppModel>(())
}

fn exit_with(result: Result<(), String>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("ghostwriter: {e}");
            std::process::exit(1)
        }
    }
}