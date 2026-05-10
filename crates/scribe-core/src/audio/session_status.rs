#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub enum SessionStatus {
    Empty,
    RecordingOnly,
    TranscriptReady,
    NotesReady,
}
