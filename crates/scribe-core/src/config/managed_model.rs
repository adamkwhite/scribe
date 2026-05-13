use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use super::model_download_event::ModelDownloadEvent;
use super::paths::config_dir;
use super::settings::Config;

const MANAGED_MODEL_FILENAME: &str = "ggml-base.en.bin";

const MANAGED_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

const MANAGED_MODEL_SHA256: &str =
    "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";

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
        MANAGED_MODEL_SHA256,
        download_managed_whisper_model,
        on_event,
    )
    .await
}

async fn ensure_managed_whisper_model_in_dir_with_events<F, Fut, R>(
    config_dir: &Path,
    expected_sha256: &str,
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
        let actual_sha256 = file_sha256(&model_path)?;
        if actual_sha256 == expected_sha256 {
            on_event(ModelDownloadEvent::AlreadyPresent(model_path.clone()));
            tracing::info!(
                model_path = %model_path.display(),
                sha256 = actual_sha256,
                "managed Whisper model already present"
            );
            return Ok(model_path);
        }

        tracing::warn!(
            model_path = %model_path.display(),
            expected_sha256,
            actual_sha256,
            "managed Whisper model checksum mismatch; redownloading"
        );
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

    if let Err(error) = verify_file_sha256(&download_path, expected_sha256) {
        let _ = std::fs::remove_file(&download_path);
        tracing::warn!(
            error = %error,
            model_path = %model_path.display(),
            "managed Whisper model download failed checksum verification"
        );
        return Err(error);
    }

    if model_path.exists() {
        std::fs::remove_file(&model_path)
            .with_context(|| format!("Failed to replace {}", model_path.display()))?;
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

fn verify_file_sha256(path: &Path, expected_sha256: &str) -> Result<()> {
    let actual_sha256 = file_sha256(path)?;
    if actual_sha256 != expected_sha256 {
        anyhow::bail!(
            "Whisper model checksum mismatch for {}: expected {}, got {}",
            path.display(),
            expected_sha256,
            actual_sha256
        );
    }

    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

async fn download_managed_whisper_model(download_path: PathBuf) -> Result<()> {
    download_managed_whisper_model_from_url(MANAGED_MODEL_URL, download_path).await
}

async fn download_managed_whisper_model_from_url(url: &str, download_path: PathBuf) -> Result<()> {
    let mut response = reqwest::get(url)
        .await
        .context("Failed to start Whisper model download")?
        .error_for_status()
        .context("Whisper model download returned an error status")?;
    let mut file = tokio::fs::File::create(&download_path)
        .await
        .with_context(|| format!("Failed to create {}", download_path.display()))?;

    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read Whisper model download")?
    {
        file.write_all(&chunk)
            .await
            .with_context(|| format!("Failed to write {}", download_path.display()))?;
    }

    file.flush()
        .await
        .with_context(|| format!("Failed to flush {}", download_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    async fn ensure_managed_whisper_model_in_dir_with_sha256<F, Fut>(
        config_dir: &Path,
        expected_sha256: &str,
        downloader: F,
    ) -> Result<PathBuf>
    where
        F: FnOnce(PathBuf) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        ensure_managed_whisper_model_in_dir_with_events(
            config_dir,
            expected_sha256,
            downloader,
            |_| {},
        )
        .await
    }

    #[test]
    fn managed_model_path_uses_config_dir() {
        let temp = tempfile::tempdir().unwrap();

        let path = managed_model_path_in_dir(temp.path());

        assert_eq!(path, temp.path().join("ggml-base.en.bin"));
    }

    #[tokio::test]
    async fn matching_cache_hit_skips_downloader() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());
        std::fs::write(&model_path, b"cached model").unwrap();
        let called = Rc::new(Cell::new(false));
        let called_for_download = called.clone();

        let path = ensure_managed_whisper_model_in_dir_with_sha256(
            temp.path(),
            "14c66da7593e2d9614a0bb4a7169d64085e5e0b952585d2a4ebd4aa5c228d96b",
            move |_| {
                called_for_download.set(true);
                async { Ok(()) }
            },
        )
        .await
        .unwrap();

        assert_eq!(path, model_path);
        assert!(!called.get());
    }

    #[tokio::test]
    async fn cache_miss_downloads_matching_model_to_temp_file_and_renames() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());
        let expected_download_path = model_path.with_extension("bin.download");

        let path = ensure_managed_whisper_model_in_dir_with_sha256(
            temp.path(),
            "6f1b9e8b969d1ea18bd8ba51a2ba697f55142b337f163df6a7daf850453dd161",
            |download_path| async move {
                assert_eq!(download_path, expected_download_path);
                std::fs::write(download_path, b"downloaded model")?;
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(path, model_path);
        assert_eq!(std::fs::read(&model_path).unwrap(), b"downloaded model");
        assert!(!model_path.with_extension("bin.download").exists());
    }

    #[tokio::test]
    async fn mismatched_cache_redownloads_and_replaces_after_verified_download() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());
        std::fs::write(&model_path, b"wrong cached model").unwrap();
        let called = Rc::new(Cell::new(false));
        let called_for_download = called.clone();

        let path = ensure_managed_whisper_model_in_dir_with_sha256(
            temp.path(),
            "6f1b9e8b969d1ea18bd8ba51a2ba697f55142b337f163df6a7daf850453dd161",
            move |download_path| {
                called_for_download.set(true);
                async move {
                    std::fs::write(download_path, b"downloaded model")?;
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(path, model_path);
        assert!(called.get());
        assert_eq!(std::fs::read(&model_path).unwrap(), b"downloaded model");
        assert!(!model_path.with_extension("bin.download").exists());
    }

    #[tokio::test]
    async fn failed_download_leaves_no_final_model_file() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());

        let error = ensure_managed_whisper_model_in_dir_with_sha256(
            temp.path(),
            "6f1b9e8b969d1ea18bd8ba51a2ba697f55142b337f163df6a7daf850453dd161",
            |download_path| async move {
                std::fs::write(download_path, b"partial model")?;
                anyhow::bail!("network failed")
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("network failed"));
        assert!(!model_path.exists());
        assert!(!model_path.with_extension("bin.download").exists());
    }

    #[tokio::test]
    async fn checksum_mismatch_removes_download_and_leaves_no_final_model_file() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = managed_model_path_in_dir(temp.path());

        let error = ensure_managed_whisper_model_in_dir_with_sha256(
            temp.path(),
            "6f1b9e8b969d1ea18bd8ba51a2ba697f55142b337f163df6a7daf850453dd161",
            |download_path| async move {
                std::fs::write(download_path, b"partial model")?;
                Ok(())
            },
        )
        .await
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("Whisper model checksum mismatch")
        );
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

    #[tokio::test]
    async fn download_writes_response_chunks_to_file() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
                )
                .unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let download_path = temp.path().join("model.bin.download");

        download_managed_whisper_model_from_url(&url, download_path.clone())
            .await
            .unwrap();
        server.join().unwrap();

        assert_eq!(std::fs::read(&download_path).unwrap(), b"hello world");
    }
}
