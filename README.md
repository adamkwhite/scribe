# scribe

Local meeting transcription and note generation. Records system audio, transcribes with Whisper, generates structured notes via LLM.

## Prerequisites

- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) — download a release or build from source, unless building with `--features embedded-whisper`
- A whisper model file (e.g., `ggml-base.en.bin`) — download from the whisper.cpp repo
- [OpenRouter](https://openrouter.ai/) API key

## Setup

### External whisper.cpp CLI

1. Build: `cargo build --release`
2. Run once to generate config: `scribe.exe`
3. Edit config at `%APPDATA%/scribe/config.toml`:
   ```toml
   whisper_bin = "C:/path/to/whisper-cli.exe"
   whisper_model = "C:/path/to/ggml-base.en.bin"
   openrouter_api_key = "sk-or-..."
   ```

### Embedded whisper.cpp

Build with the optional embedded backend:

```sh
cargo build --release --features embedded-whisper
```

This uses the `whisper-rs` bindings, which build `whisper.cpp` during Cargo's native build. The built binary no longer needs `whisper-cli` on the host, but the build machine needs native build tools such as CMake and a C/C++ toolchain, and the app still needs a `whisper_model` path:

```toml
whisper_model = "C:/path/to/ggml-base.en.bin"
openrouter_api_key = "sk-or-..."
```

## Usage

```
scribe
> r        # start recording system audio
> s        # stop recording, transcribe, generate notes
> q        # quit
```

Notes are saved as timestamped Markdown files in `~/Documents/scribe/`.
