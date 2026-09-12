# GhostWhisper

Local speech-to-text dictation for the COSMIC desktop.

Press a hotkey, talk, press it again — the words get typed into whatever
window has keyboard focus. Terminal, browser, editor, chat box, it doesn't
matter: GhostWhisper is a virtual keyboard, so every app just sees someone
typing. Everything runs on your machine. No audio leaves it.

## How it works

Three pieces inside one COSMIC panel applet:

1. **Capture** — [`cpal`](https://crates.io/crates/cpal) records the default
   microphone while you talk.
2. **Transcribe** — [`whisper-rs`](https://crates.io/crates/whisper-rs)
   (OpenAI's Whisper via whisper.cpp) turns the audio into text. The model is
   loaded once and stays resident, so only the first dictation pays the load
   cost.
3. **Type** — [`evdev`](https://crates.io/crates/evdev) drives a virtual
   keyboard on `/dev/uinput` and types the text into the focused window.

A keyboard shortcut talks to the running applet over D-Bus
(`ghostwhisper --toggle`). That matters: a shortcut doesn't move keyboard
focus, so the text lands where your cursor already is.

The first time it runs it asks — via a notification and the panel popup —
before downloading its model, then fetches it in the background and tells
you when it's ready. Nothing to fetch by hand, and nothing downloaded
without your say-so.

## Requirements

- Pop!_OS 24.04 / COSMIC (it's a panel applet)
- Rust toolchain (edition 2024, so 1.85 or newer) and
  [`just`](https://github.com/casey/just)
- Build dependencies: `cmake`, `clang`, `libclang-dev`, `libasound2-dev`
- Access to `/dev/uinput` (one-time setup below)
- A GPU is optional but makes a big difference — see *Choosing a backend*

## Build and install

```sh
sudo apt install cmake clang libclang-dev libasound2-dev just
just build-release
sudo just install
```

### Choosing a backend

The default build is **CUDA (NVIDIA)**, which needs the CUDA toolkit
(`sudo apt install nvidia-cuda-toolkit` gives you `nvcc`). The backend is a
Cargo feature, so other builds are a flag, not an edit:

| Backend | Build with | Needs | Default model |
|---|---|---|---|
| CUDA (NVIDIA) | `cargo build --release` | CUDA toolkit | `large-v3-turbo` |
| Vulkan (AMD, Intel, NVIDIA) | `cargo build --release --no-default-features --features vulkan` | `libvulkan-dev`, `glslc` | `large-v3-turbo` |
| CPU only | `cargo build --release --no-default-features` | nothing | `base.en` |

GPU builds default to the large model; the CPU build defaults to the small
English one so it stays quick. Either way the applet offers to fetch it.

## One-time setup

### 1. `/dev/uinput` access

The virtual keyboard needs to open `/dev/uinput`, which is root-only by
default. The bundled udev rule opens it to a dedicated `uinput` group:

```sh
sudo groupadd -f uinput
sudo usermod -aG uinput $USER
sudo cp resources/70-ghostwhisper-uinput.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --name-match=uinput
```

Then **log out and back in** so the panel session picks up the group.
`ls -l /dev/uinput` should show group `uinput`. If `/dev/uinput` doesn't
exist at all, the module isn't loaded: `sudo modprobe uinput` and
`echo uinput | sudo tee /etc/modules-load.d/uinput.conf`.

Without this GhostWhisper still runs and says so in the panel popup;
transcriptions only go to the log until it's done.

### 2. Panel, model and hotkey

- Settings → Desktop → Panel → Configure panel applets → add **GhostWhisper**.
- Click the new icon. The popup shows which model the build wants and how
  big it is (1.6 GB for the large one, 150 MB for the small one), with a
  **Download** button. Press it; the popup shows progress and you get a
  notification when it's ready.
- Settings → Keyboard → Shortcuts → Custom Shortcuts → add one running
  `ghostwhisper --toggle`, bound to whatever key you like.

## Using it

Hotkey, talk, hotkey. The panel icon shows the state: microphone when idle,
record dot while listening, spinner while transcribing, download arrow while
the model is being fetched. Each dictation is typed with a trailing space so
the next one doesn't run into it.

Clicking the icon opens the status popup: current state, model and download,
whether typing is set up, the hotkey command to bind, and how long the last
dictation took. Listening itself is only ever toggled by the hotkey —
clicking the panel would move keyboard focus to the panel, and then there'd
be nowhere sensible to type.

If you dictate before the model has finished downloading, you get a
notification with the progress and that clip is dropped, rather than being
typed into some other window minutes later. Clips with no speech in them
(you pressed twice by accident) are dropped silently.

Logs go to the session journal:

```sh
journalctl --user -f | grep ghostwhisper
```

### Config

GhostWhisper uses cosmic-config: one file per setting under
`~/.config/cosmic/io.github.hmrdsmoke.GhostWhisper/v1/`, in RON, so string
values keep their quotes. Changes apply live; if you switch to a model that
isn't on disk, the popup offers to download it.

| File | Default | Meaning |
|---|---|---|
| `model` | `"ggml-large-v3-turbo.bin"` (GPU builds) or `"ggml-base.en.bin"` (CPU) | File name inside the models directory |
| `language` | `"en"` | Whisper language code, or `"auto"` to detect per dictation |

```sh
mkdir -p ~/.config/cosmic/io.github.hmrdsmoke.GhostWhisper/v1
echo '"auto"' > ~/.config/cosmic/io.github.hmrdsmoke.GhostWhisper/v1/language
```

### Models

Models live in `~/.local/share/ghostwhisper/models/` and come from
[ggerganov/whisper.cpp on Hugging Face](https://huggingface.co/ggerganov/whisper.cpp/tree/main).
Any file from there works as a `model` value — the quantized
`ggml-large-v3-turbo-q5_0.bin` is the same model at a third of the VRAM,
useful on 4 GB cards.

| Model | Size | Good for |
|---|---|---|
| `ggml-base.en.bin` | 150 MB | CPU, English, quick and decent |
| `ggml-large-v3-turbo.bin` | 1.6 GB | GPU, any language, best accuracy |

### Command line

| Command | What it does |
|---|---|
| `ghostwhisper` | Runs the applet (standalone it appears as a small window) |
| `ghostwhisper --toggle` | Starts or stops dictation in the running applet |
| `ghostwhisper --type "some words"` | Types the words after 3 seconds — tests the keyboard without Whisper |

## Limitations

- The virtual keyboard uses US-layout keycodes, so it types ASCII. Curly
  quotes and dashes are folded to their ASCII forms; anything else is
  dropped and counted in the log.
- Transcription happens when you stop, not while you speak.
- No spoken commands yet — saying "enter" types the word.
- The large model uses roughly 2 GB of GPU memory while the applet runs.

## Packaging

Vendor dependencies locally with the `vendor` rule and build with the
vendored sources using `build-vendored`. The `rootdir` and `prefix`
variables change installation paths:

```sh
just vendor
just build-vendored
just rootdir=debian/ghostwhisper prefix=/usr install
```

## Translators

[Fluent][fluent] is used for localization. Translation files live in
[`i18n/`](./i18n); copy [`i18n/en`](./i18n/en), rename it to the target
[ISO 639-1 code][iso-codes], and translate each message.

## License

GPL-3.0. See [LICENSE](./LICENSE).

[fluent]: https://projectfluent.org/
[iso-codes]: https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes