use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role};

pub fn request(model: &str, system: &str, text: &str, max_tokens: u32) -> LlmRequest {
    LlmRequest {
        model: model.into(),
        system: system.into(),
        messages: vec![Message::new(Role::User, vec![ContentBlock::Text { text: text.into() }])],
        tools: Vec::new(),
        max_tokens: Some(max_tokens),
        thinking: None,
        reasoning_effort: None,
        retain_provider_round: false,
    }
}

pub async fn collect_events(mut receiver: tokio::sync::mpsc::Receiver<LlmEvent>) -> Vec<LlmEvent> {
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}
