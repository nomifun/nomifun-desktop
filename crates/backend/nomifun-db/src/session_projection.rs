//! Read models derived from the canonical AgentSession event store.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDayBucket {
    pub day: String,
    pub message_count: i64,
}
