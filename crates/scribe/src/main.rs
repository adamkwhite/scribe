use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let cfg = scribe::config::load_or_create().await?;
        return scribe::tray::run(cfg).await;
    }

    #[cfg(not(target_os = "windows"))]
    {
        println!("Scribe system tray is only available on Windows.");
        println!("Use `scribe-cli` for CLI mode or `scribe-tui` for terminal UI mode.");
        Ok(())
    }
}
