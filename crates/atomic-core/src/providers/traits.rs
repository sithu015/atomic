//! Provider trait definitions

use crate::providers::error::ProviderError;
use crate::providers::types::{
    CompletionResponse, GenerationParams, Message, StreamDelta, ToolDefinition,
};
use async_trait::async_trait;

/// Configuration for embedding requests
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub model: String,
    /// Target output dimension (Matryoshka). `None` = the model's native
    /// width. When set, it is sent as the `dimensions` request parameter
    /// where the provider supports it, and — because our models are
    /// MRL-trained — also enforced client-side (truncate + L2-renormalize) so
    /// the stored vector width matches regardless of provider support.
    /// Construct via [`crate::providers::ProviderConfig::embedding_config`] so
    /// this always agrees with the model's registered dimension.
    pub dimensions: Option<usize>,
}

impl EmbeddingConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            dimensions: None,
        }
    }

    /// Pin the output vector width (Matryoshka). See [`Self::dimensions`].
    pub fn with_dimensions(mut self, dimensions: usize) -> Self {
        self.dimensions = Some(dimensions);
        self
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: crate::providers::DEFAULT_EMBEDDING_MODEL.to_string(),
            dimensions: None,
        }
    }
}

/// Configuration for LLM requests
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub model: String,
    pub params: GenerationParams,
}

impl LlmConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            params: GenerationParams::default(),
        }
    }

    pub fn with_params(mut self, params: GenerationParams) -> Self {
        self.params = params;
        self
    }
}

/// Provider that can generate text embeddings
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate embeddings for multiple texts (batch)
    async fn embed_batch(
        &self,
        texts: &[String],
        config: &EmbeddingConfig,
    ) -> Result<Vec<Vec<f32>>, ProviderError>;
}

/// Provider that can generate text completions
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a completion for the given messages
    async fn complete(
        &self,
        messages: &[Message],
        config: &LlmConfig,
    ) -> Result<CompletionResponse, ProviderError>;

    /// Generate a completion with tool definitions (non-streaming).
    /// Default implementation ignores tools and falls back to `complete()`.
    async fn complete_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &LlmConfig,
    ) -> Result<CompletionResponse, ProviderError> {
        let _ = tools;
        self.complete(messages, config).await
    }
}

/// Callback type for streaming deltas
pub type StreamCallback = Box<dyn Fn(StreamDelta) + Send + Sync>;

/// Provider that supports streaming completions
#[async_trait]
pub trait StreamingLlmProvider: LlmProvider {
    /// Generate a streaming completion with tools
    async fn complete_streaming_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        config: &LlmConfig,
        on_delta: StreamCallback,
    ) -> Result<CompletionResponse, ProviderError>;
}
