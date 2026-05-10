use anyhow::{Context, Result};
use reqwest::StatusCode;
use std::{future::Future, pin::Pin, sync::Arc};

use crate::config::Config;

use super::chat_request::{ChatRequest, build_request};
use super::chat_response::{ChatResponse, extract_content};
use super::generator::{
    NoteGenerationFuture, NoteGenerationInput, NoteGenerationOutput, NotesGenerator,
};

const OPEN_ROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

pub struct OpenRouterNotesGenerator {
    model: String,
    api_key: String,
    transport: Arc<dyn OpenRouterTransport>,
}

impl OpenRouterNotesGenerator {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            model: cfg.model.clone(),
            api_key: cfg.openrouter_api_key.clone(),
            transport: Arc::new(ReqwestOpenRouterTransport::new()),
        }
    }

    #[cfg(test)]
    fn new(model: String, api_key: String, transport: Arc<dyn OpenRouterTransport>) -> Self {
        Self {
            model,
            api_key,
            transport,
        }
    }
}

impl NotesGenerator for OpenRouterNotesGenerator {
    fn generate(&self, input: NoteGenerationInput) -> NoteGenerationFuture<'_> {
        Box::pin(async move {
            let transcript_chars = input.transcript.len();
            let request = build_request(&input, &self.model);
            tracing::info!(
                model = %self.model,
                transcript_chars,
                "calling OpenRouter notes API"
            );

            let response = self
                .transport
                .send(OpenRouterTransportRequest {
                    api_key: self.api_key.clone(),
                    request,
                })
                .await
                .context("Failed to call OpenRouter API")?;

            if !response.status.is_success() {
                tracing::error!(
                    status = %response.status,
                    response_chars = response.body.len(),
                    "OpenRouter notes API returned error"
                );
                anyhow::bail!(
                    "OpenRouter API error ({}): {}",
                    response.status,
                    response.body
                );
            }

            let chat: ChatResponse = serde_json::from_str(&response.body)
                .context("Failed to parse OpenRouter response")?;
            let notes = extract_content(chat)?;
            tracing::info!(
                notes_chars = notes.len(),
                "OpenRouter notes API response parsed"
            );
            Ok(NoteGenerationOutput { markdown: notes })
        })
    }
}

type OpenRouterTransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<OpenRouterTransportResponse>> + Send + 'a>>;

trait OpenRouterTransport: Send + Sync {
    fn send(&self, request: OpenRouterTransportRequest) -> OpenRouterTransportFuture<'_>;
}

struct OpenRouterTransportRequest {
    api_key: String,
    request: ChatRequest,
}

struct OpenRouterTransportResponse {
    status: StatusCode,
    body: String,
}

struct ReqwestOpenRouterTransport {
    client: reqwest::Client,
}

impl ReqwestOpenRouterTransport {
    fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl OpenRouterTransport for ReqwestOpenRouterTransport {
    fn send(&self, request: OpenRouterTransportRequest) -> OpenRouterTransportFuture<'_> {
        Box::pin(async move {
            let response = self
                .client
                .post(OPEN_ROUTER_URL)
                .header("Authorization", format!("Bearer {}", request.api_key))
                .header("Content-Type", "application/json")
                .json(&request.request)
                .send()
                .await?;
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Ok(OpenRouterTransportResponse { status, body })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{NoteGenerationContext, NotesSystemPrompt};
    use anyhow::anyhow;
    use std::{
        collections::VecDeque,
        sync::{Mutex, MutexGuard},
    };

    #[derive(Clone)]
    struct FakeTransport {
        requests: Arc<Mutex<Vec<OpenRouterTransportRequest>>>,
        responses: Arc<Mutex<VecDeque<Result<OpenRouterTransportResponse, String>>>>,
    }

    impl FakeTransport {
        fn with_response(response: OpenRouterTransportResponse) -> Self {
            Self::with_result(Ok(response))
        }

        fn with_result(response: Result<OpenRouterTransportResponse, String>) -> Self {
            let mut responses = VecDeque::new();
            responses.push_back(response);
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn requests(&self) -> MutexGuard<'_, Vec<OpenRouterTransportRequest>> {
            self.requests.lock().unwrap()
        }
    }

    impl OpenRouterTransport for FakeTransport {
        fn send(&self, request: OpenRouterTransportRequest) -> OpenRouterTransportFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake response");
            Box::pin(async move { response.map_err(|message| anyhow!(message)) })
        }
    }

    #[tokio::test]
    async fn open_router_generator_sends_typed_request_and_returns_markdown() {
        let transport = FakeTransport::with_response(OpenRouterTransportResponse {
            status: StatusCode::OK,
            body: r##"{"choices":[{"message":{"role":"assistant","content":"# Notes"}}]}"##
                .to_string(),
        });
        let generator = OpenRouterNotesGenerator::new(
            "test/model".to_string(),
            "sk-test".to_string(),
            Arc::new(transport.clone()),
        );

        let output = generator
            .generate(input_with_default_prompt())
            .await
            .unwrap();

        assert_eq!(
            output,
            NoteGenerationOutput {
                markdown: "# Notes".to_string()
            }
        );
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.api_key, "sk-test");
        assert_eq!(request.request.model, "test/model");
        assert!(
            request.request.messages[0]
                .content
                .contains("January 1, 2026")
        );
        assert!(
            request.request.messages[1]
                .content
                .contains("Status update transcript")
        );
    }

    #[tokio::test]
    async fn open_router_generator_returns_http_error_body() {
        let transport = FakeTransport::with_response(OpenRouterTransportResponse {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "rate limited".to_string(),
        });
        let generator = OpenRouterNotesGenerator::new(
            "test/model".to_string(),
            "sk-test".to_string(),
            Arc::new(transport),
        );

        let error = generator
            .generate(input_with_default_prompt())
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "OpenRouter API error (429 Too Many Requests): rate limited"
        );
    }

    #[tokio::test]
    async fn open_router_generator_reports_invalid_json() {
        let transport = FakeTransport::with_response(OpenRouterTransportResponse {
            status: StatusCode::OK,
            body: "not json".to_string(),
        });
        let generator = OpenRouterNotesGenerator::new(
            "test/model".to_string(),
            "sk-test".to_string(),
            Arc::new(transport),
        );

        let error = generator
            .generate(input_with_default_prompt())
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "Failed to parse OpenRouter response");
    }

    #[tokio::test]
    async fn open_router_generator_reports_empty_choices() {
        let transport = FakeTransport::with_response(OpenRouterTransportResponse {
            status: StatusCode::OK,
            body: r#"{"choices":[]}"#.to_string(),
        });
        let generator = OpenRouterNotesGenerator::new(
            "test/model".to_string(),
            "sk-test".to_string(),
            Arc::new(transport),
        );

        let error = generator
            .generate(input_with_default_prompt())
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "No response from model");
    }

    #[tokio::test]
    async fn open_router_generator_wraps_transport_errors() {
        let transport = FakeTransport::with_result(Err("transport down".to_string()));
        let generator = OpenRouterNotesGenerator::new(
            "test/model".to_string(),
            "sk-test".to_string(),
            Arc::new(transport),
        );

        let error = generator
            .generate(input_with_default_prompt())
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "Failed to call OpenRouter API");
    }

    fn input_with_default_prompt() -> NoteGenerationInput {
        NoteGenerationInput {
            transcript: "Status update transcript".to_string(),
            context: NoteGenerationContext {
                note_date: "January 1, 2026".to_string(),
                system_prompt: NotesSystemPrompt::Default,
            },
        }
    }
}
