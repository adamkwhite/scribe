use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioRecordingEvent {
    LoopbackDevice(String),
    MicDevice(String),
    AudioConfig {
        loopback_sample_rate: u32,
        loopback_channels: u16,
        mic_sample_rate: u32,
        mic_channels: u16,
        output_sample_rate: u32,
    },
    StreamError {
        source: &'static str,
        error: String,
    },
    SavedRecording(PathBuf),
}

impl AudioRecordingEvent {
    pub fn message(&self) -> String {
        match self {
            Self::LoopbackDevice(name) => format!("Loopback: {name}"),
            Self::MicDevice(name) => format!("Mic: {name}"),
            Self::AudioConfig {
                loopback_sample_rate,
                loopback_channels,
                mic_sample_rate,
                mic_channels,
                output_sample_rate,
            } => format!(
                "Loopback: {loopback_sample_rate}Hz {loopback_channels}ch, Mic: {mic_sample_rate}Hz {mic_channels}ch, Output: {output_sample_rate}Hz mono"
            ),
            Self::StreamError { source, error } => format!("{source} stream error: {error}"),
            Self::SavedRecording(path) => format!("Saved recording to: {}", path.display()),
        }
    }

    pub(super) fn print(&self) {
        match self {
            Self::StreamError { .. } => eprintln!("{}", self.message()),
            _ => println!("{}", self.message()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_recording_events_format_user_visible_messages() {
        let saved_path = PathBuf::from("/tmp/scribe/recording.wav");

        assert_eq!(
            AudioRecordingEvent::LoopbackDevice("HD Pro Webcam C920".into()).message(),
            "Loopback: HD Pro Webcam C920"
        );
        assert_eq!(
            AudioRecordingEvent::MicDevice("Studio Mic".into()).message(),
            "Mic: Studio Mic"
        );
        assert_eq!(
            AudioRecordingEvent::AudioConfig {
                loopback_sample_rate: 16000,
                loopback_channels: 2,
                mic_sample_rate: 48000,
                mic_channels: 1,
                output_sample_rate: 16000,
            }
            .message(),
            "Loopback: 16000Hz 2ch, Mic: 48000Hz 1ch, Output: 16000Hz mono"
        );
        assert_eq!(
            AudioRecordingEvent::SavedRecording(saved_path.clone()).message(),
            format!("Saved recording to: {}", saved_path.display())
        );
        assert_eq!(
            AudioRecordingEvent::StreamError {
                source: "Mic",
                error: "device lost".into(),
            }
            .message(),
            "Mic stream error: device lost"
        );
    }
}
