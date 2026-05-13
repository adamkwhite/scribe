#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::Result;

#[cfg(target_os = "windows")]
mod tray;

#[tokio::main]
async fn main() -> Result<()> {
    let log_path = scribe_core::logging::init_file_logging("scribe")?;
    tracing::info!(log_path = %log_path.display(), "scribe tray launcher starting");

    #[cfg(target_os = "windows")]
    {
        let (cfg, origin) = scribe_core::config::load_or_create()?;
        if let scribe_core::config::ConfigOrigin::JustCreated(path) = &origin {
            // No console in release builds, so we can't print this. The
            // library already logged the create event; surface it in the
            // log with explicit setup language so a user tailing the log
            // file can find it.
            tracing::warn!(
                config_path = %path.display(),
                "default config created — edit it with your whisper model path and OpenRouter API key before recording"
            );
        }
        return tray::run(cfg).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        tracing::info!("scribe tray launcher unavailable on this platform");
        println!("Scribe system tray is only available on Windows.");
        println!("Use `scribe-cli` for CLI mode or `scribe-tui` for terminal UI mode.");
        Ok(())
    }
}
