use serde::Serialize;

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
pub(super) struct ChatRequest {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
}

pub(super) fn build_request(transcript: &str, model: &str, today: &str) -> ChatRequest {
    let system_prompt = SYSTEM_PROMPT_TEMPLATE.replace("{date}", today);
    ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: system_prompt,
            },
            Message {
                role: "user".to_string(),
                content: format!("Here is the meeting transcript:\n\n{transcript}"),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_substitutes_date_and_embeds_transcript() {
        let req = build_request("Hello world", "test/model", "January 1, 2026");
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
        let req = build_request("", "model", "today");
        assert_eq!(
            req.messages[1].content,
            "Here is the meeting transcript:\n\n"
        );
    }
}
