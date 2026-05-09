use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

pub fn open_folder(path: &Path) -> Result<()> {
    let (program, args) = folder_open_command(path);
    Command::new(program)
        .args(args)
        .spawn()
        .with_context(|| format!("Failed to open {}", path.display()))?;
    Ok(())
}

fn folder_open_command(path: &Path) -> (&'static str, Vec<String>) {
    let path = path.to_string_lossy().into_owned();

    #[cfg(target_os = "macos")]
    {
        ("open", vec![path])
    }

    #[cfg(target_os = "windows")]
    {
        ("explorer.exe", vec![path])
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        ("xdg-open", vec![path])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn folder_open_command_uses_platform_file_manager() {
        let path = PathBuf::from("/tmp/scribe-session");
        let (program, args) = folder_open_command(&path);

        #[cfg(target_os = "macos")]
        assert_eq!(program, "open");
        #[cfg(target_os = "windows")]
        assert_eq!(program, "explorer.exe");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(program, "xdg-open");

        assert_eq!(args, vec![path.to_string_lossy().into_owned()]);
    }
}
