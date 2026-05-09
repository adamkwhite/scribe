use anyhow::{Context, Result, anyhow};
use rodio::Source;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct PlaybackController {
    _stream: rodio::OutputStream,
    sink: rodio::Sink,
    path: PathBuf,
    duration: Option<Duration>,
}

impl PlaybackController {
    pub fn open(path: &Path) -> Result<Self> {
        let stream = rodio::OutputStreamBuilder::open_default_stream()
            .context("Failed to open default audio output")?;
        let sink = rodio::Sink::connect_new(stream.mixer());
        sink.pause();

        let mut controller = Self {
            _stream: stream,
            sink,
            path: path.to_path_buf(),
            duration: None,
        };
        controller.reload_source()?;
        Ok(controller)
    }

    pub fn play(&mut self) -> Result<()> {
        self.ensure_source_loaded()?;
        self.sink.play();
        Ok(())
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn toggle_play_pause(&mut self) -> Result<()> {
        if self.sink.is_paused() {
            self.play()?;
        } else {
            self.pause();
        }
        Ok(())
    }

    pub fn restart(&mut self) -> Result<()> {
        self.ensure_source_loaded()?;
        self.seek(Duration::ZERO)?;
        self.play()?;
        Ok(())
    }

    pub fn rewind(&mut self, amount: Duration) -> Result<()> {
        self.ensure_source_loaded()?;
        self.seek(rewind_position(self.position(), amount))
    }

    pub fn fast_forward(&mut self, amount: Duration) -> Result<()> {
        self.ensure_source_loaded()?;
        self.seek(fast_forward_position(
            self.position(),
            amount,
            self.duration,
        ))
    }

    pub fn stop_reset(&mut self) -> Result<()> {
        self.ensure_source_loaded()?;
        self.pause();
        self.seek(Duration::ZERO)
    }

    pub fn position(&self) -> Duration {
        self.sink.get_pos()
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
        self.ensure_source_loaded()?;
        self.sink
            .try_seek(position)
            .map_err(|error| anyhow!("Failed to seek recording playback: {error:?}"))
    }

    fn ensure_source_loaded(&mut self) -> Result<()> {
        if source_needs_reload(self.sink.empty()) {
            self.reload_source()?;
        }
        Ok(())
    }

    fn reload_source(&mut self) -> Result<()> {
        self.sink = rodio::Sink::connect_new(self._stream.mixer());
        self.sink.pause();

        let file = File::open(&self.path)
            .with_context(|| format!("Failed to open {}", self.path.display()))?;
        let source = rodio::Decoder::try_from(file)
            .with_context(|| format!("Failed to decode {}", self.path.display()))?;
        self.duration = source.total_duration();
        self.sink.append(source);
        Ok(())
    }
}

pub struct PlaybackViewState {
    pub session_name: String,
    pub path: PathBuf,
    pub controller: Option<PlaybackController>,
    pub status: String,
    pub error: Option<String>,
}

impl PlaybackViewState {
    pub fn open(session_name: String, path: PathBuf) -> Self {
        match PlaybackController::open(&path) {
            Ok(controller) => Self {
                session_name,
                path,
                controller: Some(controller),
                status: "Ready".to_string(),
                error: None,
            },
            Err(error) => Self {
                session_name,
                path,
                controller: None,
                status: "Unavailable".to_string(),
                error: Some(error.to_string()),
            },
        }
    }

    pub fn stop(&mut self) {
        if let Some(controller) = self.controller.as_mut()
            && let Err(error) = controller.stop_reset()
        {
            self.error = Some(error.to_string());
        }
        self.status = "Stopped".to_string();
    }

    pub fn position(&self) -> Duration {
        self.controller
            .as_ref()
            .map(PlaybackController::position)
            .unwrap_or(Duration::ZERO)
    }

    pub fn duration(&self) -> Option<Duration> {
        self.controller
            .as_ref()
            .and_then(PlaybackController::duration)
    }

    pub fn is_playing(&self) -> bool {
        self.controller
            .as_ref()
            .is_some_and(|controller| !controller.is_paused())
    }
}

pub fn rewind_position(position: Duration, amount: Duration) -> Duration {
    position.checked_sub(amount).unwrap_or(Duration::ZERO)
}

pub fn fast_forward_position(
    position: Duration,
    amount: Duration,
    duration: Option<Duration>,
) -> Duration {
    let next = position.saturating_add(amount);
    duration.map_or(next, |duration| next.min(duration))
}

fn source_needs_reload(sink_is_empty: bool) -> bool {
    sink_is_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sink_requires_source_reload_before_replay() {
        assert!(source_needs_reload(true));
        assert!(!source_needs_reload(false));
    }
}
