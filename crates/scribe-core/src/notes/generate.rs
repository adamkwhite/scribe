use anyhow::{Context, Result};

use crate::config::Config;

use super::chat_request::build_request;
use super::chat_response::{ChatResponse, extract_content};

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

pub async fn generate(transcript: &str, cfg: &Config) -> Result<String> {
    let client = reqwest::Client::new();

    let today = chrono::Local::now().format("%B %-d, %Y").to_string();
    let request = build_request(transcript, &cfg.model, &today);
    tracing::info!(
        model = %cfg.model,
        transcript_chars = transcript.len(),
        "calling OpenRouter notes API"
    );

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
        tracing::error!(
            status = %status,
            response_chars = body.len(),
            "OpenRouter notes API returned error"
        );
        anyhow::bail!("OpenRouter API error ({status}): {body}");
    }

    let chat: ChatResponse = response
        .json()
        .await
        .context("Failed to parse OpenRouter response")?;

    let notes = extract_content(chat)?;
    tracing::info!(
        notes_chars = notes.len(),
        "OpenRouter notes API response parsed"
    );
    Ok(notes)
}
