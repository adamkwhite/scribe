use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

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
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

fn build_request(transcript: &str, model: &str, today: &str) -> ChatRequest {
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

fn extract_content(chat: ChatResponse) -> Result<String> {
    chat.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .context("No response from model")
}

pub async fn generate(transcript: &str, cfg: &Config) -> Result<String> {
    let client = reqwest::Client::new();

    let today = chrono::Local::now().format("%B %-d, %Y").to_string();
    let request = build_request(transcript, &cfg.model, &today);

    let response = client
        .post(OPENROUTER_URL)
        .header(
            "Authorization",
            format!("Bearer {}", cfg.openrouter_api_key),
        )
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .context("Failed to call OpenRouter API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("OpenRouter API error ({status}): {body}");
    }

    let chat: ChatResponse = response
        .json()
        .await
        .context("Failed to parse OpenRouter response")?;

    extract_content(chat)
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

    #[test]
    fn extract_content_returns_first_choice() {
        let chat = ChatResponse {
            choices: vec![
                Choice {
                    message: Message {
                        role: "assistant".into(),
                        content: "first".into(),
                    },
                },
                Choice {
                    message: Message {
                        role: "assistant".into(),
                        content: "second".into(),
                    },
                },
            ],
        };
        assert_eq!(extract_content(chat).unwrap(), "first");
    }

    #[test]
    fn extract_content_errors_on_empty_choices() {
        let chat = ChatResponse { choices: vec![] };
        assert!(extract_content(chat).is_err());
    }

    #[test]
    fn extract_content_parses_real_openrouter_shape() {
        let json = r#"{
            "choices": [
                {"message": {"role": "assistant", "content": "Summary line\nDetail."}}
            ]
        }"#;
        let chat: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(extract_content(chat).unwrap(), "Summary line\nDetail.");
    }
}
