//! Engine-neutral view of the platform Broker. Engines choose when and how
//! to sample; the Broker alone owns routes, credentials, retries and transport.
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::{
    BrokerEventEnvelope, ChatBrokerPort, ChatModelError, ChatModelErrorCode, ChatModelEvent,
    ChatModelRequest, ChatRetryDirective,
};

pub type EngineModelStream =
    Pin<Box<dyn Stream<Item = Result<ChatModelEvent, ChatModelError>> + Send>>;

#[async_trait]
pub trait EngineModelPort: Send + Sync {
    async fn open_stream(
        &self,
        request: ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineModelStream, ChatModelError>;
}

pub struct BrokerEngineModelPort {
    broker: Arc<dyn ChatBrokerPort>,
}

impl BrokerEngineModelPort {
    /// Supply a Broker already bound to the platform's causality/route owners.
    /// This wrapper does not provide an alternate provider or credential path.
    pub fn new(broker: Arc<dyn ChatBrokerPort>) -> Self {
        Self { broker }
    }
}

#[async_trait]
impl EngineModelPort for BrokerEngineModelPort {
    async fn open_stream(
        &self,
        request: ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<EngineModelStream, ChatModelError> {
        if cancellation.is_cancelled() {
            return Err(ChatModelError::new(
                ChatModelErrorCode::Cancelled,
                "engine model request was cancelled before opening the Broker stream",
                ChatRetryDirective::Never,
            ));
        }
        let stream = self
            .broker
            .open_chat_stream_cancellable(request, cancellation)
            .await?;
        Ok(Box::pin(stream.map(|result| {
            result.map(|envelope: BrokerEventEnvelope| envelope.event)
        })))
    }
}
