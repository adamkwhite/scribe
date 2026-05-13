use crate::audio::AudioSessionTimestamp;

pub trait RuntimeClock: Send + Sync {
    fn recording_timestamp(&self) -> AudioSessionTimestamp;
    fn note_date(&self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalRuntimeClock;

impl RuntimeClock for LocalRuntimeClock {
    fn recording_timestamp(&self) -> AudioSessionTimestamp {
        AudioSessionTimestamp::now_local()
    }

    fn note_date(&self) -> String {
        chrono::Local::now().format("%B %-d, %Y").to_string()
    }
}
