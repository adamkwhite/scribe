# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 workspace for local meeting transcription and note generation. Workspace crates live under `crates/`: `scribe-core` owns reusable config, audio recording, transcription, notes, runtime orchestration, logging, and file-opening primitives; `scribe-cli` is the interactive command-line binary; `scribe-tui` is the optional terminal UI behind its `tui` feature; and `scribe` is the Windows tray application. Legacy root `src/` modules have been moved into the workspace crates. Unit tests are colocated in each module under `#[cfg(test)]`, with broader runtime coverage in `crates/scribe-core/tests/`. CI configuration lives in `.github/workflows/ci.yml`. There are no checked-in app assets; recordings, transcripts, and notes are runtime output under the user's documents folder.

## Build, Test, and Development Commands

- `cargo fmt --all -- --check`: verify Rust formatting.
- `cargo clippy --all-targets`: run lints for application and tests.
- `cargo test`: run unit tests.
- `cargo build`: build a debug binary.
- `cargo build --release`: build the optimized user-facing binary.
- `cargo run -p scribe-cli`: run the CLI locally.
- `cargo run -p scribe-tui --features tui`: run the feature-gated terminal UI.
- `cargo run -p scribe`: run the Windows tray app on Windows.

CI runs fmt, clippy, build, tests, and TUI feature checks on Linux and Windows with `RUSTFLAGS=-D warnings`.

## Coding Style & Naming Conventions

Use standard `rustfmt` output and keep warnings clean. Prefer small module-local helpers over broad abstractions. Function and module names use `snake_case`; types use `UpperCamelCase`; constants use `SCREAMING_SNAKE_CASE`. Keep error context actionable with `anyhow::Context`, especially around filesystem, process, audio, and HTTP boundaries. Gate Windows-only code with `#[cfg(target_os = "windows")]`.

## Testing Guidelines

Add focused unit tests next to the code being changed. Current tests cover TOML parsing, audio mixing/session lookup, and OpenRouter request/response handling. Name tests by behavior, for example `parses_minimal_config_with_defaults` or `latest_session_errors_when_no_sessions`. Avoid tests that require real microphones, speakers, whisper.cpp binaries, or live OpenRouter credentials unless explicitly marked and isolated from default `cargo test`.

## Commit & Pull Request Guidelines

Recent history uses short imperative commits such as `Add Rust CI workflow (build, test, clippy, fmt) (#2)`. Keep commit subjects direct and scoped. For PRs, include the user-visible change, tests run, linked issues, and screenshots only when UI/tray behavior changes. Do not include API keys, local config files, recordings, transcripts, or generated notes in commits.
