use anyhow::{Context, Result};
use std::future::Future;
use std::path::{Path, PathBuf};

use super::model_download_event::ModelDownloadEvent;
use super::paths::config_dir;
use super::settings::Config;

const MANAGED_MODEL_FILENAME: &str = "ggml-base.en.bin";

const MANAGED_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

pub fn managed_model_filename() -> &'static str {
    MANAGED_MODEL_FILENAME
}

pub fn managed_model_path_in_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(managed_model_filename())
}

pub fn resolve_managed_whisper_model_config(mut config: Config, config_dir: &Path) -> Config {
    if config.whisper_model == managed_model_filename() {
        config.whisper_model = managed_model_path_in_dir(config_dir)
            .to_string_lossy()
            .into_owned();
    }
    config
}

pub(super) fn is_managed_model_path(model_path: &str, config_dir: &Path) -> bool {
    model_path == managed_model_path_in_dir(config_dir).to_string_lossy()
}

pub async fn ensure_managed_whisper_model() -> Result<PathBuf> {
    ensure_managed_whisper_model_with_events(|event| {
        if matches!(event, ModelDownloadEvent::Downloading(_)) {
            println!("{}", event.message());
        }
    })
    .await
}

pub async fn ensure_managed_whisper_model_with_events<F>(on_event: F) -> Result<PathBuf>
where
    F: FnMut(ModelDownloadEvent),
{
    let config_dir = config_dir()?;
    ensure_managed_whisper_model_in_dir_with_events(
        &config_dir,
        download_managed_whisper_model,
        on_event,
    )
    .await
}

#[cfg(test)]
async fn ensure_managed_whisper_model_in_dir<F, Fut>(
    config_dir: &Path,
    downloader: F,
) -> Result<PathBuf>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    ensure_managed_whisper_model_in_dir_with_events(config_dir, downloader, |_| {}).await
}

async fn ensure_managed_whisper_model_in_dir_with_events<F, Fut, R>(
    config_dir: &Path,
    downloader: F,
    mut on_event: R,
) -> Result<PathBuf>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = Result<()>>,
    R: FnMut(ModelDownloadEvent),
{
    let model_path = managed_model_path_in_dir(config_dir);
    if model_path.exists() {
        on_event(ModelDownloadEvent::AlreadyPresent(model_path.clone()));
        tracing::info!(model_path = %model_path.display(), "managed Whisper model already present");
        return Ok(model_path);
    }

    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("Failed to create {}", config_dir.display()))?;

    let download_path = model_path.with_extension("bin.download");
    let _ = std::fs::remove_file(&download_path);
    on_event(ModelDownloadEvent::Downloading(model_path.clone()));
    tracing::info!(
        model_path = %model_path.display(),
        download_path = %download_path.display(),
        "managed Whisper model download starting"
    );

    if let Err(error) = downloader(download_path.clone()).await {
        let _ = std::fs::remove_file(&download_path);
        tracing::warn!(
            error = %error,
            model_path = %model_path.display(),
            "managed Whisper model download failed"
        );
        return Err(error);
    }

    std::fs::rename(&download_path, &model_path).with_context(|| {
        format!(
            "Failed to move downloaded model from {} to {}",
            download_path.display(),
            model_path.display()
        )
    })?;

    on_event(ModelDownloadEvent::Downloaded(model_path.clone()));
    tracing::info!(model_path = %model_path.display(), "managed Whisper model downloaded");
    Ok(model_path)
}

async fn download_managed_whisper_model(download_path: PathBuf) -> Result<()> {
    let response = reqwest::get(MANAGED_MODEL_URL)
        .await
        .context("Failed to start Whisper model download")?
        .error_for_status()
        .context("Whisper model download returned an error status")?;
    let bytes = response
        .bytes()
        .await
        .context("Failed to read Whisper model download")?;
    tokio::fs::write(&download_path, bytes)
        .await
        .with_context(|| format!("Failed to write {}", download_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn managed_model_path_uses_config_dir() {
        let temp = tempfile::tempdir().unwrap();

        let path = managed_model_path_in_dir(temp.path());

        assert_eq!(path, temp.path().join("ggml-base.en.bin"));
    }

    #[tokio::test]
    async fn cache_hit_skips_downloader() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());
        std::fs::write(&model_path, b"cached model").unwrap();
        let called = Rc::new(Cell::new(false));
        let called_for_download = called.clone();

        let path = ensure_managed_whisper_model_in_dir(temp.path(), move |_| {
            called_for_download.set(true);
            async { Ok(()) }
        })
        .await
        .unwrap();

        assert_eq!(path, model_path);
        assert!(!called.get());
    }

    #[tokio::test]
    async fn cache_miss_downloads_to_temp_file_and_renames() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());
        let expected_download_path = model_path.with_extension("bin.download");

        let path = ensure_managed_whisper_model_in_dir(temp.path(), |download_path| async move {
            assert_eq!(download_path, expected_download_path);
            std::fs::write(download_path, b"downloaded model")?;
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(path, model_path);
        assert_eq!(std::fs::read(&model_path).unwrap(), b"downloaded model");
        assert!(!model_path.with_extension("bin.download").exists());
    }

    #[tokio::test]
    async fn failed_download_leaves_no_final_model_file() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());

        let error = ensure_managed_whisper_model_in_dir(temp.path(), |download_path| async move {
            std::fs::write(download_path, b"partial model")?;
            anyhow::bail!("network failed")
        })
        .await
        .unwrap_err();

        assert!(error.to_string().contains("network failed"));
        assert!(!model_path.exists());
        assert!(!model_path.with_extension("bin.download").exists());
    }

    #[test]
    fn default_model_filename_resolves_to_managed_config_path() {
        let temp = tempfile::tempdir().unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper".into()),
            whisper_model: "ggml-base.en.bin".into(),
            openrouter_api_key: "key".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: None,
        };

        let cfg = resolve_managed_whisper_model_config(cfg, temp.path());

        assert_eq!(
            cfg.whisper_model,
            temp.path().join("ggml-base.en.bin").to_string_lossy()
        );
    }

    #[test]
    fn custom_model_path_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper".into()),
            whisper_model: "/models/custom.bin".into(),
            openrouter_api_key: "key".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: None,
        };

        let cfg = resolve_managed_whisper_model_config(cfg, temp.path());

        assert_eq!(cfg.whisper_model, "/models/custom.bin");
    }
}
