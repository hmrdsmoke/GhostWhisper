# GhostWriter

Local speech-to-text dictation for the COSMIC desktop.

Press a hotkey, talk, press it again — the words get typed into whatever
window has keyboard focus. Terminal, browser, editor, chat box, it doesn't
matter: GhostWriter is a virtual keyboard, so every app just sees someone
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
(`ghostwriter --toggle`). That matters: a shortcut doesn't move keyboard
focus, so the text lands where your cursor already is.

On first run the applet downloads its model by itself and tells you when
it's ready. Nothing to fetch by hand.

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
English one so it stays quick. Either way the applet fetches it for you.

## One-time setup

### 1. `/dev/uinput` access

The virtual keyboard needs to open `/dev/uinput`, which is root-only by
default. The bundled udev rule opens it to a dedicated `uinput` group:

```sh
sudo groupadd -f uinput
sudo usermod -aG uinput $USER
sudo cp resources/70-ghostwriter-uinput.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --name-match=uinput
```

Then **log out and back in** so the panel session picks up the group.
`ls -l /dev/uinput` should show group `uinput`. If `/dev/uinput` doesn't
exist at all, the module isn't loaded: `sudo modprobe uinput` and
`echo uinput | sudo tee /etc/modules-load.d/uinput.conf`.

Without this GhostWriter still runs and will tell you so; transcriptions
only go to the log until it's done.

### 2. Panel and hotkey

- Settings → Desktop → Panel → Configure panel applets → add **GhostWriter**.
  It starts downloading the model the first time it runs; you'll get a
  notification when it's ready (about 1.6 GB for the large model, 150 MB
  for the small one).
- Settings → Keyboard → Shortcuts → Custom Shortcuts → add one running
  `ghostwriter --toggle`, bound to whatever key you like.

## Using it

Hotkey, talk, hotkey. The panel icon shows the state: microphone when idle,
record dot while listening, spinner while transcribing, download arrow while
the model is being fetched. Each dictation is typed with a trailing space so
the next one doesn't run into it.

If you dictate before the model has finished downloading, you get a
notification with the progress and that clip is dropped, rather than being
typed into some other window minutes later.

Clicking the panel icon also starts and stops listening, but the result is
only logged, never typed — clicking the panel moves keyboard focus to the
panel, so there'd be nowhere sensible to type. Use the hotkey.

Logs go to the session journal:

```sh
journalctl --user -f | grep ghostwriter
```

### Config

GhostWriter uses cosmic-config: one file per setting under
`~/.config/cosmic/io.github.hmrdsmoke.GhostWriter/v1/`, in RON, so string
values keep their quotes. Changes apply live; changing the model triggers a
download if the new one isn't on disk.

| File | Default | Meaning |
|---|---|---|
| `model` | `"ggml-large-v3-turbo.bin"` (GPU builds) or `"ggml-base.en.bin"` (CPU) | File name inside the models directory |
| `language` | `"en"` | Whisper language code, or `"auto"` to detect per dictation |

```sh
mkdir -p ~/.config/cosmic/io.github.hmrdsmoke.GhostWriter/v1
echo '"auto"' > ~/.config/cosmic/io.github.hmrdsmoke.GhostWriter/v1/language
```

### Models

Models live in `~/.local/share/ghostwriter/models/` and come from
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
| `ghostwriter` | Runs the applet (standalone it appears as a small window) |
| `ghostwriter --toggle` | Starts or stops dictation in the running applet |
| `ghostwriter --type "some words"` | Types the words after 3 seconds — tests the keyboard without Whisper |

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
just rootdir=debian/ghostwriter prefix=/usr install
```

## Translators

[Fluent][fluent] is used for localization. Translation files live in
[`i18n/`](./i18n); copy [`i18n/en`](./i18n/en), rename it to the target
[ISO 639-1 code][iso-codes], and translate each message.

## License

GPL-3.0. See [LICENSE](./LICENSE).

[fluent]: https://projectfluent.org/
[iso-codes]: https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes