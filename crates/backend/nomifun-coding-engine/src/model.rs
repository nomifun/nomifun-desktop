//! Compatibility names for the shared engine-neutral Broker port. The Coding
//! core retains its public API without owning another provider adapter.
pub use nomifun_chat_model_broker::{
    BrokerEngineModelPort as BrokerCodingModelPort, EngineModelPort as CodingModelPort,
    EngineModelStream as CodingModelStream,
};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nomifun_chat_model_broker::{
        ChatBrokerPort, ChatModelError, ChatModelErrorCode, ChatModelRequest, ChatModelStream,
        ChatRetryDirective, recorded_conformance_fixtures,
    };
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    struct CancellableBroker(Arc<Notify>);

    struct LegacyOnlyBroker;

    #[async_trait]
    impl ChatBrokerPort for LegacyOnlyBroker {
        async fn open_chat_stream(
            &self,
            _: ChatModelRequest,
        ) -> Result<ChatModelStream, ChatModelError> {
            panic!("unsupported cancellation must not fall back to legacy opening");
        }
    }

    #[tokio::test]
    async fn adapter_rejects_broker_without_native_cancellation() {
        let adapter = BrokerCodingModelPort::new(Arc::new(LegacyOnlyBroker));
        let request = recorded_conformance_fixtures().remove(0).request;
        let error = adapter
            .open_stream(request, CancellationToken::new())
            .await
            .err()
            .expect("unsupported cancellation must fail closed");
        assert_eq!(error.code, ChatModelErrorCode::AdapterUnavailable);
    }

    #[async_trait]
    impl ChatBrokerPort for CancellableBroker {
        async fn open_chat_stream(
            &self,
            _: ChatModelRequest,
        ) -> Result<ChatModelStream, ChatModelError> {
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
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        cancellation.cancel();
        let error = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .err()
            .expect("cancelled broker open must fail");
        assert_eq!(error.code, ChatModelErrorCode::Cancelled);
    }
}
