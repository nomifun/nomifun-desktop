use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use nomifun_chat_model_broker::{
    BrokerEventEnvelope, ChatBrokerPort, ChatModelError, ChatModelEvent, ChatModelRequest,
    ChatModelStream,
};
use tokio_util::sync::CancellationToken;

pub type CodingModelStream =
    Pin<Box<dyn Stream<Item = Result<ChatModelEvent, ChatModelError>> + Send>>;

#[async_trait]
pub trait CodingModelPort: Send + Sync {
    async fn open_stream(
        &self,
        request: ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<CodingModelStream, ChatModelError>;
}

/// Adapter from the existing NomiFun Broker to the isolated Coding Engine.
///
/// The current broker API exposes cancellation by dropping its receiver. The
/// adapter additionally stops forwarding events as soon as the engine token
/// is cancelled. A later central integration can extend the broker port with
/// native cancellation without changing the engine turn loop.
pub struct BrokerCodingModelPort {
    broker: Arc<dyn ChatBrokerPort>,
}

impl BrokerCodingModelPort {
    pub fn new(broker: Arc<dyn ChatBrokerPort>) -> Self {
        Self { broker }
    }
}

#[async_trait]
impl CodingModelPort for BrokerCodingModelPort {
    async fn open_stream(
        &self,
        request: ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<CodingModelStream, ChatModelError> {
        if cancellation.is_cancelled() {
            return Err(ChatModelError::new(
                nomifun_chat_model_broker::ChatModelErrorCode::Cancelled,
                "coding model request was cancelled before opening the broker stream",
                nomifun_chat_model_broker::ChatRetryDirective::Never,
            ));
        }
        let stream = self.broker.open_chat_stream(request).await?;
        Ok(Box::pin(BrokerCodingStream {
            stream,
            cancellation,
        }))
    }
}

struct BrokerCodingStream {
    stream: ChatModelStream,
    cancellation: CancellationToken,
}

impl Stream for BrokerCodingStream {
    type Item = Result<ChatModelEvent, ChatModelError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.cancellation.is_cancelled() {
            return Poll::Ready(None);
        }

        self.stream
            .poll_next_unpin(context)
            .map(|item| {
                item.map(|result| result.map(|envelope: BrokerEventEnvelope| envelope.event))
            })
    }
}
