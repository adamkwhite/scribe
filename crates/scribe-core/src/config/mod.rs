use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
use std::future::Future;
use std::path::{Path, PathBuf};

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
const MANAGED_MODEL_FILENAME: &str = "ggml-base.en.bin";

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
const MANAGED_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelDownloadEvent {
    AlreadyPresent(PathBuf),
    Downloading(PathBuf),
    Downloaded(PathBuf),
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Path to whisper.cpp executable
    #[serde(default)]
    pub whisper_bin: Option<String>,

    /// Path to whisper model file (e.g., ggml-base.en.bin)
    pub whisper_model: String,

    /// OpenRouter API key
    pub openrouter_api_key: String,

    /// Model to use for note generation
    #[serde(default = "default_model")]
    pub model: String,

    /// Audio sample rate
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Output directory for recordings and notes
    #[serde(default)]
    pub output_dir: Option<String>,
}

fn default_model() -> String {
    "google/gemini-2.5-flash".to_string()
}

fn default_sample_rate() -> u32 {
    16000
}

pub fn config_dir() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Could not find config directory")?
        .join("scribe"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

fn default_output_dir() -> Result<PathBuf> {
    let dir = dirs::document_dir()
        .or_else(dirs::home_dir)
        .context("Could not find home directory")?
        .join("scribe");
    Ok(dir)
}

pub fn effective_output_dir(cfg: &Config) -> Result<PathBuf> {
    let dir = match cfg.output_dir.as_deref() {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => default_output_dir()?,
    };
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create output directory {}", dir.display()))?;
    Ok(dir)
}

#[cfg(feature = "tui")]
pub fn load_existing() -> Result<Option<Config>> {
    let path = config_path()?;
    if path.exists() {
        let config_dir = path
            .parent()
            .context("Config path has no parent directory")?
            .to_path_buf();
        load_from_path(&path)
            .map(|config| resolve_managed_whisper_model_config(config, &config_dir))
            .map(Some)
    } else {
        Ok(None)
    }
}

pub fn load_from_path(path: &Path) -> Result<Config> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("Failed to parse {}", path.display()))
}

#[cfg(feature = "tui")]
pub fn save(cfg: &Config) -> Result<PathBuf> {
    let path = config_path()?;
    save_to_path(&path, cfg)?;
    Ok(path)
}

pub fn save_to_path(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let toml_str = toml::to_string_pretty(cfg)?;
    std::fs::write(path, toml_str).with_context(|| format!("Failed to write {}", path.display()))
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn validate_setup(cfg: &Config) -> Result<()> {
    let key = cfg.openrouter_api_key.trim();
    if key.is_empty() || key == "YOUR_KEY_HERE" {
        anyhow::bail!("OpenRouter API key is required");
    }

    if cfg.model.trim().is_empty() {
        anyhow::bail!("Notes model is required");
    }

    #[cfg(not(feature = "embedded-whisper"))]
    {
        if cfg
            .whisper_bin
            .as_deref()
            .map(str::trim)
            .filter(|bin| !bin.is_empty())
            .is_none()
        {
            anyhow::bail!("whisper_bin is required when embedded-whisper is disabled");
        }
    }

    let model_path = Path::new(&cfg.whisper_model);
    if !model_path.exists() {
        anyhow::bail!("Whisper model does not exist: {}", model_path.display());
    }

    let output = effective_output_dir(cfg)?;
    let metadata = std::fs::metadata(&output)
        .with_context(|| format!("Failed to inspect {}", output.display()))?;
    if !metadata.is_dir() {
        anyhow::bail!("Output path is not a directory: {}", output.display());
    }

    Ok(())
}

pub async fn load_or_create() -> Result<Config> {
    let path = config_path()?;
    let config_dir = path
        .parent()
        .context("Config path has no parent directory")?
        .to_path_buf();

    let config = if path.exists() {
        load_from_path(&path)?
    } else {
        // Create a default config for the user to fill in
        let config = default_config(&config_dir);

        save_to_path(&path, &config)?;

        println!("Created config at: {}", path.display());
        println!("Please edit it with your whisper model path and OpenRouter API key.");
        #[cfg(all(
            not(feature = "embedded-whisper"),
            not(feature = "auto-download-whisper-model")
        ))]
        println!("Set whisper_bin to your whisper.cpp executable path.\n");
        #[cfg(any(feature = "embedded-whisper", feature = "auto-download-whisper-model"))]
        println!();

        config
    };

    #[cfg(feature = "auto-download-whisper-model")]
    {
        let config = resolve_managed_whisper_model_config(config, &config_dir);
        if is_managed_model_path(&config.whisper_model, &config_dir) {
            ensure_managed_whisper_model().await?;
        }
        Ok(config)
    }

    #[cfg(not(feature = "auto-download-whisper-model"))]
    {
        Ok(config)
    }
}

pub fn default_config(config_dir: &Path) -> Config {
    Config {
        whisper_bin: default_whisper_bin(),
        whisper_model: default_whisper_model(config_dir),
        openrouter_api_key: "YOUR_KEY_HERE".to_string(),
        model: default_model(),
        sample_rate: default_sample_rate(),
        output_dir: None,
    }
}

#[cfg(not(any(feature = "auto-download-whisper-model", feature = "tui")))]
fn default_whisper_model(_config_dir: &Path) -> String {
    "ggml-base.en.bin".to_string()
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
fn default_whisper_model(config_dir: &Path) -> String {
    managed_model_path_in_dir(config_dir)
        .to_string_lossy()
        .into_owned()
}

fn default_whisper_bin() -> Option<String> {
    #[cfg(feature = "embedded-whisper")]
    {
        None
    }

    #[cfg(not(feature = "embedded-whisper"))]
    {
        Some("whisper-cli".to_string())
    }
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
pub fn managed_model_filename() -> &'static str {
    MANAGED_MODEL_FILENAME
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
pub fn managed_model_path_in_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(managed_model_filename())
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
pub fn resolve_managed_whisper_model_config(mut config: Config, config_dir: &Path) -> Config {
    if config.whisper_model == managed_model_filename() {
        config.whisper_model = managed_model_path_in_dir(config_dir)
            .to_string_lossy()
            .into_owned();
    }
    config
}

#[cfg(feature = "auto-download-whisper-model")]
fn is_managed_model_path(model_path: &str, config_dir: &Path) -> bool {
    model_path == managed_model_path_in_dir(config_dir).to_string_lossy()
}

#[cfg(feature = "auto-download-whisper-model")]
pub async fn ensure_managed_whisper_model() -> Result<PathBuf> {
    ensure_managed_whisper_model_with_events(|event| {
        if matches!(event, ModelDownloadEvent::Downloading(_)) {
            println!("{}", event.message());
        }
    })
    .await
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
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

#[cfg(all(test, feature = "auto-download-whisper-model"))]
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

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
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
        return Ok(model_path);
    }

    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("Failed to create {}", config_dir.display()))?;

    let download_path = model_path.with_extension("bin.download");
    let _ = std::fs::remove_file(&download_path);
    on_event(ModelDownloadEvent::Downloading(model_path.clone()));

    if let Err(error) = downloader(download_path.clone()).await {
        let _ = std::fs::remove_file(&download_path);
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
    Ok(model_path)
}

#[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
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

    #[test]
    fn parses_complete_config() {
        let toml_str = r#"
            whisper_bin = "/usr/bin/whisper-cli"
            whisper_model = "/models/ggml-base.en.bin"
            openrouter_api_key = "sk-or-test"
            model = "anthropic/claude-3-5-sonnet"
            sample_rate = 44100
            output_dir = "/tmp/scribe-out"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.whisper_bin.as_deref(), Some("/usr/bin/whisper-cli"));
        assert_eq!(cfg.whisper_model, "/models/ggml-base.en.bin");
        assert_eq!(cfg.openrouter_api_key, "sk-or-test");
        assert_eq!(cfg.model, "anthropic/claude-3-5-sonnet");
        assert_eq!(cfg.sample_rate, 44100);
        assert_eq!(cfg.output_dir.as_deref(), Some("/tmp/scribe-out"));
    }

    #[test]
    fn parses_minimal_config_with_defaults() {
        let toml_str = r#"
            whisper_bin = "whisper-cli"
            whisper_model = "ggml-base.en.bin"
            openrouter_api_key = "sk-or-test"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.whisper_bin.as_deref(), Some("whisper-cli"));
        assert_eq!(cfg.model, "google/gemini-2.5-flash");
        assert_eq!(cfg.sample_rate, 16000);
        assert!(cfg.output_dir.is_none());
    }

    #[test]
    fn parses_config_without_whisper_bin() {
        let toml_str = r#"
            whisper_model = "ggml-base.en.bin"
            openrouter_api_key = "sk-or-test"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.whisper_bin.is_none());
        assert_eq!(cfg.whisper_model, "ggml-base.en.bin");
    }

    #[test]
    fn rejects_config_missing_api_key() {
        let toml_str = r#"
            whisper_model = "ggml-base.en.bin"
        "#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Config {
            whisper_bin: Some("whisper".into()),
            whisper_model: "model.bin".into(),
            openrouter_api_key: "key".into(),
            model: "some/model".into(),
            sample_rate: 22050,
            output_dir: Some("/data/out".into()),
        };
        let serialized = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.whisper_bin, original.whisper_bin);
        assert_eq!(parsed.whisper_model, original.whisper_model);
        assert_eq!(parsed.openrouter_api_key, original.openrouter_api_key);
        assert_eq!(parsed.model, original.model);
        assert_eq!(parsed.sample_rate, original.sample_rate);
        assert_eq!(parsed.output_dir, original.output_dir);
    }

    #[test]
    fn effective_output_dir_uses_configured_output_dir() {
        let temp = tempfile::tempdir().unwrap();
        let configured = temp.path().join("custom-scribe");
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: "model.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(configured.to_string_lossy().into_owned()),
        };

        let result = effective_output_dir(&cfg).unwrap();

        assert_eq!(result, configured);
        assert!(result.exists());
    }

    #[test]
    fn validate_setup_rejects_placeholder_api_key() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("model.bin");
        std::fs::write(&model_path, b"model").unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: model_path.to_string_lossy().into_owned(),
            openrouter_api_key: "YOUR_KEY_HERE".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let error = validate_setup(&cfg).unwrap_err();

        assert!(error.to_string().contains("OpenRouter API key is required"));
    }

    #[test]
    fn validate_setup_rejects_missing_whisper_model() {
        let temp = tempfile::tempdir().unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: temp
                .path()
                .join("missing.bin")
                .to_string_lossy()
                .into_owned(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let error = validate_setup(&cfg).unwrap_err();

        assert!(error.to_string().contains("Whisper model does not exist"));
    }

    #[test]
    fn save_and_load_existing_config_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: "model.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 22050,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        save_to_path(&path, &cfg).unwrap();
        let loaded = load_from_path(&path).unwrap();

        assert_eq!(loaded.whisper_bin, cfg.whisper_bin);
        assert_eq!(loaded.whisper_model, cfg.whisper_model);
        assert_eq!(loaded.openrouter_api_key, cfg.openrouter_api_key);
        assert_eq!(loaded.model, cfg.model);
        assert_eq!(loaded.sample_rate, cfg.sample_rate);
        assert_eq!(loaded.output_dir, cfg.output_dir);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn tui_setup_accepts_managed_model_filename_next_to_config() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("ggml-base.en.bin"), b"model").unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: "ggml-base.en.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let cfg = resolve_managed_whisper_model_config(cfg, temp.path());

        validate_setup(&cfg).unwrap();
        assert_eq!(
            cfg.whisper_model,
            temp.path().join("ggml-base.en.bin").to_string_lossy()
        );
    }

    #[cfg(any(feature = "auto-download-whisper-model", feature = "tui"))]
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

    #[cfg(feature = "auto-download-whisper-model")]
    mod managed_model {
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

            let path =
                ensure_managed_whisper_model_in_dir(temp.path(), |download_path| async move {
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

            let error =
                ensure_managed_whisper_model_in_dir(temp.path(), |download_path| async move {
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
}
