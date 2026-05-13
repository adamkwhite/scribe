use anyhow::{Context, Result};
use serde::Deserialize;

use super::message::Message;

#[derive(Deserialize)]
pub(super) struct ChatResponse {
    pub(super) choices: Vec<Choice>,
}

#[derive(Deserialize)]
pub(super) struct Choice {
    pub(super) message: Message,
}

pub(super) fn extract_content(chat: ChatResponse) -> Result<String> {
    chat.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .context("No response from model")
}

#[cfg(test)]
mod tests {
    use super::*;

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
