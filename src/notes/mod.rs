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

pub async fn generate(transcript: &str, cfg: &Config) -> Result<String> {
    let client = reqwest::Client::new();

    let today = chrono::Local::now().format("%B %-d, %Y").to_string();
    let system_prompt = SYSTEM_PROMPT_TEMPLATE.replace("{date}", &today);

    let request = ChatRequest {
        model: cfg.model.clone(),
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
    };

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

    chat.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .context("No response from model")
}
