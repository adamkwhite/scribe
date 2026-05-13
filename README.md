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
