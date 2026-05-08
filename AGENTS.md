# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 CLI/tray application for local meeting transcription and note generation. Source lives in `src/`: `main.rs` wires CLI/tray flows, `audio/` records and manages session directories, `config/` loads TOML settings, `transcribe/` shells out to whisper.cpp, `notes/` calls OpenRouter, and `tray.rs` is Windows-only tray UI. Unit tests are colocated in each module under `#[cfg(test)]`. CI configuration lives in `.github/workflows/ci.yml`; `.cargo/config.toml` contains the Windows GNU linker hint. There are no checked-in app assets; recordings, transcripts, and notes are runtime output under the user's documents folder.

## Build, Test, and Development Commands

- `cargo fmt --all -- --check`: verify Rust formatting.
- `cargo clippy --all-targets`: run lints for application and tests.
- `cargo test`: run unit tests.
- `cargo build`: build a debug binary.
- `cargo build --release`: build the optimized user-facing binary.
- `cargo run -- --cli`: run the CLI mode locally. The Windows tray is the default on Windows; non-Windows hosts fall back to CLI mode.

CI runs fmt, clippy, build, and tests on Linux and Windows with `RUSTFLAGS=-D warnings`.

## Coding Style & Naming Conventions

Use standard `rustfmt` output and keep warnings clean. Prefer small module-local helpers over broad abstractions. Function and module names use `snake_case`; types use `UpperCamelCase`; constants use `SCREAMING_SNAKE_CASE`. Keep error context actionable with `anyhow::Context`, especially around filesystem, process, audio, and HTTP boundaries. Gate Windows-only code with `#[cfg(target_os = "windows")]`.

## Testing Guidelines

Add focused unit tests next to the code being changed. Current tests cover TOML parsing, audio mixing/session lookup, and OpenRouter request/response handling. Name tests by behavior, for example `parses_minimal_config_with_defaults` or `latest_session_errors_when_no_sessions`. Avoid tests that require real microphones, speakers, whisper.cpp binaries, or live OpenRouter credentials unless explicitly marked and isolated from default `cargo test`.

## Commit & Pull Request Guidelines

Recent history uses short imperative commits such as `Add Rust CI workflow (build, test, clippy, fmt) (#2)`. Keep commit subjects direct and scoped. For PRs, include the user-visible change, tests run, linked issues, and screenshots only when UI/tray behavior changes. Do not include API keys, local config files, recordings, transcripts, or generated notes in commits.
