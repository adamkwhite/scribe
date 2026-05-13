use crate::audio::{AudioRecordingOutput, RecordingControl};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct ActiveRecording {
    session_dir: PathBuf,
    control: RecordingControl,
    task: tokio::task::JoinHandle<Result<AudioRecordingOutput>>,
}

impl ActiveRecording {
    pub(crate) fn new(
        session_dir: PathBuf,
        control: RecordingControl,
        task: tokio::task::JoinHandle<Result<AudioRecordingOutput>>,
    ) -> Self {
        Self {
            session_dir,
            control,
            task,
        }
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn is_recording(&self) -> bool {
        self.control.is_recording()
    }

    pub fn stop(&self) {
        self.control.stop();
    }

    pub async fn wait(self) -> Result<AudioRecordingOutput> {
        self.task.await.context("Recording task failed to join")?
    }
}
