use serde::Serialize;

use super::generator::{NoteGenerationInput, NotesSystemPrompt};
use super::message::Message;

const SYSTEM_PROMPT_TEMPLATE: &str = r#"You are a meeting notes assistant. Given a transcript of a meeting or call, produce structured notes in Markdown format with these sections:

# Meeting Notes — {date}

## Summary
A 2-3 sentence overview of what was discussed.

## Key Points
- Bullet points of important topics discussed

## Action Items
- [ ] Specific tasks mentioned, with owners if identifiable

## Decisions Made
- Any decisions or agreements reached

## Follow-ups
- Topics that need further discussion

Keep it concise. Skip any section that has no content. Use the speakers' words where helpful but don't transcribe verbatim. Always use the date provided in the header above — do not infer a date from the transcript content."#;

#[derive(Serialize)]
#[cfg_attr(test, derive(Clone, Debug, PartialEq, Eq))]
pub(super) struct ChatRequest {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
}

pub(super) fn build_request(input: &NoteGenerationInput, model: &str) -> ChatRequest {
    let system_prompt = match &input.context.system_prompt {
        NotesSystemPrompt::Default => default_system_prompt(&input.context.note_date),
        NotesSystemPrompt::Custom(prompt) => prompt.clone(),
    };
    ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: system_prompt,
            },
            Message {
                role: "user".to_string(),
                content: format!("Here is the meeting transcript:\n\n{}", input.transcript),
            },
        ],
    }
}

fn default_system_prompt(note_date: &str) -> String {
    SYSTEM_PROMPT_TEMPLATE.replace("{date}", note_date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{NoteGenerationContext, NoteGenerationInput, NotesSystemPrompt};

    #[test]
    fn build_request_substitutes_date_and_embeds_transcript() {
        let input = NoteGenerationInput {
            transcript: "Hello world".to_string(),
            context: NoteGenerationContext {
                note_date: "January 1, 2026".to_string(),
                system_prompt: NotesSystemPrompt::Default,
            },
        };

        let req = build_request(&input, "test/model");
        assert_eq!(req.model, "test/model");
        assert_eq!(req.messages.len(), 2);

        let system = &req.messages[0];
        assert_eq!(system.role, "system");
        assert!(system.content.contains("January 1, 2026"));
        assert!(!system.content.contains("{date}"));

        let user = &req.messages[1];
        assert_eq!(user.role, "user");
        assert!(user.content.contains("Hello world"));
        assert!(user.content.starts_with("Here is the meeting transcript:"));
    }

    #[test]
    fn build_request_handles_empty_transcript() {
        let input = NoteGenerationInput {
            transcript: String::new(),
            context: NoteGenerationContext {
                note_date: "today".to_string(),
                system_prompt: NotesSystemPrompt::Default,
            },
        };

        let req = build_request(&input, "model");
        assert_eq!(
            req.messages[1].content,
            "Here is the meeting transcript:\n\n"
        );
    }

    #[test]
    fn build_request_uses_custom_system_prompt_verbatim() {
        let input = NoteGenerationInput {
            transcript: "Transcript".to_string(),
            context: NoteGenerationContext {
                note_date: "January 1, 2026".to_string(),
                system_prompt: NotesSystemPrompt::Custom("Custom {date} prompt".to_string()),
            },
        };

        let req = build_request(&input, "model");

        assert_eq!(req.messages[0].content, "Custom {date} prompt");
    }
}
