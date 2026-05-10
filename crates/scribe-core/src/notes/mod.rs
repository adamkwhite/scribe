mod chat_request;
mod chat_response;
mod generator;
mod message;
mod open_router;

pub use generator::{
    NoteGenerationContext, NoteGenerationFuture, NoteGenerationInput, NoteGenerationOutput,
    NotesGenerator, NotesSystemPrompt,
};
pub use open_router::OpenRouterNotesGenerator;
