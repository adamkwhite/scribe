#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Empty,
    RecordingOnly,
    TranscriptReady,
    NotesReady,
}
