use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use nomifun_chat_model_broker::{
    BrokerEventEnvelope, ChatBrokerPort, ChatModelError, ChatModelEvent, ChatModelRequest,
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
/// Cancellation is passed to the Broker's native attempt lifecycle. Dropping
/// the returned stream also cancels its attempt; the Broker remains the sole
/// owner of retry/failover and provider transport.
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
        let stream = self.broker
            .open_chat_stream_cancellable(request, cancellation)
            .await?;
        Ok(Box::pin(stream.map(|result| {
            result.map(|envelope: BrokerEventEnvelope| envelope.event)
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_chat_model_broker::{
        ChatModelErrorCode, ChatModelStream, ChatRetryDirective, recorded_conformance_fixtures,
    };
    use std::time::Duration;
    use tokio::sync::Notify;

    struct CancellableBroker(Arc<Notify>);

    struct LegacyOnlyBroker;

    #[async_trait]
    impl ChatBrokerPort for LegacyOnlyBroker {
        async fn open_chat_stream(&self, _: ChatModelRequest) -> Result<ChatModelStream, ChatModelError> {
            panic!("unsupported cancellation must not fall back to legacy opening");
        }
    }

    #[tokio::test]
    async fn adapter_rejects_broker_without_native_cancellation() {
        let adapter = BrokerCodingModelPort::new(Arc::new(LegacyOnlyBroker));
        let request = recorded_conformance_fixtures().remove(0).request;
        let error = adapter.open_stream(request, CancellationToken::new()).await
            .err().expect("unsupported cancellation must fail closed");
        assert_eq!(error.code, ChatModelErrorCode::AdapterUnavailable);
    }

    #[async_trait]
    impl ChatBrokerPort for CancellableBroker {
        async fn open_chat_stream(&self, _: ChatModelRequest) -> Result<ChatModelStream, ChatModelError> {
            panic!("Coding must not fall back to the uncancellable broker port");
        }

        async fn open_chat_stream_cancellable(
            &self,
            _: ChatModelRequest,
            cancellation: CancellationToken,
        ) -> Result<ChatModelStream, ChatModelError> {
            self.0.notify_one();
            cancellation.cancelled().await;
            Err(ChatModelError::new(
                ChatModelErrorCode::Cancelled,
                "attempt cancelled",
                ChatRetryDirective::Never,
            ))
        }
    }

    #[tokio::test]
    async fn adapter_propagates_cancellation_during_broker_open() {
        let entered = Arc::new(Notify::new());
        let adapter = BrokerCodingModelPort::new(Arc::new(CancellableBroker(entered.clone())));
        let cancellation = CancellationToken::new();
        let token = cancellation.clone();
        let request = recorded_conformance_fixtures().remove(0).request;
        let task = tokio::spawn(async move { adapter.open_stream(request, token).await });
        tokio::time::timeout(Duration::from_secs(2), entered.notified()).await.unwrap();
        cancellation.cancel();
        let error = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap()
            .unwrap().err().expect("cancelled broker open must fail");
        assert_eq!(error.code, ChatModelErrorCode::Cancelled);
    }
}
