use anyhow::Result;
use std::{future::Future, pin::Pin};

pub type NoteGenerationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<NoteGenerationOutput>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteGenerationInput {
    pub transcript: String,
    pub context: NoteGenerationContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteGenerationContext {
    pub note_date: String,
    pub system_prompt: NotesSystemPrompt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesSystemPrompt {
    Default,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteGenerationOutput {
    pub markdown: String,
}

pub trait NotesGenerator: Send + Sync {
    fn generate(&self, input: NoteGenerationInput) -> NoteGenerationFuture<'_>;
}
