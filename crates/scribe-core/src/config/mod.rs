mod default_config;
mod load_or_create;
mod managed_model;
mod model_download_event;
mod output_dir;
mod paths;
mod persistence;
mod settings;
mod validate_setup;

pub use default_config::default_config;
pub use load_or_create::{ConfigOrigin, load_or_create};
pub use managed_model::{
    ensure_managed_whisper_model, ensure_managed_whisper_model_with_events, managed_model_filename,
    managed_model_path_in_dir, resolve_managed_whisper_model_config,
};
pub use model_download_event::ModelDownloadEvent;
pub use output_dir::effective_output_dir;
pub use paths::{config_dir, config_path};
pub use persistence::{load_from_path, save_to_path};
pub use settings::Config;
pub use validate_setup::validate_setup;
