//! Streaming client for any OpenAI-compatible endpoint.
//!
//! Self-hosted and hosted endpoints differ only by base URL, so the same client
//! serves both. Replies stream: tokens are handed on as they arrive rather than
//! withheld until the reply is complete, which is what lets synthesis start on
//! the first sentence.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared_kernel::{AppError, AppResult};

use crate::ports::{
    LlmCompletionRequest, LlmDelta, LlmGateway, LlmStream, LlmUsage, ToolCall, ToolDefinition,
};

#[derive(Debug, Clone)]
pub struct ReqwestLlmGateway {
    client: Client,
}

impl Default for ReqwestLlmGateway {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[async_trait]
impl LlmGateway for ReqwestLlmGateway {
    async fn stream(&self, request: LlmCompletionRequest) -> AppResult<LlmStream> {
        let endpoint_url = request.endpoint_url.trim().to_owned();
        if endpoint_url.is_empty() {
            return Err(AppError::invalid_input("llm endpoint_url is required"));
        }
        if request.model_name.trim().is_empty() {
            return Err(AppError::invalid_input("llm model_name is required"));
        }
        if request.messages.is_empty() {
            return Err(AppError::invalid_input("llm messages are required"));
        }

        let body = ChatCompletionRequest {
            model: request.model_name.clone(),
            temperature: request.temperature,
            frequency_penalty: (request.frequency_penalty != 0.0)
                .then_some(request.frequency_penalty),
            messages: request
                .messages
                .into_iter()
                .map(|message| ChatMessage {
                    role: message.role,
                    content: message.content,
                })
                .collect(),
            max_tokens: request.max_tokens,
            stream: true,
            // Without this an endpoint reports no usage for a streamed reply,
            // and the turn would be recorded with none.
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            tools: (!request.tools.is_empty()).then(|| {
                request
                    .tools
                    .iter()
                    .map(|tool| Tool {
                        kind: "function",
                        function: ToolFunction::from(tool),
                    })
                    .collect()
            }),
        };

        let response = self
            .client
            .post(&endpoint_url)
            .bearer_auth(request.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::unavailable(format!("llm request failed: {error}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable response body>"));
            return Err(AppError::unavailable(format!(
                "llm request failed with status {status}: {body}"
            )));
        }

        Ok(Box::pin(decode(response.bytes_stream())))
    }
}

/// Turns the response body into deltas.
///
/// Server-sent events arrive as `data:` lines that do not necessarily align
/// with network reads, so bytes are buffered until a line is complete. Tool
/// calls arrive as fragments across many events and are assembled here, so a
/// consumer sees each call once and whole.
fn decode<S>(body: S) -> impl futures::Stream<Item = AppResult<LlmDelta>> + Send
where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
{
    type Body = futures::stream::BoxStream<'static, reqwest::Result<bytes::Bytes>>;
    let start: (Body, DecodeState) = (body.boxed(), DecodeState::default());
    futures::stream::unfold(start, |(mut body, mut state)| async move {
        loop {
            if let Some(delta) = state.take_ready() {
                return Some((Ok(delta), (body, state)));
            }
            if state.finished {
                return None;
            }
            match body.next().await {
                Some(Ok(bytes)) => {
                    if let Err(error) = state.push(&bytes) {
                        return Some((Err(error), (body, state)));
                    }
                }
                Some(Err(error)) => {
                    return Some((
                        Err(AppError::unavailable(format!("llm stream failed: {error}"))),
                        (body, state),
                    ));
                }
                // The endpoint closed without `[DONE]`. Whatever was assembled
                // is still worth reporting.
                None => {
                    state.finish();
                }
            }
        }
    })
}

#[derive(Default)]
struct DecodeState {
    buffer: String,
    ready: std::collections::VecDeque<LlmDelta>,
    /// Tool calls under construction, keyed by the index the stream assigns.
    partial_tools: std::collections::BTreeMap<u32, PartialTool>,
    usage: LlmUsage,
    finished: bool,
    done_emitted: bool,
}

#[derive(Default)]
struct PartialTool {
    id: String,
    name: String,
    arguments: String,
}

impl DecodeState {
    fn take_ready(&mut self) -> Option<LlmDelta> {
        self.ready.pop_front()
    }

    fn push(&mut self, bytes: &[u8]) -> AppResult<()> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        while let Some(position) = self.buffer.find('\n') {
            let line = self.buffer[..position].trim().to_owned();
            self.buffer.drain(..=position);
            if line.is_empty() {
                continue;
            }
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                self.finish();
                continue;
            }
            let chunk: ChatCompletionChunk = serde_json::from_str(payload).map_err(|error| {
                AppError::internal(format!("failed to decode llm stream chunk: {error}"))
            })?;
            self.absorb(chunk);
        }
        Ok(())
    }

    fn absorb(&mut self, chunk: ChatCompletionChunk) {
        if let Some(usage) = chunk.usage {
            self.usage = LlmUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
            };
        }
        for choice in chunk.choices {
            if let Some(content) = choice.delta.content
                && !content.is_empty()
            {
                self.ready.push_back(LlmDelta::Token(content));
            }
            for fragment in choice.delta.tool_calls {
                let entry = self.partial_tools.entry(fragment.index).or_default();
                if let Some(id) = fragment.id {
                    entry.id = id;
                }
                if let Some(function) = fragment.function {
                    if let Some(name) = function.name {
                        entry.name = name;
                    }
                    if let Some(arguments) = function.arguments {
                        entry.arguments.push_str(&arguments);
                    }
                }
            }
            // `tool_calls` as a finish reason means every fragment has arrived.
            if choice.finish_reason.as_deref() == Some("tool_calls") {
                self.flush_tools();
            }
        }
    }

    fn flush_tools(&mut self) {
        for (_, tool) in std::mem::take(&mut self.partial_tools) {
            if tool.name.is_empty() {
                continue;
            }
            self.ready.push_back(LlmDelta::ToolCall(ToolCall {
                id: tool.id,
                name: tool.name,
                arguments: tool.arguments,
            }));
        }
    }

    fn finish(&mut self) {
        if self.done_emitted {
            return;
        }
        self.flush_tools();
        self.ready.push_back(LlmDelta::Done(self.usage));
        self.done_emitted = true;
        self.finished = true;
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct Tool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ToolFunction,
}

#[derive(Serialize)]
struct ToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl From<&ToolDefinition> for ToolFunction {
    fn from(tool: &ToolDefinition) -> Self {
        Self {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        }
    }
}

#[derive(Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: ChunkDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallFragment>,
}

#[derive(Deserialize)]
struct ToolCallFragment {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ToolCallFunctionFragment>,
}

#[derive(Deserialize)]
struct ToolCallFunctionFragment {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: i32,
    #[serde(default)]
    completion_tokens: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a canned body through the decoder, split at the given byte
    /// boundaries so that events straddle reads the way they do on a socket.
    async fn decode_body(parts: &[&str]) -> Vec<LlmDelta> {
        let chunks: Vec<reqwest::Result<bytes::Bytes>> = parts
            .iter()
            .map(|part| Ok(bytes::Bytes::from(part.to_string())))
            .collect();
        decode(futures::stream::iter(chunks))
            .map(|delta| delta.expect("delta"))
            .collect()
            .await
    }

    #[tokio::test]
    async fn tokens_arrive_one_by_one() {
        let deltas = decode_body(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: [DONE]\n\n",
        ])
        .await;
        assert_eq!(
            deltas,
            vec![
                LlmDelta::Token("Hel".to_owned()),
                LlmDelta::Token("lo".to_owned()),
                LlmDelta::Done(LlmUsage::default()),
            ]
        );
    }

    #[tokio::test]
    async fn an_event_split_across_reads_is_still_decoded() {
        // Network reads do not respect event boundaries.
        let deltas = decode_body(&[
            "data: {\"choices\":[{\"delta\":{\"con",
            "tent\":\"split\"}}]}\n\ndata: [DONE]\n\n",
        ])
        .await;
        assert_eq!(
            deltas,
            vec![
                LlmDelta::Token("split".to_owned()),
                LlmDelta::Done(LlmUsage::default()),
            ]
        );
    }

    #[tokio::test]
    async fn tool_call_fragments_are_assembled_into_one_call() {
        let deltas = decode_body(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\
             \"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"q\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\
             \"function\":{\"arguments\":\"\\\"rain\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        ])
        .await;
        assert_eq!(
            deltas,
            vec![
                LlmDelta::ToolCall(ToolCall {
                    id: "call_1".to_owned(),
                    name: "lookup".to_owned(),
                    arguments: "{\"q\":\"rain\"}".to_owned(),
                }),
                LlmDelta::Done(LlmUsage::default()),
            ]
        );
    }

    #[tokio::test]
    async fn usage_from_the_final_event_is_reported() {
        let deltas = decode_body(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        ])
        .await;
        assert_eq!(
            deltas.last(),
            Some(&LlmDelta::Done(LlmUsage {
                prompt_tokens: 11,
                completion_tokens: 2,
            }))
        );
    }

    #[tokio::test]
    async fn a_stream_that_ends_without_done_still_completes() {
        let deltas =
            decode_body(&["data: {\"choices\":[{\"delta\":{\"content\":\"cut\"}}]}\n\n"]).await;
        assert_eq!(
            deltas,
            vec![
                LlmDelta::Token("cut".to_owned()),
                LlmDelta::Done(LlmUsage::default()),
            ]
        );
    }
}
