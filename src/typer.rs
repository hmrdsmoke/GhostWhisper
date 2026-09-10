// SPDX-License-Identifier: GPL-3.0

//! Types text into whatever window has keyboard focus, by way of a virtual
//! keyboard on /dev/uinput. The compositor sees ordinary key presses, so this
//! works in any app on any display server — terminals included.
//!
//! Needs read/write access to /dev/uinput; see resources/70-ghostwriter-uinput.rules.
//! Keycodes are US layout. Anything the US keymap can't produce is dropped.

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, InputEvent, KeyCode, KeyEvent};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::Duration;

/// Gap between keystrokes so nothing gets dropped or reordered downstream.
const KEY_DELAY: Duration = Duration::from_millis(5);

static KEYBOARD: OnceLock<Mutex<VirtualDevice>> = OnceLock::new();

/// Creates the virtual keyboard. Call once at startup so the compositor has
/// long since picked up the device by the time we type. Later calls are no-ops.
pub fn init() -> Result<(), String> {
    if KEYBOARD.get().is_some() {
        return Ok(());
    }

    let mut keys = AttributeSet::<KeyCode>::new();
    for code in LETTERS.iter().chain(DIGITS.iter()).chain(OTHER_KEYS.iter()) {
        keys.insert(*code);
    }

    let device = VirtualDevice::builder()
        .map_err(|e| {
            format!("cannot open /dev/uinput: {e} (see resources/70-ghostwriter-uinput.rules)")
        })?
        .name("GhostWriter virtual keyboard")
        .with_keys(&keys)
        .map_err(|e| format!("cannot register keys on virtual keyboard: {e}"))?
        .build()
        .map_err(|e| format!("cannot create virtual keyboard: {e}"))?;

    // Give udev and the compositor a moment to notice the new device.
    sleep(Duration::from_millis(300));
    let _ = KEYBOARD.set(Mutex::new(device));
    Ok(())
}

/// Types `text` as key presses. Blocking; call it off the UI thread.
pub fn type_text(text: &str) -> Result<(), String> {
    let keyboard = KEYBOARD
        .get()
        .ok_or_else(|| String::from("virtual keyboard not initialised"))?;
    let mut device = keyboard.lock().unwrap_or_else(|p| p.into_inner());

    let mut skipped = 0usize;
    for c in normalise(text).chars() {
        match keystroke(c) {
            Some((key, shift)) => press(&mut device, key, shift)?,
            None => skipped += 1,
        }
    }
    if skipped > 0 {
        eprintln!("ghostwriter: skipped {skipped} character(s) the US keymap can't type");
    }
    Ok(())
}

fn press(device: &mut VirtualDevice, key: KeyCode, shift: bool) -> Result<(), String> {
    if shift {
        emit(device, KeyCode::KEY_LEFTSHIFT, 1)?;
    }
    emit(device, key, 1)?;
    emit(device, key, 0)?;
    if shift {
        emit(device, KeyCode::KEY_LEFTSHIFT, 0)?;
    }
    sleep(KEY_DELAY);
    Ok(())
}

/// One key event plus the SYN_REPORT that `emit` appends for us.
fn emit(device: &mut VirtualDevice, key: KeyCode, value: i32) -> Result<(), String> {
    let event: InputEvent = *KeyEvent::new(key, value);
    device
        .emit(&[event])
        .map_err(|e| format!("uinput write failed: {e}"))
}

/// Whisper occasionally emits typographic punctuation; fold it to ASCII.
fn normalise(text: &str) -> String {
    text.replace('…', "...")
        .chars()
        .map(|c| match c {
            '‘' | '’' | '‚' => '\'',
            '“' | '”' | '„' => '"',
            '–' | '—' | '−' => '-',
            '\u{a0}' => ' ',
            other => other,
        })
        .collect()
}

/// US-layout keycode and whether Shift is needed.
fn keystroke(c: char) -> Option<(KeyCode, bool)> {
    use KeyCode as K;
    let stroke = match c {
        'a'..='z' => (LETTERS[usize::from(c as u8 - b'a')], false),
        'A'..='Z' => (LETTERS[usize::from(c as u8 - b'A')], true),
        '0'..='9' => (DIGITS[usize::from(c as u8 - b'0')], false),
        ' ' => (K::KEY_SPACE, false),
        '\n' => (K::KEY_ENTER, false),
        '\t' => (K::KEY_TAB, false),
        '-' => (K::KEY_MINUS, false),
        '_' => (K::KEY_MINUS, true),
        '=' => (K::KEY_EQUAL, false),
        '+' => (K::KEY_EQUAL, true),
        '[' => (K::KEY_LEFTBRACE, false),
        '{' => (K::KEY_LEFTBRACE, true),
        ']' => (K::KEY_RIGHTBRACE, false),
        '}' => (K::KEY_RIGHTBRACE, true),
        '\\' => (K::KEY_BACKSLASH, false),
        '|' => (K::KEY_BACKSLASH, true),
        ';' => (K::KEY_SEMICOLON, false),
        ':' => (K::KEY_SEMICOLON, true),
        '\'' => (K::KEY_APOSTROPHE, false),
        '"' => (K::KEY_APOSTROPHE, true),
        ',' => (K::KEY_COMMA, false),
        '<' => (K::KEY_COMMA, true),
        '.' => (K::KEY_DOT, false),
        '>' => (K::KEY_DOT, true),
        '/' => (K::KEY_SLASH, false),
        '?' => (K::KEY_SLASH, true),
        '`' => (K::KEY_GRAVE, false),
        '~' => (K::KEY_GRAVE, true),
        '!' => (K::KEY_1, true),
        '@' => (K::KEY_2, true),
        '#' => (K::KEY_3, true),
        '$' => (K::KEY_4, true),
        '%' => (K::KEY_5, true),
        '^' => (K::KEY_6, true),
        '&' => (K::KEY_7, true),
        '*' => (K::KEY_8, true),
        '(' => (K::KEY_9, true),
        ')' => (K::KEY_0, true),
        _ => return None,
    };
    Some(stroke)
}

const LETTERS: [KeyCode; 26] = [
    KeyCode::KEY_A,
    KeyCode::KEY_B,
    KeyCode::KEY_C,
    KeyCode::KEY_D,
    KeyCode::KEY_E,
    KeyCode::KEY_F,
    KeyCode::KEY_G,
    KeyCode::KEY_H,
    KeyCode::KEY_I,
    KeyCode::KEY_J,
    KeyCode::KEY_K,
    KeyCode::KEY_L,
    KeyCode::KEY_M,
    KeyCode::KEY_N,
    KeyCode::KEY_O,
    KeyCode::KEY_P,
    KeyCode::KEY_Q,
    KeyCode::KEY_R,
    KeyCode::KEY_S,
    KeyCode::KEY_T,
    KeyCode::KEY_U,
    KeyCode::KEY_V,
    KeyCode::KEY_W,
    KeyCode::KEY_X,
    KeyCode::KEY_Y,
    KeyCode::KEY_Z,
];

const DIGITS: [KeyCode; 10] = [
    KeyCode::KEY_0,
    KeyCode::KEY_1,
    KeyCode::KEY_2,
    KeyCode::KEY_3,
    KeyCode::KEY_4,
    KeyCode::KEY_5,
    KeyCode::KEY_6,
    KeyCode::KEY_7,
    KeyCode::KEY_8,
    KeyCode::KEY_9,
];

const OTHER_KEYS: [KeyCode; 15] = [
    KeyCode::KEY_SPACE,
    KeyCode::KEY_ENTER,
    KeyCode::KEY_TAB,
    KeyCode::KEY_LEFTSHIFT,
    KeyCode::KEY_MINUS,
    KeyCode::KEY_EQUAL,
    KeyCode::KEY_LEFTBRACE,
    KeyCode::KEY_RIGHTBRACE,
    KeyCode::KEY_BACKSLASH,
    KeyCode::KEY_SEMICOLON,
    KeyCode::KEY_APOSTROPHE,
    KeyCode::KEY_COMMA,
    KeyCode::KEY_DOT,
    KeyCode::KEY_SLASH,
    KeyCode::KEY_GRAVE,
];
