# scribe

Local meeting transcription and note generation. Records system audio, transcribes with Whisper, generates structured notes via LLM.

## Prerequisites

- Native build tools such as CMake and a C/C++ toolchain for the embedded Whisper backend
- A whisper model file (e.g., `ggml-base.en.bin`) — provide a path in config or let Scribe download the managed base English model
- Optional: `whisper-cli` from whisper.cpp when you want to override the embedded backend
- [OpenRouter](https://openrouter.ai/) API key

### Linux contributor packages

Install these packages before running `cargo build`, `cargo clippy`, or `cargo test` on Linux:

- `libasound2-dev` for `cpal`/`alsa-sys`
- `clang` and `libclang-dev` for `whisper-rs-sys`/`bindgen`
- `cmake` for the embedded `whisper.cpp` native build

On Ubuntu/Debian:

```sh
sudo apt-get update
sudo apt-get install -y libasound2-dev clang libclang-dev cmake
```

## Setup

### Default embedded Whisper backend

1. Build: `cargo build --release --workspace`
2. Run once to generate config: `scribe.exe` on Windows or `scribe-cli` on other platforms
3. Edit the generated config file:
   - Windows: `%APPDATA%/scribe/config.toml`
   - Linux: `~/.config/scribe/config.toml`
   ```toml
   whisper_model = "/path/to/ggml-base.en.bin"
   openrouter_api_key = "sk-or-..."
   ```

The default build uses the `whisper-rs` bindings, which build `whisper.cpp`
during Cargo's native build. The built binary does not need `whisper-cli` on the
host, but it still needs a model path:

```toml
whisper_model = "/path/to/ggml-base.en.bin"
openrouter_api_key = "sk-or-..."
```

If the config uses the managed model path, use the TUI setup screen's Download
model action to fetch `ggml-base.en.bin` under the same directory as the
application config. Later uses of that action verify the existing managed file's
SHA-256 before reusing it and redownload the file if the checksum does not
match. Startup only creates or loads config; it does not begin a model download.

### Choosing a Whisper model

The default `ggml-base.en.bin` (142 MB) is what scribe auto-downloads via the
TUI's Download model action. It's a fine baseline for clear single-speaker
audio. For meeting recordings with multiple speakers, accents, or technical
jargon, the whisper.cpp community generally recommends a larger model.

| Model | Size | Notes |
|---|---|---|
| `ggml-tiny.en.bin` | ~75 MB | Smallest English-only model. Generally too inaccurate for meeting transcription. |
| `ggml-base.en.bin` | ~142 MB | Default. Good baseline for clear single-speaker audio. |
| `ggml-small.en.bin` | ~466 MB | Widely recommended for meeting-grade English transcription. Real-time on a modern laptop. |
| `ggml-medium.en.bin` | ~1.5 GB | Another notable quality jump per the whisper community. Slow on CPU; faster on Apple Silicon or with a GPU. |
| `ggml-large-v3-turbo.bin` | ~1.6 GB | Best accuracy/speed ratio; multilingual only — no `.en` variant. |

Quality comparisons above reflect whisper.cpp community consensus rather than
exhaustive in-house testing.

To swap models:

1. Download a `ggml-*.bin` from
   [whisper.cpp on Hugging Face](https://huggingface.co/ggerganov/whisper.cpp/tree/main).
2. Place it next to your scribe config:
   - Windows: `%APPDATA%\scribe\`
   - Linux: `~/.config/scribe/`
   - macOS: `~/Library/Application Support/scribe/`
3. Update the path in `config.toml`:
   ```toml
   whisper_model = "C:/Users/you/AppData/Roaming/scribe/ggml-small.en.bin"
   ```

#### Why models aren't bundled in releases

The smallest usable English model is ~140 MB, and quality models reach
500 MB–3 GB. Bundling would either bloat every download or force a single
choice on every user. Scribe ships small platform archives (~6–9 MB) and lets
you pick.

### External whisper.cpp CLI override

Set `whisper_bin` when you want Scribe to use an installed whisper.cpp
executable instead of embedded Whisper:

```toml
whisper_bin = "C:/path/to/whisper-cli.exe"
whisper_model = "C:/path/to/ggml-base.en.bin"
openrouter_api_key = "sk-or-..."
```

## Usage

Scribe now ships separate binaries for each interface:

- `scribe` starts the Windows system tray app. On macOS/Linux it prints guidance
  and exits because the tray UI is Windows-only. The tray app depends on
  `scribe-core` for shared recording, transcription, notes, config, and folder
  opening behavior.
- `scribe-cli` starts the interactive terminal CLI backed by `scribe-core`.
- `scribe-tui` starts the Ratatui terminal UI backed by `scribe-core` when built
  with the `tui` feature.

### CLI

```
cargo run -p scribe-cli
> r        # start recording system audio
> s        # stop recording, transcribe, generate notes
> q        # quit
```

Notes are saved as timestamped Markdown files in `~/Documents/scribe/`.

### Terminal UI

Build and run the TUI with its package-level feature:

```sh
cargo run -p scribe-tui --features tui
```

The TUI provides first-run setup, session browsing, recording, processing
progress, and folder-opening actions. Without the `tui` feature, the
`scribe-tui` binary target is not built.

Shared implementation code lives in the `scribe-core` library crate. Common run
commands:

```sh
cargo run -p scribe-cli
cargo run -p scribe-tui --features tui
```
