use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelDownloadEvent {
    AlreadyPresent(PathBuf),
    Downloading(PathBuf),
    Downloaded(PathBuf),
}

impl ModelDownloadEvent {
    pub fn message(&self) -> String {
        match self {
            Self::AlreadyPresent(path) => {
                format!("Whisper model already present: {}", path.display())
            }
            Self::Downloading(path) => {
                format!("Downloading Whisper model to {}...", path.display())
            }
            Self::Downloaded(path) => format!("Whisper model downloaded to {}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_download_events_format_user_visible_messages() {
        let path = PathBuf::from("/tmp/scribe/ggml-base.en.bin");

        assert_eq!(
            ModelDownloadEvent::AlreadyPresent(path.clone()).message(),
            format!("Whisper model already present: {}", path.display())
        );
        assert_eq!(
            ModelDownloadEvent::Downloading(path.clone()).message(),
            format!("Downloading Whisper model to {}...", path.display())
        );
        assert_eq!(
            ModelDownloadEvent::Downloaded(path.clone()).message(),
            format!("Whisper model downloaded to {}", path.display())
        );
    }
}
