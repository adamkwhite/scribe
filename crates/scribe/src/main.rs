use anyhow::Result;

#[cfg(target_os = "windows")]
mod tray;

#[tokio::main]
async fn main() -> Result<()> {
    let log_path = scribe_core::logging::init_file_logging("scribe")?;
    tracing::info!(log_path = %log_path.display(), "scribe tray launcher starting");

    #[cfg(target_os = "windows")]
    {
        let cfg = scribe_core::config::load_or_create().await?;
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
