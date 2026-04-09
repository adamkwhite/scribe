# scribe

Local meeting transcription and note generation. Records system audio, transcribes with whisper.cpp, generates structured notes via LLM.

## Prerequisites

- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) — download a release or build from source
- A whisper model file (e.g., `ggml-base.en.bin`) — download from the whisper.cpp repo
- [OpenRouter](https://openrouter.ai/) API key

## Setup

1. Build: `cargo build --release`
2. Run once to generate config: `scribe.exe`
3. Edit config at `%APPDATA%/scribe/config.toml`:
   ```toml
   whisper_bin = "C:/path/to/whisper-cli.exe"
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
